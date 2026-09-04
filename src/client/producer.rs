//! # The queue
//!
//! The producer role (STORAGE §8, §15.1), whose only write is the
//! enqueue transaction, and the owner's drain (§15.2), which stages each
//! action through the mutate verb and deletes its row in one transaction.

use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, named_params};

use crate::{
    change::PimdirWriteOp,
    client::{
        PimdirError, PimdirSourceStore,
        blobs::PimdirBlobs,
        busy_or_sql,
        lock::PimdirLock,
        reader::{PimdirItem, item_from_row},
        rows, schema, write,
    },
    codec::{self, PimdirAction},
    collection::PimdirCollectionId,
    coroutine::*,
    hash::{PimdirHashAlgo, PimdirHasher},
    hub::PimdirSourceId,
    load::PimdirLoaded,
    mutate::{PimdirMutate, PimdirMutation},
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement, PimdirSortKey, PimdirStatus,
    },
    sql, summary,
};

/// A pimdir store opened as a producer: a process that originates
/// mutations without owning the store, whose sole write is the enqueue.
///
/// It holds the staging lock shared for its lifetime, so a body it
/// writes before enqueueing is never swept in between (§8).
pub struct PimdirProducer {
    conn: Connection,
    _lock: PimdirLock,
    blobs: PimdirBlobs,
    producer: String,
    account: Option<String>,
}

impl PimdirProducer {
    /// Opens the store rooted at `dir` as producer `producer`, a
    /// diagnostic name recorded on each row. The store must exist at the
    /// current version: a producer never creates one.
    pub fn open(dir: impl AsRef<Path>, producer: impl Into<String>) -> Result<Self, PimdirError> {
        let dir = dir.as_ref();
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(dir.join("pimdir.db"), flags)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 30000;",
        )?;
        schema::check_version(&conn)?;
        let hash = schema::hash_algo(&conn, None)?;

        Ok(Self {
            conn,
            _lock: PimdirLock::stage(dir)?,
            blobs: PimdirBlobs::open(dir, hash),
            producer: producer.into(),
            account: None,
        })
    }

    /// The hash this store names its objects by (§5).
    pub fn hash_algo(&self) -> PimdirHashAlgo {
        self.blobs.hash_algo()
    }

    /// The content hash of a whole body, under this store's algorithm.
    pub fn hash(&self, bytes: &[u8]) -> PimdirHash {
        self.blobs.hash(bytes)
    }

    /// An incremental hasher for a body streamed into the blob store.
    pub fn hasher(&self) -> PimdirHasher {
        self.blobs.hasher()
    }

    /// The blob directory, where a body is written before its enqueue.
    pub fn blobs(&self) -> PimdirBlobs {
        self.blobs.clone()
    }

    /// Binds this producer to an account (§9.2).
    pub fn for_account(mut self, account: impl Into<String>) -> Self {
        self.account = Some(account.into());
        self
    }

    /// Appends one action to a collection's queue (§15.1), returning the
    /// row's id: `ensure_collection`, at most one object upsert for a body
    /// the caller wrote through [`blobs`](Self::blobs) whose size it
    /// passes, and the insert that pins it. SQLite stamps `created_at`.
    pub fn enqueue(
        &mut self,
        collection: &str,
        action: &PimdirAction,
        object_size: Option<u64>,
    ) -> Result<i64, PimdirError> {
        let hash = action.object_hash().cloned();

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        tx.execute(
            sql::ENSURE_COLLECTION,
            named_params! { ":collection": collection, ":account": self.account.as_deref() },
        )?;
        if let (Some(hash), Some(size)) = (&hash, object_size) {
            tx.execute(
                sql::STORE_OBJECT,
                named_params! { ":hash": hash.0, ":size": size as i64 },
            )?;
        }
        tx.execute(
            sql::ENQUEUE_ACTION,
            named_params! {
                ":producer": self.producer,
                ":collection": collection,
                ":action": action.kind(),
                ":payload": codec::action_to_payload(action),
                ":object_hash": hash.as_ref().map(|h| h.0.as_str()),
            },
        )?;
        if let Some(hash) = &hash {
            tx.execute(
                sql::ADJUST_REFCOUNT,
                named_params! { ":delta": 1, ":hash": hash.0 },
            )?;
        }
        let id = tx.last_insert_rowid();
        tx.commit().map_err(busy_or_sql)?;
        Ok(id)
    }

    /// The collection's pending actions in append order, the producer's
    /// read-your-writes overlay (§15.4).
    pub fn pending_actions(
        &self,
        collection: &str,
    ) -> Result<Vec<PimdirPendingAction>, PimdirError> {
        pending_actions(&self.conn, collection)
    }
}

/// One pending queue row, in append order (§15.4).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirPendingAction {
    /// The row's global append id.
    pub id: i64,
    /// The RFC 3339 instant SQLite stamped the row with.
    pub created_at: String,
    /// The enqueuing process, diagnostic only.
    pub producer: String,
    /// The decoded action.
    pub action: PimdirAction,
    /// Apply attempts so far.
    pub attempts: i64,
}

/// One parked queue row: an action the owner judged permanently
/// unappliable, left for operators (§15.2).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirParkedAction {
    /// The row's global append id.
    pub id: i64,
    /// The RFC 3339 instant SQLite stamped the row with.
    pub created_at: String,
    /// The enqueuing process, diagnostic only.
    pub producer: String,
    /// The target collection.
    pub collection: String,
    /// The raw action kind.
    pub action: String,
    /// The raw versioned JSON payload.
    pub payload: String,
    /// Apply attempts before parking.
    pub attempts: i64,
    /// The failure that parked the row.
    pub error: String,
}

/// What a drain pass did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PimdirDrainReport {
    /// Actions applied and deleted from the queue.
    pub applied: usize,
    /// Actions parked with an error.
    pub parked: usize,
    /// Actions this owner could not perform, left pending for one that can.
    pub skipped: usize,
}

/// Reads a collection's pending rows, decoding each payload strictly.
pub(crate) fn pending_actions(
    conn: &Connection,
    collection: &str,
) -> Result<Vec<PimdirPendingAction>, PimdirError> {
    let pending = rows(
        conn,
        sql::LOAD_PENDING_ACTIONS,
        named_params! { ":collection": collection },
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(6)?,
            ))
        },
    )?;

    let mut actions = Vec::new();
    for (id, created_at, producer, kind, payload, attempts) in pending {
        actions.push(PimdirPendingAction {
            id,
            created_at,
            producer,
            action: codec::action_from_payload(&kind, &payload)?,
            attempts,
        });
    }
    Ok(actions)
}

/// One raw pending row, the payload undecoded so a malformed one parks
/// rather than failing the pass.
struct PimdirQueueRow {
    id: i64,
    action: String,
    payload: String,
    object_hash: Option<String>,
}

/// Why a queued action was not staged (§15.2).
enum PimdirRefusal {
    /// It will never stage: the row parks with this reason.
    Park(String),
    /// It cannot stage against this source: the row stays pending, untouched.
    Skip,
}

/// The owner's drain (§15.2).
impl PimdirSourceStore {
    /// Drains a collection's pending actions in append order: each is
    /// applied as the mutation it names and its row deleted in one
    /// transaction. A permanently unappliable action parks; one this
    /// owner cannot perform, or cannot place on its source, is skipped;
    /// a transient failure bumps the attempts and stops the pass.
    pub fn drain_collection(&mut self, collection: &str) -> Result<PimdirDrainReport, PimdirError> {
        let pending: Vec<PimdirQueueRow> = rows(
            &self.store.reader.conn,
            sql::LOAD_PENDING_ACTIONS,
            named_params! { ":collection": collection },
            |r| {
                Ok(PimdirQueueRow {
                    id: r.get(0)?,
                    action: r.get(3)?,
                    payload: r.get(4)?,
                    object_hash: r.get(5)?,
                })
            },
        )?;

        let mut report = PimdirDrainReport::default();
        for row in pending {
            let action = match codec::action_from_payload(&row.action, &row.payload) {
                Ok(action) => action,
                Err(err) => {
                    self.fail_action(row.id, Some(&err.to_string()))?;
                    report.parked += 1;
                    continue;
                }
            };
            if matches!(action, PimdirAction::Unknown { .. }) {
                report.skipped += 1;
                continue;
            }
            match self.apply_queued(collection, &row, &action) {
                Ok(None) => report.applied += 1,
                Ok(Some(PimdirRefusal::Skip)) => report.skipped += 1,
                Ok(Some(PimdirRefusal::Park(reason))) => {
                    self.fail_action(row.id, Some(&reason))?;
                    report.parked += 1;
                }
                Err(err) => {
                    self.store
                        .conn
                        .execute(sql::BUMP_ATTEMPTS, named_params! { ":id": row.id })?;
                    return Err(err);
                }
            }
        }
        Ok(report)
    }

    /// Applies one action and deletes its row in one transaction, the
    /// claim first (§15.2); `None` when applied, the refusal otherwise.
    fn apply_queued(
        &mut self,
        collection: &str,
        row: &PimdirQueueRow,
        action: &PimdirAction,
    ) -> Result<Option<PimdirRefusal>, PimdirError> {
        let blobs = self.store.reader.blobs();
        let tx = self
            .store
            .reader
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;

        let claimed = tx
            .prepare(sql::CLAIM_ACTION)?
            .query_row(named_params! { ":id": row.id }, |r| r.get::<_, i64>(0))
            .optional()?;
        if claimed.is_none() {
            return Ok(None);
        }

        let ops = match stage_action(&tx, &blobs, &self.source, collection, row.id, action)? {
            Ok(ops) => ops,
            Err(refusal) => return Ok(Some(refusal)),
        };
        write::apply(
            &tx,
            &blobs,
            &self.source,
            self.store.account.as_deref(),
            ops,
        )?;
        if let Some(hash) = &row.object_hash {
            tx.execute(
                sql::ADJUST_REFCOUNT,
                named_params! { ":delta": -1, ":hash": hash },
            )?;
        }
        tx.commit().map_err(busy_or_sql)?;
        Ok(None)
    }
}

/// Stages the writes one queued action folds into the store (§15.3),
/// inside the drain transaction.
///
/// An `add` stages the `Created` placement the engine's `Add` stages,
/// its summary derived from the body the producer wrote. Every other
/// kind resolves its `seq` to this source's placement and runs the
/// mutate verb, so the staging semantics stay the engine's.
fn stage_action(
    tx: &Connection,
    blobs: &PimdirBlobs,
    source: &PimdirSourceId,
    collection: &str,
    row_id: i64,
    action: &PimdirAction,
) -> Result<Result<Vec<PimdirWriteOp>, PimdirRefusal>, PimdirError> {
    let collection_id = PimdirCollectionId(collection.to_string());
    let kind = write::kind_of(tx, collection)?;

    if let PimdirAction::Add {
        link_id,
        flags,
        object,
        handle,
    } = action
    {
        let derivation = match object {
            Some(hash) => blobs
                .get(hash)?
                .and_then(|body| summary::derive(&kind, &body)),
            None => None,
        };
        let link = link_id
            .clone()
            .or_else(|| derivation.as_ref().map(|d| d.link_id.clone()));
        let Some(link) = link else {
            return Ok(Err(PimdirRefusal::Park(
                "add carries no link id and none derives from its body".to_string(),
            )));
        };
        let live = tx
            .query_row(
                sql::LIVE_ITEM_FOR_LINK,
                named_params! { ":collection": collection, ":link_id": link.0 },
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        if live.is_some() {
            return Ok(Err(PimdirRefusal::Park(format!(
                "link id already present: {}",
                link.0
            ))));
        }
        let (summary, sort_key) = derivation
            .map(|d| (d.summary, d.sort_key))
            .unwrap_or_default();
        let create = PimdirPlacement {
            collection: collection_id,
            handle: handle
                .clone()
                .unwrap_or_else(|| PimdirHandle(format!("queue-{row_id}"))),
            link_id: Some(link),
            object: object.clone(),
            level: match object {
                Some(_) => PimdirLevel::Full,
                None => PimdirLevel::Probed,
            },
            summary,
            sort_key,
            flags: flags.clone(),
            status: PimdirStatus::Created,
            conflict_revision: None,
            conflict_object: None,
            base: None,
            origin: None,
        };
        return Ok(Ok(vec![PimdirWriteOp::UpsertPlacement(create)]));
    }

    let (seq, removes) = match action {
        PimdirAction::SetFlags { seq, .. }
        | PimdirAction::Move { seq, .. }
        | PimdirAction::Copy { seq, .. }
        | PimdirAction::Update { seq, .. } => (*seq, false),
        PimdirAction::Remove { seq } => (*seq, true),
        PimdirAction::Add { .. } => unreachable!("add staged above"),
        PimdirAction::Unknown { .. } => unreachable!("unknown kinds are skipped, never staged"),
    };
    let item: Option<PimdirItem> = tx
        .query_row(
            sql::GET_ITEM,
            named_params! { ":collection": collection, ":seq": seq },
            item_from_row,
        )
        .optional()?;
    let Some(item) = item else {
        return if removes {
            Ok(Ok(Vec::new()))
        } else {
            Ok(Err(PimdirRefusal::Park(format!("unknown seq: {seq}"))))
        };
    };

    let handle = tx
        .query_row(
            sql::HANDLE_FOR_LINK,
            named_params! {
                ":collection": collection,
                ":link_id": item.link_id.0,
                ":source": source.0,
            },
            |r| r.get::<_, String>(0),
        )
        .optional()?;
    let handle = match handle {
        Some(handle) => PimdirHandle(handle),
        None if removes => return Ok(Ok(Vec::new())),
        None => return Ok(Err(PimdirRefusal::Skip)),
    };

    let mutation = match action {
        PimdirAction::SetFlags { flags, .. } => PimdirMutation::SetFlags {
            handle,
            flags: flags.clone(),
        },
        PimdirAction::Remove { .. } => PimdirMutation::Remove(handle),
        PimdirAction::Move { to, .. } => PimdirMutation::Move {
            handle,
            target: to.clone(),
            placeholder: PimdirHandle(format!("queue-{row_id}")),
        },
        PimdirAction::Copy { to, .. } => PimdirMutation::Copy {
            handle,
            target: to.clone(),
            placeholder: PimdirHandle(format!("queue-{row_id}")),
        },
        PimdirAction::Update { object, .. } => {
            let derivation = blobs
                .get(object)?
                .and_then(|body| summary::derive(&kind, &body));
            let (summary, sort_key) = match derivation {
                Some(derivation) => (derivation.summary, Some(derivation.sort_key)),
                None => (None, None::<PimdirSortKey>),
            };
            PimdirMutation::Edit {
                handle,
                // NOTE: the body already sits in the blob store, indexed
                // and pinned at enqueue; the op is stripped below.
                object: PimdirObject {
                    hash: object.clone(),
                    size: 0,
                },
                body: Vec::new(),
                summary,
                sort_key,
            }
        }
        PimdirAction::Add { .. } => unreachable!("add staged above"),
        PimdirAction::Unknown { .. } => unreachable!("unknown kinds are skipped, never staged"),
    };

    let mut mutate = PimdirMutate::new(collection_id.clone(), mutation);
    let _ = mutate.resume(None);
    let link: PimdirLinkId = item.link_id.clone();
    let placements = write::read_hub(tx, collection, Some(core::slice::from_ref(&link.0)))?
        .project(&collection_id, source);
    let mut state = mutate.resume(Some(PimdirArg::Load(PimdirLoaded {
        placements,
        checkpoint: None,
    })));

    // NOTE: a copy or a move reads its target for the identity it carries.
    if let PimdirCoroutineState::Yielded(PimdirYield::WantsLoad { collection, scope }) = state {
        let links = match &scope {
            crate::load::PimdirLoadScope::Links(links) => {
                links.iter().map(|l| l.0.clone()).collect::<Vec<_>>()
            }
            _ => Vec::new(),
        };
        let placements =
            write::read_hub(tx, &collection.0, Some(&links))?.project(&collection, source);
        state = mutate.resume(Some(PimdirArg::Load(PimdirLoaded {
            placements,
            checkpoint: None,
        })));
    }

    match state {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => {
            let ops = ops
                .into_iter()
                .filter(|op| !matches!(op, PimdirWriteOp::StoreObject { .. }))
                .collect();
            Ok(Ok(ops))
        }
        PimdirCoroutineState::Complete(Err(err)) => Ok(Err(PimdirRefusal::Park(err.to_string()))),
        state => Ok(Err(PimdirRefusal::Park(format!(
            "unexpected mutate state: {state:?}"
        )))),
    }
}
