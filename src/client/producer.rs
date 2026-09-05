//! # The queue
//!
//! The producer role (STORAGE §8, §15.1), whose only write is the
//! enqueue transaction, and the owner's drain (§15.2), which stages each
//! action through the mutate verb and deletes its row in one transaction.

use core::slice;

use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use std::{path::Path, sync::Arc};

use rusqlite::{
    Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior, named_params,
};

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
    codec::{self, PimdirAction, PimdirActionError},
    collection::PimdirCollectionId,
    coroutine::*,
    hash::{PimdirHashAlgo, PimdirHasher},
    hub::PimdirSourceId,
    load::{PimdirLoadScope, PimdirLoaded},
    mutate::{PimdirMutate, PimdirMutation},
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement, PimdirSortKey, PimdirStatus,
    },
    sql, summary,
};

/// A pimdir store opened as a producer (STORAGE §8, §15.1).
///
/// A process that originates mutations without owning the store, whose
/// sole write is the enqueue. It holds the staging lock shared for its
/// lifetime, so a body it writes before enqueueing is never swept in
/// between.
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
    /// current version: a producer never creates one, and a directory
    /// with no database is [`PimdirError::Uncreated`].
    pub fn open(dir: impl AsRef<Path>, producer: impl Into<String>) -> Result<Self, PimdirError> {
        let dir = dir.as_ref();
        if !dir.join("pimdir.db").is_file() {
            return Err(PimdirError::Uncreated);
        }
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
    /// row's id: `ensure_collection`, at most one object upsert for the
    /// body the caller wrote through [`blobs`](Self::blobs) and passes as
    /// `object`, hash and size together, and the insert that pins the hash
    /// the action names. SQLite stamps `created_at`. A body the store
    /// already indexes may be passed again or left out.
    pub fn enqueue(
        &mut self,
        collection: &str,
        action: &PimdirAction,
        object: Option<&PimdirObject>,
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
        if let Some(object) = object {
            tx.execute(
                sql::STORE_OBJECT,
                named_params! { ":hash": object.hash.0, ":size": object.size as i64 },
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

/// One parked queue row (§15.2).
///
/// An action the owner judged permanently unappliable, left for
/// operators.
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

/// Reads a collection's pending rows, decoding each payload strictly: a
/// malformed one is the read's error.
pub(crate) fn pending_actions(
    conn: &Connection,
    collection: &str,
) -> Result<Vec<PimdirPendingAction>, PimdirError> {
    let mut actions = Vec::new();
    for row in pending_rows(conn, collection)? {
        actions.push(row.decode()?);
    }
    Ok(actions)
}

/// Reads a collection's pending rows for an overlay (§15.4): a row whose
/// payload does not decode is left out, the drain being what parks it.
pub(crate) fn overlaid_actions(
    conn: &Connection,
    collection: &str,
) -> Result<Vec<PimdirPendingAction>, PimdirError> {
    Ok(pending_rows(conn, collection)?
        .into_iter()
        .filter_map(|row| row.decode().ok())
        .collect())
}

/// One raw pending row, the payload undecoded so a malformed one parks
/// rather than failing the pass.
struct PimdirQueueRow {
    id: i64,
    created_at: String,
    producer: String,
    action: String,
    payload: String,
    object_hash: Option<String>,
    attempts: i64,
}

impl PimdirQueueRow {
    /// The row with its payload decoded.
    fn decode(&self) -> Result<PimdirPendingAction, PimdirActionError> {
        Ok(PimdirPendingAction {
            id: self.id,
            created_at: self.created_at.clone(),
            producer: self.producer.clone(),
            action: codec::action_from_payload(&self.action, &self.payload)?,
            attempts: self.attempts,
        })
    }
}

/// A collection's pending rows in append order, undecoded.
fn pending_rows(conn: &Connection, collection: &str) -> Result<Vec<PimdirQueueRow>, PimdirError> {
    Ok(rows(
        conn,
        sql::LOAD_PENDING_ACTIONS,
        named_params! { ":collection": collection },
        |r| {
            Ok(PimdirQueueRow {
                id: r.get(0)?,
                created_at: r.get(1)?,
                producer: r.get(2)?,
                action: r.get(3)?,
                payload: r.get(4)?,
                object_hash: r.get(5)?,
                attempts: r.get(6)?,
            })
        },
    )?)
}

/// What a drain did with one row (§15.2).
enum PimdirOutcome {
    /// Applied and its row deleted.
    Applied,
    /// It will never apply: the row parks with this reason.
    Parked(String),
    /// Not this owner's, or not this transaction's: the row is left as found.
    Skipped,
}

/// The owner's drain (§15.2).
impl PimdirSourceStore {
    /// Drains a collection's pending actions in append order: each is
    /// applied as the mutation it names and its row deleted in one
    /// transaction. A failure of the store (a refused rebind, a
    /// constraint, a malformed payload) parks the row; one this owner
    /// cannot perform, cannot place on its source, or lost the claim to,
    /// is skipped; neither stops the rows behind it. A failure of the
    /// environment (the database busy, a body unreadable) bumps the
    /// attempts and stops the pass.
    pub fn drain_collection(&mut self, collection: &str) -> Result<PimdirDrainReport, PimdirError> {
        let pending = pending_rows(&self.store.reader.conn, collection)?;

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
                Ok(PimdirOutcome::Applied) => report.applied += 1,
                Ok(PimdirOutcome::Skipped) => report.skipped += 1,
                Ok(PimdirOutcome::Parked(reason)) => {
                    self.fail_action(row.id, Some(&reason))?;
                    report.parked += 1;
                }
                Err(err) if retryable(&err) => {
                    self.fail_action(row.id, None)?;
                    return Err(err);
                }
                Err(err) => {
                    self.fail_action(row.id, Some(&err.to_string()))?;
                    report.parked += 1;
                }
            }
        }
        Ok(report)
    }

    /// Applies one action and deletes its row in one transaction, the
    /// claim first (§15.2): a claim that deletes nothing is a row another
    /// handle applied or a cancellation removed, and is skipped.
    fn apply_queued(
        &mut self,
        collection: &str,
        row: &PimdirQueueRow,
        action: &PimdirAction,
    ) -> Result<PimdirOutcome, PimdirError> {
        let blobs = self.store.reader.blobs();
        let lock = Arc::clone(&self.store.lock);
        let _writing = lock.writing();
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
            return Ok(PimdirOutcome::Skipped);
        }

        let ops = match stage_action(&tx, &blobs, &self.source, collection, row.id, action)? {
            Ok(ops) => ops,
            Err(outcome) => return Ok(outcome),
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
        Ok(PimdirOutcome::Applied)
    }
}

/// Whether a failure is the environment's rather than the store's (§15.2):
/// the database busy or a body unreadable is retried, everything else
/// parks the row.
fn retryable(err: &PimdirError) -> bool {
    match err {
        PimdirError::Busy | PimdirError::Io(_) => true,
        PimdirError::Sql(rusqlite::Error::SqliteFailure(failure, _)) => matches!(
            failure.code,
            ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
        ),
        _ => false,
    }
}

/// Stages the writes one queued action folds into the store (§15.3),
/// inside the drain transaction.
///
/// An `add` stages the `Created` placement the engine's `Add` stages,
/// its summary derived from the body the producer wrote. Every other
/// kind resolves its `seq` to this source's placement and runs the
/// mutate verb, so the staging semantics stay the engine's. An absent
/// item is a remove's success and parks anything else (§15.3); a live
/// item this source does not bind is another source's, and skips.
fn stage_action(
    tx: &Connection,
    blobs: &PimdirBlobs,
    source: &PimdirSourceId,
    collection: &str,
    row_id: i64,
    action: &PimdirAction,
) -> Result<Result<Vec<PimdirWriteOp>, PimdirOutcome>, PimdirError> {
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
            return Ok(Err(PimdirOutcome::Parked(
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
            return Ok(Err(PimdirOutcome::Parked(format!(
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
            Ok(Err(PimdirOutcome::Parked(format!("unknown seq: {seq}"))))
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
    let Some(handle) = handle.map(PimdirHandle) else {
        return Ok(Err(PimdirOutcome::Skipped));
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
    let placements = write::read_hub(tx, collection, Some(slice::from_ref(&link.0)))?
        .project(&collection_id, source);
    let mut state = mutate.resume(Some(PimdirArg::Load(PimdirLoaded {
        placements,
        checkpoint: None,
    })));

    // NOTE: a copy or a move reads its target for the identity it carries.
    if let PimdirCoroutineState::Yielded(PimdirYield::WantsLoad { collection, scope }) = state {
        let links = match &scope {
            PimdirLoadScope::Links(links) => links.iter().map(|l| l.0.clone()).collect::<Vec<_>>(),
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
        PimdirCoroutineState::Complete(Err(err)) => Ok(Err(PimdirOutcome::Parked(err.to_string()))),
        state => Ok(Err(PimdirOutcome::Parked(format!(
            "unexpected mutate state: {state:?}"
        )))),
    }
}
