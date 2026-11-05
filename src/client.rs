//! # Client
//!
//! The std store: the three roles the format defines (STORAGE §8), one
//! handle each. [`PimdirStore`] owns the store and carries every write
//! that consults no source: retention, the collector, the queue rows a
//! cancellation removes. [`PimdirSourceStore`], which
//! [`for_source`](PimdirStore::for_source) yields, is the store as one
//! source: the load and write seam, the runner of the five verbs
//! against a [`PimdirRemote`], and the queue drain. [`PimdirProducer`]
//! enqueues and nothing else, [`PimdirReader`] reads and takes no lock.
//!
//! [`PimdirRemote`]: crate::remote::PimdirRemote
//! [`PimdirProducer`]: crate::client::producer::PimdirProducer

use alloc::{string::String, vec::Vec};

use std::{
    fmt, fs, io,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use log::{debug, trace, warn};
use rusqlite::{
    Connection, ErrorCode, OpenFlags, OptionalExtension, Params, Row, ToSql, TransactionBehavior,
    config::DbConfig, named_params,
};

use crate::{
    capability::{PimdirCapability, PimdirRefusal},
    client::{
        blobs::{PimdirBlobCheck, PimdirBlobStage, PimdirBlobStray, PimdirBlobs},
        lock::PimdirLock,
        reader::{PimdirReader, reference_from_row},
        write::kind_of,
    },
    codec::{self, PimdirAction, PimdirActionError},
    hash,
    hub::{PimdirHub, PimdirHubConflict, PimdirSourceId},
    object::{PimdirHash, PimdirObject},
    placement::PimdirLinkId,
    reference::{PimdirEndpoint, PimdirReference, PimdirReferenceOrigin, PimdirReferenceRole},
    security, sql,
    summary::{
        self,
        file::{self, PimdirFileSummary},
    },
    sync::SYNC_VERSION,
};

/// The local time collection's id (§14.3): `U+0001` and `time`, under no
/// account, a name no server gives a collection, so every owner of a store
/// finds the same one.
pub const TIME_COLLECTION: &str = "\u{1}time";

pub mod blobs;
mod capability;
pub mod diagnostics;
pub mod modes;
pub mod producer;
pub mod reader;
#[cfg(feature = "search")]
pub mod search;

mod attachments;
mod lock;
mod run;
#[doc(inline)]
pub use run::PimdirRunError;
mod schema;
mod write;

/// A pimdir store held as its owner (STORAGE §8).
///
/// The write surface over the read surface every role shares.
pub struct PimdirStore {
    reader: PimdirReader,
    /// The exclusive owner lock (§8), held for the handle's lifetime and
    /// shared by every handle of this process, carrying the lock its
    /// writers and collector serialise on.
    pub(crate) lock: Arc<PimdirLock>,
    /// The account every collection this handle creates belongs to (§9.2).
    account: Option<String>,
}

impl Deref for PimdirStore {
    type Target = PimdirReader;

    fn deref(&self) -> &Self::Target {
        &self.reader
    }
}

impl DerefMut for PimdirStore {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.reader
    }
}

/// A pimdir store acting as one source: the sync seam (STORAGE §14).
///
/// Every operation means "as this side". Dereferences to the
/// [`PimdirStore`] it was made from.
pub struct PimdirSourceStore {
    store: PimdirStore,
    source: PimdirSourceId,
}

impl Deref for PimdirSourceStore {
    type Target = PimdirStore;

    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

impl DerefMut for PimdirSourceStore {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.store
    }
}

/// What a purge retired: rows, never bytes, which the collector frees.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PimdirPurgeReport {
    /// Retained items deleted.
    pub items: usize,
}

/// What the owner's collection below a date removed (§11.3): rows, never
/// bytes, which the collector frees.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirCollectReport {
    /// The public ids of the items collected.
    pub seqs: Vec<i64>,
}

/// What the owner's release of bodies below a date lowered to `Meta`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirReleaseReport {
    /// The public ids of the items whose body was released.
    pub seqs: Vec<i64>,
}

/// What a collection reclaimed.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirGcReport {
    /// Object rows dropped: bodies nothing references any more.
    pub objects: usize,
    /// Blob files unlinked: those rows' bodies and the orphans a crash left.
    pub blobs: usize,
    /// The bytes those files freed.
    pub bytes: u64,
    /// The entries of objects/ that are no body, left in place (§5): a
    /// link, a writer's temporary, any other name or position.
    pub strays: Vec<PimdirBlobStray>,
}

/// What an erase removed (§11.5).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirEraseReport {
    /// Whether a retained item, or a live one no source binds, held the
    /// public id and was purged.
    pub erased: bool,
    /// What the collector reclaimed after the purge.
    pub collected: PimdirGcReport,
}

/// What emptying quarantine/ removed (§7).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirQuarantineReport {
    /// Files and links unlinked.
    pub files: usize,
    /// The bytes those files held.
    pub bytes: u64,
    /// The entries left in place: anything no file nor link, which no
    /// owner moves there.
    pub left: Vec<PathBuf>,
}

impl PimdirStore {
    /// Opens (creating if absent) the store rooted at `dir` as its owner.
    ///
    /// Takes the store's exclusive advisory lock (§8) and holds it until
    /// the handle drops; a store another process owns is
    /// [`PimdirError::Owned`] at once, never a wait. A fresh database is
    /// created by 0001_init.sql (§6). An existing store is refused before
    /// anything is written when this crate does not open it: not a pimdir
    /// store ([`PimdirError::Foreign`]), a store of another draft or one
    /// lacking `format_version` ([`PimdirError::Version`]), disagreeing
    /// stamps ([`PimdirError::VersionMismatch`]), another algorithm
    /// ([`PimdirError::HashAlgo`]) or a damaged schema
    /// ([`PimdirError::Damaged`]). Nothing is reconciled nor migrated.
    ///
    /// A store is its user's alone (§18): created with its directories
    /// `0700` and its files `0600`, the directory it is created in
    /// restricted so too, and an existing one whose directory or pimdir.db
    /// another user can read or write refused as [`PimdirError::Exposed`].
    ///
    /// A store whose rows stand under an earlier Annex A revision than this
    /// crate's is re-derived before the handle is answered (§6).
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, PimdirError> {
        let dir = dir.as_ref();
        let db = dir.join("pimdir.db");
        if !db.exists() {
            modes::create_dir_all(dir)?;
            modes::restrict(dir)?;
        }
        modes::check(dir)?;
        modes::create_dir_all(&dir.join("objects"))?;

        let lock = PimdirLock::own(dir)?;

        modes::create_database(&db)?;
        let mut conn = Connection::open(&db)?;
        // NOTE: the journal mode is set by the schema runner once it knows
        // the database is a fresh one (§6).
        prepare_connection(&conn)?;
        schema::init(&mut conn)?;

        let mut store = Self {
            reader: PimdirReader::over(conn, dir.to_path_buf())?,
            lock,
            account: None,
        };
        store.rederive()?;
        Ok(store)
    }

    /// Opens the existing store rooted at `dir` as its owner: unlike
    /// [`open`](Self::open), it creates nothing, so a caller that owns the
    /// store for a moment (one local write) never makes a store its sync
    /// engine has not.
    ///
    /// Refused, writing nothing, as [`PimdirError::NotCurrent`] when no
    /// database is under `dir` ([`PimdirNotCurrent::Absent`]) or the one
    /// there holds no schema ([`PimdirNotCurrent::Version`]). Otherwise as
    /// `open`: the same lock, [`PimdirError::Owned`] at once when another
    /// process holds it, the same refusals, [`PimdirError::Exposed`]
    /// included, and the same re-derivation.
    pub fn open_current(dir: impl AsRef<Path>) -> Result<Self, PimdirError> {
        let dir = dir.as_ref();
        let db = dir.join("pimdir.db");
        if !db.is_file() {
            return Err(PimdirError::NotCurrent(PimdirNotCurrent::Absent));
        }

        modes::check(dir)?;
        let lock = PimdirLock::own(dir)?;

        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(&db, flags)?;
        prepare_connection(&conn)?;
        match schema::check(&conn) {
            Err(PimdirError::Uncreated) => {
                return Err(PimdirError::NotCurrent(PimdirNotCurrent::Version {
                    found: 0,
                }));
            }
            checked => checked?,
        }

        let mut store = Self {
            reader: PimdirReader::over(conn, dir.to_path_buf())?,
            lock,
            account: None,
        };
        store.rederive()?;
        Ok(store)
    }

    /// Re-derives the held mail and tasks of a store whose rows stand under
    /// an earlier Annex A revision than [`schema::DERIVATION`] (STORAGE §6):
    /// every message and task holding a body, a keyset page of
    /// `list_held_mail`, then of `list_held_tasks`, at a time, each page
    /// written in its own transaction and the revision recorded with
    /// `set_derivation` in the last one. An open interrupted before then
    /// owes it again, and the next open resumes it from the first page.
    ///
    /// A body that is absent or fails its name derives nothing, the one
    /// failing it moved into quarantine/; its rows stay as written.
    fn rederive(&mut self) -> Result<(), PimdirError> {
        const PAGE: i64 = 256;

        let stored = self.store_meta()?.derivation;
        if stored >= schema::DERIVATION {
            return Ok(());
        }
        debug!("re-derive the store from revision {stored}");

        let lists = [
            (sql::LIST_HELD_MAIL, "message/rfc822"),
            (sql::LIST_HELD_TASKS, "text/calendar"),
        ];
        let mut moved = 0;
        for (at, (statement, kind)) in lists.into_iter().enumerate() {
            let mut after: Option<(String, String)> = None;
            loop {
                let page: Vec<(String, String, String)> = rows(
                    &self.conn,
                    statement,
                    named_params! {
                        ":after_collection": after.as_ref().map(|(collection, _)| collection),
                        ":after_link_id": after.as_ref().map(|(_, link)| link),
                        ":limit": PAGE,
                    },
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(3)?)),
                )?;

                let mut derived = Vec::with_capacity(page.len());
                for (collection, link, hash) in &page {
                    let hash = PimdirHash(hash.clone());
                    match self.blobs().get(&hash)? {
                        Some(body) => derived.extend(
                            summary::derive(kind, &body)
                                .map(|derivation| (collection, link, derivation)),
                        ),
                        None => {
                            self.quarantine(&hash)?;
                        }
                    }
                }

                let tx = self
                    .conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(busy_or_sql)?;
                for (collection, link, derivation) in &derived {
                    let link = PimdirLinkId((*link).clone());
                    let hub =
                        write::read_hub(&tx, collection, Some(core::slice::from_ref(&link.0)))?;
                    if let Some(held) = hub.items.get(&link)
                        && write::rederive_item(&tx, collection, &link, held, derivation)?
                    {
                        moved += 1;
                    }
                }
                let last = page.len() < PAGE as usize;
                if last && at + 1 == lists.len() {
                    tx.execute_cached(
                        sql::SET_DERIVATION,
                        named_params! { ":derivation": schema::DERIVATION },
                    )?;
                }
                tx.commit().map_err(busy_or_sql)?;

                if last {
                    break;
                }
                after = page
                    .last()
                    .map(|(collection, link, _)| (collection.clone(), link.clone()));
            }
        }

        debug!(
            "re-derived {moved} items to revision {}",
            schema::DERIVATION
        );
        Ok(())
    }

    /// Binds this handle to an account, so every collection it creates
    /// is grouped under it (§9.2).
    pub fn for_account(mut self, account: impl Into<String>) -> Self {
        self.account = Some(account.into());
        self
    }

    /// The account this handle writes under, `None` in a single-account store.
    pub fn account(&self) -> Option<&str> {
        self.account.as_deref()
    }

    /// The write side of the blob directory (§5), holding the owner lock
    /// while it lives: what a connector streams a fetched body through
    /// before the byteless `StoreObject` that indexes it (§14).
    pub fn blob_stage(&self) -> PimdirBlobStage {
        PimdirBlobStage::new(self.blobs(), Arc::clone(&self.lock))
    }

    /// Binds this handle to a source, yielding the sync seam.
    pub fn for_source(self, source: impl Into<String>) -> PimdirSourceStore {
        PimdirSourceStore {
            store: self,
            source: PimdirSourceId(source.into()),
        }
    }

    /// A collection's whole hub: every source's items and bindings.
    pub fn load_hub(&self, collection: impl AsRef<str>) -> Result<PimdirHub, PimdirError> {
        write::read_hub(&self.conn, collection.as_ref(), None)
    }

    /// Declares a collection's media type, creating the row if absent
    /// (§14). The lazy creation inside a write never overwrites it.
    pub fn ensure_collection(
        &self,
        collection: impl AsRef<str>,
        kind: &str,
    ) -> Result<(), PimdirError> {
        self.conn
            .execute_cached(
                sql::SET_COLLECTION_KIND,
                named_params! {
                    ":collection": collection.as_ref(),
                    ":account": self.account.as_deref(),
                    ":kind": kind,
                },
            )
            .map_err(busy_or_sql)?;
        Ok(())
    }

    /// Records what a collection is called, creating the row if absent
    /// (§14).
    ///
    /// The name is a label and the id is the address, so an owner
    /// namespacing its ids records the bare name here and a reader
    /// renders it without parsing a prefix it cannot know the shape of.
    /// Nothing keys on it: moving it costs a label and never a re-sync.
    pub fn set_collection_name(
        &self,
        collection: impl AsRef<str>,
        name: &str,
    ) -> Result<(), PimdirError> {
        self.conn
            .execute_cached(
                sql::SET_COLLECTION_NAME,
                named_params! {
                    ":collection": collection.as_ref(),
                    ":account": self.account.as_deref(),
                    ":name": name,
                },
            )
            .map_err(busy_or_sql)?;
        Ok(())
    }

    /// Records what the source states a collection is for, or clears it
    /// with `None` (§14): a mail role (`inbox`, `sent`, `drafts`, `trash`,
    /// `junk`, `archive`, `all`, `flagged`, `important`), or `default` for
    /// a calendar or an address book.
    ///
    /// Only from what a source states, never guessed from a name. The
    /// collection holding the role in the account loses it in the same
    /// statement. The collection must be declared: a role outside its
    /// kind's vocabulary is refused as [`PimdirError::Role`], the column
    /// being an open vocabulary no constraint checks (§13).
    pub fn set_collection_role(
        &self,
        collection: impl AsRef<str>,
        role: Option<&str>,
    ) -> Result<(), PimdirError> {
        let collection = collection.as_ref();
        if let Some(role) = role
            && !crate::collection::role_of_kind(&write::kind_of(&self.conn, collection)?, role)
        {
            return Err(PimdirError::Role {
                collection: String::from(collection),
                role: String::from(role),
            });
        }
        self.conn
            .execute_cached(
                sql::SET_COLLECTION_ROLE,
                named_params! { ":collection": collection, ":role": role },
            )
            .map_err(busy_or_sql)?;
        Ok(())
    }

    /// Records a reference from one item to another (`add_reference`,
    /// §14.2), answering it as it stands, recorded now or before, a
    /// person's taking over a rule's; `None` when it cannot link: an end
    /// the store holds no row of under its kind, or one under a
    /// writer-derived key.
    ///
    /// Nothing requires one. A role outside the vocabulary, or an item
    /// referring to itself, is refused by the schema.
    pub fn add_reference(
        &self,
        from: &PimdirEndpoint,
        to: &PimdirEndpoint,
        role: &PimdirReferenceRole,
        origin: PimdirReferenceOrigin,
    ) -> Result<Option<PimdirReference>, PimdirError> {
        self.conn
            .query_row_cached(
                sql::ADD_REFERENCE,
                named_params! {
                    ":from_kind": from.kind,
                    ":from_link_id": from.link_id.as_str(),
                    ":to_kind": to.kind,
                    ":to_link_id": to.link_id.as_str(),
                    ":role": role.as_str(),
                    ":origin": origin.as_str(),
                },
                reference_from_row,
            )
            .optional()
            .map_err(busy_or_sql)
    }

    /// Removes one reference, whatever its origin (`remove_reference`,
    /// §14.2), answering it, or `None` when none was recorded. A rule
    /// matching again records it anew.
    pub fn remove_reference(
        &self,
        from: &PimdirEndpoint,
        to: &PimdirEndpoint,
        role: &PimdirReferenceRole,
    ) -> Result<Option<PimdirReference>, PimdirError> {
        self.conn
            .query_row_cached(
                sql::REMOVE_REFERENCE,
                named_params! {
                    ":from_kind": from.kind,
                    ":from_link_id": from.link_id.as_str(),
                    ":to_kind": to.kind,
                    ":to_link_id": to.link_id.as_str(),
                    ":role": role.as_str(),
                },
                reference_from_row,
            )
            .optional()
            .map_err(busy_or_sql)
    }

    /// Writes a file holding no body into a collection no source syncs
    /// (§14.3), or restates its summary when the collection holds it,
    /// answering its public id: the stand-in of an attachment, keyed by
    /// [`part_key`](crate::summary::file::part_key), at `Meta`, which no
    /// sync reads as a body to fetch.
    ///
    /// The owner records the stand-in and its `attachment` reference in
    /// one transaction of its own; a stand-in no reference names goes on
    /// the next reference removed. A collection of another kind than
    /// files is refused as [`PimdirError::NotFiles`].
    pub fn put_file(
        &mut self,
        collection: impl AsRef<str>,
        link_id: &PimdirLinkId,
        summary: &PimdirFileSummary,
    ) -> Result<i64, PimdirError> {
        let collection = collection.as_ref();
        let kind = kind_of(&self.conn, collection)?;
        if kind != file::KIND {
            return Err(PimdirError::NotFiles {
                collection: collection.into(),
                kind,
            });
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        let key = named_params! { ":collection": collection, ":link_id": link_id.as_str() };
        let held: Option<i64> = tx
            .query_row_cached(sql::SEQ_BY_LINK, key, |row| row.get(0))
            .optional()?;
        let seq = match held {
            Some(seq) => {
                tx.execute_cached(
                    sql::SET_SORT_KEY,
                    named_params! {
                        ":collection": collection,
                        ":link_id": link_id.as_str(),
                        ":sort_key": summary.sort_key().0,
                    },
                )?;
                attachments::upsert_file_summary(&tx, collection, link_id, summary)?;
                seq
            }
            None => attachments::insert_file(&tx, collection, link_id, summary)?,
        };
        tx.commit().map_err(busy_or_sql)?;
        Ok(seq)
    }

    /// Writes a body into a collection no source syncs (§14.3), answering
    /// its link id and public id: a time entry of the
    /// [local time collection](Self::ensure_time_collection), or a card or
    /// calendar item kept on the device alone.
    ///
    /// The key, the summary and the sort key are derived from the body as
    /// a synced write derives them (Annex A), and the item, its summary,
    /// its automatic references (§14.2) and its pins are written in one
    /// transaction. A body under a key the collection holds replaces it,
    /// the flags kept. [`delete_unbound`](Self::delete_unbound) removes
    /// it. A collection a source syncs is refused as
    /// [`PimdirError::Synced`], its items being the source's to write
    /// (through the queue, §15), and one of another kind than contacts or
    /// calendars as [`PimdirError::NotUnbound`]: mail is a source's, and
    /// a file's summary is the caller's ([`put_file`](Self::put_file)).
    pub fn put_item(
        &mut self,
        collection: impl AsRef<str>,
        body: &[u8],
    ) -> Result<(PimdirLinkId, i64), PimdirError> {
        let collection = collection.as_ref();
        let kind = kind_of(&self.conn, collection)?;
        let derivation = match kind.split(';').next().unwrap_or_default().trim() {
            "text/vcard" | "text/calendar" => summary::derive(&kind, body),
            _ => None,
        };
        let Some(derivation) = derivation else {
            return Err(PimdirError::NotUnbound {
                collection: collection.into(),
                kind,
            });
        };
        let object = PimdirObject {
            hash: hash::hash(body),
            size: body.len(),
        };
        let blobs = PimdirBlobs::open(&self.dir);
        let dir = self.dir.clone();
        let lock = Arc::clone(&self.lock);
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        let synced = tx
            .prepare_cached(sql::COLLECTION_SOURCES)?
            .exists(named_params! { ":collection": collection })?;
        if synced {
            return Err(PimdirError::Synced {
                collection: collection.into(),
            });
        }
        // NOTE: inside the transaction, so no collector runs between the
        // blob landing and the row pinning it (§5); a name held by a file
        // failing it is set aside and the body written again.
        match blobs.write(&object.hash, body) {
            Err(err) if blobs::held_name(&err).is_some() => {
                let Ok(staging) = PimdirLock::collect(&dir) else {
                    return Err(err.into());
                };
                let collecting = lock.collecting();
                if let Some(target) = blobs.quarantine(&object.hash)? {
                    warn!(
                        "body {} fails its name, moved to {}",
                        object.hash.0,
                        target.display()
                    );
                }
                drop((collecting, staging));
                blobs.write(&object.hash, body)?;
            }
            written => written?,
        }
        let link = derivation.link_id.clone();
        let seq = write::put_unbound(&tx, collection, &object, derivation)?;
        tx.commit().map_err(busy_or_sql)?;
        Ok((link, seq))
    }

    /// Declares the local time collection (§14.3) once and answers its id,
    /// [`TIME_COLLECTION`]: a calendar collection under no account that no
    /// source syncs, where time entries kept on the device go through
    /// [`put_item`](Self::put_item). A reader finds it by the id alone,
    /// declaring nothing.
    pub fn ensure_time_collection(&mut self) -> Result<&'static str, PimdirError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        tx.execute_cached(
            sql::SET_COLLECTION_KIND,
            named_params! {
                ":collection": TIME_COLLECTION,
                ":account": None::<&str>,
                ":kind": "text/calendar",
            },
        )?;
        tx.execute_cached(
            sql::SET_COLLECTION_NAME,
            named_params! {
                ":collection": TIME_COLLECTION,
                ":account": None::<&str>,
                ":name": "time",
            },
        )?;
        tx.commit().map_err(busy_or_sql)?;
        Ok(TIME_COLLECTION)
    }

    /// Regroups a collection under `account`, or out of one with `None` (§9.2).
    pub fn set_collection_account(
        &self,
        collection: impl AsRef<str>,
        account: Option<&str>,
    ) -> Result<(), PimdirError> {
        self.conn
            .execute_cached(
                sql::SET_COLLECTION_ACCOUNT,
                named_params! { ":collection": collection.as_ref(), ":account": account },
            )
            .map_err(busy_or_sql)?;
        Ok(())
    }

    /// Sets a collection's cross-source conflict policy (SYNC §9).
    pub fn set_collection_conflict(
        &self,
        collection: impl AsRef<str>,
        policy: PimdirHubConflict,
    ) -> Result<(), PimdirError> {
        self.conn
            .execute_cached(
                sql::SET_CONFLICT,
                named_params! {
                    ":collection": collection.as_ref(),
                    ":conflict": codec::conflict_to_str(policy),
                },
            )
            .map_err(busy_or_sql)?;
        Ok(())
    }

    /// Restates one item's ordering key (§9.3), outside the write path.
    pub fn set_sort_key(
        &self,
        collection: impl AsRef<str>,
        link_id: &str,
        sort_key: &str,
    ) -> Result<(), PimdirError> {
        self.conn
            .execute_cached(
                sql::SET_SORT_KEY,
                named_params! {
                    ":collection": collection.as_ref(),
                    ":link_id": link_id,
                    ":sort_key": sort_key,
                },
            )
            .map_err(busy_or_sql)?;
        Ok(())
    }

    /// Gives a collection a new id, its contents following through the
    /// cascades and its items restamped (§14): the only safe way to change
    /// one. A pending move, copy or superseding add naming it as a target
    /// follows too, its `target_collection` being what no cascade reaches.
    pub fn rename_collection(
        &mut self,
        collection: impl AsRef<str>,
        new_id: &str,
    ) -> Result<(), PimdirError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        tx.execute_cached(
            sql::RENAME_QUEUE_TARGETS,
            named_params! { ":collection": collection.as_ref(), ":new_id": new_id },
        )?;
        tx.execute_cached(
            sql::RENAME_COLLECTION,
            named_params! { ":collection": collection.as_ref(), ":new_id": new_id },
        )?;
        tx.commit().map_err(busy_or_sql)?;
        Ok(())
    }

    /// Removes a collection with everything under it (§14), reporting
    /// whether there was one. Retention does not apply: the operator
    /// removed the collection itself. The pins the cascade drops are
    /// settled by a recompute in the same transaction, skipped when the
    /// collection held none (`collection_holds_objects`); the bodies fall
    /// to the collector. A collection holding a pending or parked queue
    /// row is refused as [`PimdirError::Queued`], the cascade dropping the
    /// action with no receipt (`collections_keep_queued`).
    pub fn delete_collection(&mut self, collection: impl AsRef<str>) -> Result<bool, PimdirError> {
        let collection = collection.as_ref();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        let key = named_params! { ":collection": collection };
        let pins: bool =
            tx.query_row_cached(sql::COLLECTION_HOLDS_OBJECTS, key, |row| row.get(0))?;
        let deleted = match tx.execute_cached(sql::DELETE_COLLECTION, key) {
            Err(rusqlite::Error::SqliteFailure(err, Some(message)))
                if err.code == ErrorCode::ConstraintViolation
                    && message == "collection holds queued actions" =>
            {
                return Err(PimdirError::Queued {
                    collection: String::from(collection),
                });
            }
            deleted => deleted?,
        };
        if pins {
            tx.execute_cached(sql::RECOMPUTE_REFCOUNTS, [])?;
        }
        tx.commit().map_err(busy_or_sql)?;
        Ok(deleted > 0)
    }
}

/// Retention (§11): the trash a store keeps, and the only deletes.
impl PimdirStore {
    /// Deletes an item no source binds (`delete_unbound_item`, §14.3), a
    /// file of a device folder, a stand-in or an item
    /// [put](Self::put_item) there, a time entry say, releasing its pins,
    /// and reports whether there was one; a bound item is its source's to
    /// remove and is left alone.
    pub fn delete_unbound(
        &mut self,
        collection: impl AsRef<str>,
        link_id: &PimdirLinkId,
    ) -> Result<bool, PimdirError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        let pinned: Option<(Option<String>, Option<String>)> = tx
            .prepare_cached(sql::DELETE_UNBOUND_ITEM)?
            .query_row(
                named_params! { ":collection": collection.as_ref(), ":link_id": link_id.as_str() },
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((object, conflict_object)) = pinned else {
            return Ok(false);
        };

        release_pins(&tx, [object, conflict_object].into_iter().flatten())?;
        tx.commit().map_err(busy_or_sql)?;
        Ok(true)
    }

    /// Purges one retained item by its public id, reporting whether there
    /// was one; a live item is never reached.
    pub fn purge(&mut self, collection: impl AsRef<str>, seq: i64) -> Result<bool, PimdirError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;

        let pinned: Option<(Option<String>, Option<String>)> = tx
            .prepare_cached(sql::PURGE_ITEM)?
            .query_row(
                named_params! { ":collection": collection.as_ref(), ":seq": seq },
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((object, conflict_object)) = pinned else {
            return Ok(false);
        };

        release_pins(&tx, [object, conflict_object].into_iter().flatten())?;
        tx.commit().map_err(busy_or_sql)?;
        Ok(true)
    }

    /// Purges every item retired strictly before `cutoff` (RFC 3339),
    /// store-wide: the cutoff is the caller's policy, never the store's clock.
    ///
    /// The comparison is textual (§11.2), so the cutoff is written as the
    /// stamps are, UTC with milliseconds (`2026-06-01T00:00:00.000Z`):
    /// anything else is [`PimdirError::Cutoff`] and nothing is purged.
    pub fn purge_retained_before(
        &mut self,
        cutoff: &str,
    ) -> Result<PimdirPurgeReport, PimdirError> {
        if !is_stamp(cutoff) {
            return Err(PimdirError::Cutoff(cutoff.into()));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;

        let pinned: Vec<(Option<String>, Option<String>)> = rows(
            &tx,
            sql::PURGE_RETAINED_BEFORE,
            named_params! { ":cutoff": cutoff },
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let items = pinned.len();
        release_pins(
            &tx,
            pinned
                .into_iter()
                .flat_map(|(object, conflict)| [object, conflict])
                .flatten(),
        )?;
        tx.commit().map_err(busy_or_sql)?;

        Ok(PimdirPurgeReport { items })
    }

    /// Erases the item `seq` of `collection` from the store's files, at the
    /// user's request (§11.5): its purge when it is retained, or
    /// `delete_unbound_item` when it is live and no source binds it, its
    /// pins released; then the collector, which unlinks every body no row
    /// names any more; then a `TRUNCATE` checkpoint of pimdir.db, again
    /// until every frame is checkpointed. With `secure_delete` on every
    /// connection, the item's rows then stand in neither the file nor its
    /// WAL. A live item a source binds is that source's to remove first:
    /// nothing is purged, [`erased`](PimdirEraseReport::erased) false.
    ///
    /// The other steps run whether or not an item was purged, so an erase
    /// a busy reader stopped ([`PimdirError::Busy`] past three waits on
    /// the WAL) finishes when it is run again. Where an index is kept,
    /// `PimdirIndex::erase` (feature `search`) runs the rest of §11.5 on
    /// index.db after it: a body another item names, a backup, quarantine/
    /// and the filesystem's own copies stay.
    pub fn erase(
        &mut self,
        collection: impl AsRef<str>,
        seq: i64,
    ) -> Result<PimdirEraseReport, PimdirError> {
        let collection = collection.as_ref();
        let live = self.get_item(collection, seq)?;

        let erased = match self.purge(collection, seq)? {
            true => true,
            false => match &live {
                Some(item) => self.delete_unbound(collection, &item.link_id)?,
                None => false,
            },
        };
        debug!("erase {collection}/{seq}: purged {erased}");

        let collected = self.collect_garbage()?;
        checkpoint(&self.conn)?;

        Ok(PimdirEraseReport { erased, collected })
    }

    /// The owner's manual collection of one mail collection below a date
    /// (§11.3): every live item whose `date` is older than `before` (RFC
    /// 3339, `Z`) and that owes nothing, removed with its bindings,
    /// summary and addresses, the public ids collected answered.
    ///
    /// An item owes something while it is conflicted, a binding of it is
    /// conflicted or has no base, or its flags differ from a binding's
    /// base; it stays, as does an item with no `date`. Not a delete: no
    /// tombstone, no push, and the remote keeps every member, which a
    /// later widening lists and names again. The refcounts are recomputed
    /// in the same transaction and the bodies fall to the collector. A
    /// verb of this process between two of its chunks refuses it as
    /// [`PimdirError::InFlight`] (§5). An owner narrowing a scope records
    /// the narrower coverage with its next sync, not here.
    pub fn collect_before(
        &mut self,
        collection: impl AsRef<str>,
        before: &str,
    ) -> Result<PimdirCollectReport, PimdirError> {
        if self.lock.in_flight() {
            return Err(PimdirError::InFlight(self.dir.clone()));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        let seqs: Vec<i64> = rows(
            &tx,
            sql::COLLECT_BEFORE,
            named_params! { ":collection": collection.as_ref(), ":before": before },
            |row| row.get(0),
        )?;
        tx.execute_cached(sql::RECOMPUTE_REFCOUNTS, [])?;
        tx.commit().map_err(busy_or_sql)?;
        Ok(PimdirCollectReport { seqs })
    }

    /// The owner's release of the bodies of a set of mail collections
    /// below a sort key (§11.4): every live item whose `sort_key` is
    /// below `until` (RFC 3339, `Z`; `None` for no ceiling), the undated
    /// below any, goes back to `Meta`, keeping its summary, addresses,
    /// flags, bindings and public id, the public ids released answered.
    ///
    /// An item needing its body keeps it: one conflicted, holding a
    /// conflicted binding, a pending create or a local edit, bound by no
    /// source, or not yet bound by a source of its collection. A binding
    /// whose base is the body lets go of it and stays based. Not a
    /// delete: no tombstone, no push; the next `Full` upgrade fetches or
    /// links the body again. The refcounts are recomputed in the same
    /// transaction and the bodies fall to the collector. A verb of this
    /// process between two of its chunks refuses it as
    /// [`PimdirError::InFlight`] (§5).
    pub fn release_before(
        &mut self,
        collections: &[impl AsRef<str>],
        until: Option<&str>,
    ) -> Result<PimdirReleaseReport, PimdirError> {
        if self.lock.in_flight() {
            return Err(PimdirError::InFlight(self.dir.clone()));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        bind_list(&tx, "collections", collections.iter().map(AsRef::as_ref))?;
        tx.execute_cached(sql::RELEASE_BASES_BEFORE, named_params! { ":until": until })?;
        let seqs: Vec<i64> = rows(
            &tx,
            sql::RELEASE_BEFORE,
            named_params! { ":until": until },
            |row| row.get(0),
        )?;
        tx.execute_cached(sql::RECOMPUTE_REFCOUNTS, [])?;
        tx.commit().map_err(busy_or_sql)?;
        Ok(PimdirReleaseReport { seqs })
    }
}

/// Reclamation and repair (§5, §7): what no write does to the store.
impl PimdirStore {
    /// Reclaims what nothing references: the object rows at refcount
    /// zero, their bodies, and the orphan blobs a crash left.
    ///
    /// Takes the staging lock exclusively on an owning handle, and the
    /// process's own writer lock exclusively across the rows and the
    /// walk, so no writer is between a body and the row that pins it
    /// (§8). A verb of this process between two of its chunks refuses it
    /// as [`PimdirError::InFlight`], a body it streamed having no pointer
    /// yet (§5). The counts are recomputed first, so a count a writer left
    /// behind never decides a delete; the rows go inside a transaction
    /// and the files after it.
    pub fn collect_garbage(&mut self) -> Result<PimdirGcReport, PimdirError> {
        if self.lock.in_flight() {
            return Err(PimdirError::InFlight(self.dir.clone()));
        }
        let _staging = PimdirLock::collect(&self.dir)?;
        let lock = Arc::clone(&self.lock);
        let _collecting = lock.collecting();

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        tx.execute_cached(sql::RECOMPUTE_REFCOUNTS, [])?;
        let objects = tx.execute_cached(sql::DELETE_GARBAGE_OBJECTS, [])?;
        tx.commit().map_err(busy_or_sql)?;

        let mut report = PimdirGcReport {
            objects,
            ..Default::default()
        };
        // NOTE: the scan reads entries by lstat, so a body is a regular
        // file named well in its own shard, and anything else is a stray
        // left in place (§5).
        let scan = self.blobs().scan()?;
        let mut exists = self.conn.prepare_cached(sql::OBJECT_EXISTS)?;
        for blob in scan.files {
            if exists.exists(named_params! { ":hash": blob.hash })? {
                continue;
            }
            fs::remove_file(&blob.path)?;
            report.blobs += 1;
            report.bytes += blob.size;
        }
        drop(exists);
        for stray in &scan.strays {
            debug!("leave {:?} {}", stray.kind, stray.path.display());
        }
        report.strays = scan.strays;

        Ok(report)
    }

    /// Moves the body under `hash` into quarantine/ when it fails its name
    /// (§7): bytes hashing to another name, or a link where its shard or
    /// itself belongs. Checked again under the staging lock taken
    /// exclusively and the process's own writer lock, as the collector
    /// holds them, and never unlinked; the row stays, so the next write of
    /// that body lands. Answers where it went, `None` when the body is
    /// sound or absent, or when a producer holds the staging lock: the
    /// move is then made by the next read finding the mismatch.
    pub fn quarantine(&self, hash: &PimdirHash) -> Result<Option<PathBuf>, PimdirError> {
        let Ok(_staging) = PimdirLock::collect(&self.dir) else {
            return Ok(None);
        };
        let lock = Arc::clone(&self.lock);
        let _collecting = lock.collecting();

        let moved = self.blobs().quarantine(hash)?;
        if let Some(target) = &moved {
            warn!(
                "body {} fails its name, moved to {}",
                hash.0,
                target.display()
            );
        }

        Ok(moved)
    }

    /// Empties quarantine/ (§7), at the user's request: every file and
    /// link in it unlinked, under the staging lock taken exclusively and
    /// the process's own writer lock, as a move into it holds them. What
    /// it held may have been the only copy of a body.
    ///
    /// A producer holding the staging lock is [`PimdirError::Staging`],
    /// reported at once.
    pub fn empty_quarantine(&self) -> Result<PimdirQuarantineReport, PimdirError> {
        let _staging = PimdirLock::collect(&self.dir)?;
        let lock = Arc::clone(&self.lock);
        let _collecting = lock.collecting();

        let mut report = PimdirQuarantineReport::default();
        for path in self.blobs().quarantined()? {
            let meta = fs::symlink_metadata(&path)?;
            if !meta.is_file() && !meta.file_type().is_symlink() {
                report.left.push(path);
                continue;
            }

            fs::remove_file(&path)?;
            report.files += 1;
            if meta.is_file() {
                report.bytes += meta.len();
            }
        }
        debug!("emptied quarantine: {} files", report.files);

        Ok(report)
    }

    /// Checks the body under `hash` against its name, streamed (§7), and
    /// moves one failing it into quarantine/: what the owner's reads of a
    /// body it is about to use go through.
    pub(crate) fn check_body(&self, hash: &PimdirHash) -> Result<PimdirBlobCheck, PimdirError> {
        let check = self.blobs().check(hash)?;
        if check == PimdirBlobCheck::Failing {
            self.quarantine(hash)?;
        }

        Ok(check)
    }

    /// Recomputes every refcount from the five pointer columns (§7),
    /// returning how many rows disagreed.
    pub fn recompute_refcounts(&self) -> Result<usize, PimdirError> {
        self.conn
            .execute_cached(sql::RECOMPUTE_REFCOUNTS, [])
            .map_err(busy_or_sql)
    }

    /// Deletes the bindings whose item is gone, returning how many: the
    /// one dangling row a repair clears without guessing.
    pub fn clear_dangling_bindings(&self) -> Result<usize, PimdirError> {
        self.conn
            .execute_cached(sql::DELETE_DANGLING_BINDINGS, [])
            .map_err(busy_or_sql)
    }

    /// Raises the store's incarnation (`raise_incarnation`, STORAGE §4.5,
    /// §8), answering the new one: what a restore runs on the copy before
    /// anything else opens it, so no consumer takes the copy for the store
    /// it followed. `replaced` is the incarnation of the store the copy
    /// replaces when that one could still be read, the new one rising
    /// above both.
    pub fn raise_incarnation(&self, replaced: Option<i64>) -> Result<i64, PimdirError> {
        self.conn
            .execute_cached(
                sql::RAISE_INCARNATION,
                named_params! { ":replaced": replaced },
            )
            .map_err(busy_or_sql)?;
        Ok(self.change_cursor()?.incarnation)
    }
}

/// The queue's owner side (§15): declaring, cancelling and recording failures.
impl PimdirStore {
    /// Declares what `source` can do (§15.6), under the account this handle
    /// is bound to, replacing its previous declaration in one transaction. A declaration writes every
    /// capability of the kinds the source syncs, `none` included, so a
    /// source with no row stays the undeclared one of an older owner.
    pub fn declare(
        &mut self,
        source: &str,
        capabilities: &[PimdirCapability],
    ) -> Result<(), PimdirError> {
        let account = self.account.clone();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        capability::declare(&tx, account.as_deref(), source, capabilities)?;
        tx.commit().map_err(busy_or_sql)
    }

    /// Cancels one queue row as the store's owner, holding the role for
    /// the length of the call (§15.5); the store must exist already.
    ///
    /// Opened through [`open_current`](Self::open_current): a consumer
    /// owning the store for one write never creates it, and a store this
    /// crate does not open is refused and left as it was.
    pub fn cancel_action(dir: impl AsRef<Path>, id: i64) -> Result<bool, PimdirError> {
        let dir = dir.as_ref();
        if !dir.join("pimdir.db").is_file() {
            return Err(PimdirError::Uncreated);
        }

        Self::open_current(dir)?.drop_action(id)
    }

    /// Removes one queue row by request, pending or parked, releasing its
    /// body pin in the same transaction; reports whether it existed. It
    /// records no receipt, so a producer reads the row as withdrawn
    /// (§15.4): an intent its owner performed is
    /// [`acknowledge_action`](Self::acknowledge_action)'s.
    pub fn drop_action(&mut self, id: i64) -> Result<bool, PimdirError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;

        let Some(hash) = cancel_queued(&tx, id)? else {
            return Ok(false);
        };

        release_pins(&tx, hash.into_iter())?;
        tx.commit().map_err(busy_or_sql)?;
        Ok(true)
    }

    /// Acknowledges the intent `id` its owner performed out of band
    /// (§15.5): deletes the row, pending or parked, releases its body pin
    /// and records its receipt in one transaction, so a producer reads it
    /// `Applied` rather than gone (§15.4). `seq` is the item the intent's
    /// effect left in the store when the performer knows it, `None`
    /// otherwise (a collection created, a message sent whose copy arrives
    /// with a later sync). Reports whether the row existed; when it did
    /// not, nothing is recorded.
    pub fn acknowledge_action(&mut self, id: i64, seq: Option<i64>) -> Result<bool, PimdirError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;

        let Some(collection) = queued_collection(&tx, id)? else {
            return Ok(false);
        };
        let Some(hash) = cancel_queued(&tx, id)? else {
            return Ok(false);
        };
        record_receipt(&tx, id, &collection, seq)?;

        release_pins(&tx, hash.into_iter())?;
        tx.commit().map_err(busy_or_sql)?;
        Ok(true)
    }

    /// Acknowledges the performed intent `id` by replacing it with the
    /// store change it leaves behind, `action` on `collection` enqueued
    /// under `producer` (§15.5): the change's body is pinned before the
    /// intent's pin is released and the intent's receipt recorded with no
    /// `seq`, in one transaction, so the change exists only once the
    /// intent is performed and a producer reads the intent `Applied`
    /// (§15.4). Reports whether the intent was still queued; when it was
    /// not, nothing is enqueued. A change whose payload its schema refuses
    /// is [`PimdirError::Action`], the intent left queued (§15.3).
    pub fn replace_action(
        &mut self,
        id: i64,
        producer: &str,
        collection: &str,
        action: &PimdirAction,
    ) -> Result<bool, PimdirError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;

        let Some(queued) = queued_collection(&tx, id)? else {
            return Ok(false);
        };
        let payload = codec::action_to_payload(action);
        match codec::action_from_payload(action.kind(), &payload) {
            Ok(_) | Err(PimdirActionError::UnknownVersion(Some(_))) => {}
            Err(err) => return Err(err.into()),
        }
        let hash = action.object_hash().cloned();
        if let Some(hash) = &hash {
            tx.execute_cached(sql::PIN_OBJECT, named_params! { ":hash": hash.0 })?;
        }
        tx.execute_cached(
            sql::ENQUEUE_ACTION,
            named_params! {
                ":producer": producer,
                ":collection": collection,
                ":action": action.kind(),
                ":payload": payload,
                ":target_collection": action.target_collection().map(|to| to.0.as_str()),
                ":object_hash": hash.as_ref().map(|h| h.0.as_str()),
            },
        )?;

        let Some(pin) = cancel_queued(&tx, id)? else {
            // NOTE: dropping the transaction rolls the enqueue back.
            return Ok(false);
        };
        record_receipt(&tx, id, &queued, None)?;

        release_pins(&tx, pin.into_iter())?;
        tx.commit().map_err(busy_or_sql)?;
        Ok(true)
    }

    /// Records a failed apply (§15.2): `None` bumps the attempts and
    /// leaves the row pending, `Some(error)` parks it, the attempt counted,
    /// its text recorded once [scrubbed](security::scrub) of the
    /// credentials a connector's error may carry (§18).
    pub fn fail_action(&self, id: i64, error: Option<&str>) -> Result<(), PimdirError> {
        let Some(error) = error else {
            self.conn
                .execute_cached(sql::BUMP_ATTEMPTS, named_params! { ":id": id })
                .map_err(busy_or_sql)?;
            return Ok(());
        };

        let error = security::scrub(error);
        self.conn
            .execute_cached(
                sql::PARK_ACTION,
                named_params! { ":id": id, ":error": error },
            )
            .map_err(busy_or_sql)?;
        Ok(())
    }
}

impl PimdirSourceStore {
    /// The source this handle acts as.
    pub fn source(&self) -> &str {
        &self.source.0
    }

    /// Binds this handle to an account (§9.2), in either order with the source.
    pub fn for_account(mut self, account: impl Into<String>) -> Self {
        self.store = self.store.for_account(account);
        self
    }
}

/// Runs one statement and collects every row through `map`, the statement
/// kept in the connection's cache.
pub(crate) fn rows<T>(
    conn: &Connection,
    sql: &str,
    params: impl Params,
    map: impl FnMut(&Row) -> rusqlite::Result<T>,
) -> rusqlite::Result<Vec<T>> {
    conn.prepare_cached(sql)?.query_map(params, map)?.collect()
}

/// `execute` and `query_row` through the connection's statement cache.
///
/// Preparing a statement compiles the programs of the triggers it fires,
/// which for the store's writes costs about as much again as running them,
/// so a statement the crate runs more than once is prepared once per
/// connection. A query compiled per search, whose text is new each time,
/// keeps the uncached methods.
pub(crate) trait PreparedCached {
    /// [`Connection::execute`], prepared once.
    fn execute_cached<P: Params>(&self, sql: &str, params: P) -> rusqlite::Result<usize>;

    /// [`Connection::query_row`], prepared once.
    fn query_row_cached<T, P, F>(&self, sql: &str, params: P, f: F) -> rusqlite::Result<T>
    where
        P: Params,
        F: FnOnce(&Row<'_>) -> rusqlite::Result<T>;
}

impl PreparedCached for Connection {
    fn execute_cached<P: Params>(&self, sql: &str, params: P) -> rusqlite::Result<usize> {
        self.prepare_cached(sql)?.execute(params)
    }

    fn query_row_cached<T, P, F>(&self, sql: &str, params: P, f: F) -> rusqlite::Result<T>
    where
        P: Params,
        F: FnOnce(&Row<'_>) -> rusqlite::Result<T>,
    {
        self.prepare_cached(sql)?.query_row(params, f)
    }
}

/// Fills the list parameter `slot` of the canonical statements with
/// `values` in order (STORAGE §4.4), every slot cleared first: what a
/// statement reading `temp.param` binds before it runs, in its
/// transaction. The connection created the table when it opened.
pub(crate) fn bind_list<V: ToSql>(
    conn: &Connection,
    slot: &str,
    values: impl IntoIterator<Item = V>,
) -> rusqlite::Result<()> {
    conn.execute_cached(sql::CLEAR_PARAM, [])?;
    let mut bind = conn.prepare_cached(sql::BIND_PARAM)?;
    for (position, value) in values.into_iter().enumerate() {
        bind.execute(named_params! {
            ":slot": slot,
            ":position": position as i64,
            ":value": value,
        })?;
    }
    Ok(())
}

/// How long a connection waits on another's write transaction before a
/// statement fails busy (STORAGE §4.1): long enough for a large write
/// batch of the owner, or of a same-source worker, to commit.
const BUSY_TIMEOUT: Duration = Duration::from_secs(30);

/// How many prepared statements a connection keeps ([`PreparedCached`]):
/// above the distinct statements a write batch, a sync's load and an index
/// refresh run together, rusqlite's default of 16 holding a fraction.
const STATEMENT_CACHE: usize = 256;

/// The durability every connection sets (STORAGE §4.1): a commit synced
/// before it reports, and a deleted row's bytes overwritten (§11.5).
#[cfg(not(target_vendor = "apple"))]
const DURABILITY: &str = "PRAGMA synchronous = FULL; PRAGMA secure_delete = ON;";

/// The durability every connection sets (STORAGE §4.1): a commit synced
/// before it reports, through `F_FULLFSYNC` since a plain `fsync` leaves
/// the drive's cache unflushed here, and a deleted row's bytes
/// overwritten (§11.5).
#[cfg(target_vendor = "apple")]
const DURABILITY: &str =
    "PRAGMA synchronous = FULL; PRAGMA fullfsync = ON; PRAGMA secure_delete = ON;";

/// The SQLite version `SQLITE_DBCONFIG_DEFENSIVE` arrived in, as
/// `sqlite3_libversion_number` spells it: an older one has no such flag.
const DEFENSIVE_MIN: i32 = 3_026_000;

/// Sets up a connection to a store or an index as every role does
/// (STORAGE §4.1, §4.4): foreign keys on, the durability pragmas, a busy
/// timeout, and the list parameters' temporary table, which a read-only
/// connection creates too.
///
/// Every store is opened as one from elsewhere (§18), whoever created it:
/// `trusted_schema` off and the defensive flag on, under which the
/// canonical schemas and statements run, so a crafted trigger or view
/// calls no function with side effects and no write reaches the file
/// behind SQLite's back.
pub(crate) fn prepare_connection(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("PRAGMA trusted_schema = OFF; PRAGMA foreign_keys = ON;")?;
    if rusqlite::version_number() >= DEFENSIVE_MIN {
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    }
    conn.execute_batch(DURABILITY)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.set_prepared_statement_cache_capacity(STATEMENT_CACHE);
    conn.execute_batch(sql::CREATE_PARAM)
}

/// How many `TRUNCATE` checkpoints an erase tries, each waiting out the
/// busy timeout, before it reports the store busy (§11.5).
const CHECKPOINT_TRIES: usize = 3;

/// Checkpoints the WAL of `conn`'s main database into it and truncates it
/// (§11.5), again until it reports every frame checkpointed, so no byte a
/// delete overwrote stays in the WAL. A database in another journal mode
/// has nothing to checkpoint; a reader holding the WAL past
/// [`CHECKPOINT_TRIES`] tries is [`PimdirError::Busy`].
pub(crate) fn checkpoint(conn: &Connection) -> Result<(), PimdirError> {
    for _ in 0..CHECKPOINT_TRIES {
        let (busy, log, done): (i64, i64, i64) = conn
            .query_row_cached("PRAGMA main.wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(busy_or_sql)?;
        trace!("checkpoint: busy {busy}, {log} frames, {done} checkpointed");
        if busy == 0 && log == done {
            return Ok(());
        }
    }

    Err(PimdirError::Busy)
}

/// Deletes one queue row by id (`cancel_action`, §15.5), answering the
/// pin it held, `None` when no row had that id.
fn cancel_queued(conn: &Connection, id: i64) -> Result<Option<Option<String>>, PimdirError> {
    Ok(conn
        .query_row_cached(sql::CANCEL_ACTION, named_params! { ":id": id }, |r| {
            r.get(0)
        })
        .optional()?)
}

/// The collection the queue row `id` was queued on (`load_action`),
/// `None` when no row has that id.
fn queued_collection(conn: &Connection, id: i64) -> Result<Option<String>, PimdirError> {
    Ok(conn
        .query_row_cached(sql::LOAD_ACTION, named_params! { ":id": id }, |r| {
            r.get::<_, String>(3)
        })
        .optional()?)
}

/// Records what the removed queue row `id` became (`record_receipt`,
/// §15.2, §15.5), in the caller's transaction.
pub(crate) fn record_receipt(
    conn: &Connection,
    id: i64,
    collection: &str,
    seq: Option<i64>,
) -> Result<(), PimdirError> {
    conn.execute_cached(
        sql::RECORD_RECEIPT,
        named_params! { ":id": id, ":collection": collection, ":seq": seq },
    )?;
    Ok(())
}

/// Releases the pins a deleted row held, set-based, in the caller's
/// transaction.
pub(crate) fn release_pins(
    conn: &Connection,
    hashes: impl Iterator<Item = String>,
) -> Result<(), PimdirError> {
    let hashes: Vec<String> = hashes.collect();
    if hashes.is_empty() {
        return Ok(());
    }
    bind_list(conn, "hashes", &hashes)?;
    conn.execute_cached(sql::RELEASE_PINS, [])?;
    Ok(())
}

/// Maps a SQLite busy or locked failure to [`PimdirError::Busy`].
pub(crate) fn busy_or_sql(err: rusqlite::Error) -> PimdirError {
    match &err {
        rusqlite::Error::SqliteFailure(e, _)
            if matches!(e.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) =>
        {
            PimdirError::Busy
        }
        _ => PimdirError::Sql(PimdirSqlError(err)),
    }
}

/// Why [`PimdirStore::open_current`] refused a store: what
/// [`PimdirStore::open`] would have created (§6).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirNotCurrent {
    /// No database under the directory.
    Absent,
    /// The database holds no schema: no migration ran on it.
    Version {
        /// The store's `PRAGMA user_version`, `0`.
        found: i64,
    },
}

/// What already holds the link id a new `add` names (§15.3), see
/// [`PimdirProducer::enqueue_add`].
///
/// [`PimdirProducer::enqueue_add`]: crate::client::producer::PimdirProducer::enqueue_add
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirDuplicate {
    /// A live item of the collection, by its public id.
    Live(i64),
    /// A pending `add` in the collection, by its queue row id.
    Pending(i64),
}

/// A failure of SQLite, opaque so the public API names no rusqlite type:
/// its message is the cause's, and the cause is its
/// [`source`](core::error::Error::source).
#[derive(Debug)]
pub struct PimdirSqlError(rusqlite::Error);

impl PimdirSqlError {
    /// Whether SQLite answered busy or locked.
    pub(crate) fn is_busy(&self) -> bool {
        matches!(
            &self.0,
            rusqlite::Error::SqliteFailure(failure, _)
                if matches!(failure.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
        )
    }
}

impl fmt::Display for PimdirSqlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for PimdirSqlError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// A failure to encode or decode JSON, opaque so the public API names no
/// serde_json type: its message is the cause's, and the cause is its
/// [`source`](core::error::Error::source).
#[derive(Debug)]
pub struct PimdirJsonError(serde_json::Error);

impl fmt::Display for PimdirJsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for PimdirJsonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// Everything that can go wrong servicing the store.
///
/// [`code`](Self::code) names each failure with a stable code, the
/// contract a consumer maps its own output to; the message is for a
/// person and may change.
#[derive(Debug)]
#[non_exhaustive]
pub enum PimdirError {
    /// The SQLite database refused a statement, or the connection failed.
    Sql(PimdirSqlError),
    /// The blob directory refused a read, a write or a rename.
    Io(io::Error),
    /// JSON encoding failed at the seam.
    Json(PimdirJsonError),
    /// A queue action payload is malformed (§15.3).
    Action(PimdirActionError),
    /// A write resolved an existing binding to a different handle and was
    /// refused (§10): a binding pins one handle, and the one licensed
    /// rebind is a `Superseded` or `Rekeyed` drop in the same batch.
    Rebind {
        /// The collection holding the binding.
        collection: String,
        /// The identity it is keyed by.
        link_id: String,
        /// The source whose binding it is.
        source: String,
        /// The handle the binding holds, and keeps.
        bound: String,
        /// The handle the refused write carried.
        incoming: String,
    },
    /// A write carried a placement no link id names, for a handle no
    /// binding holds: nothing reaches the store unnamed (SYNC §10), a
    /// listing naming every member it carries.
    Unnamed {
        /// The collection of the refused placement.
        collection: String,
        /// Its handle.
        handle: String,
    },
    /// An overruling edit would settle a conflict whose diverging body
    /// nobody has read (SYNC §7, §9): refused, staging nothing, until the
    /// upgrade fetches it.
    Unread {
        /// The collection of the conflicted placement.
        collection: String,
        /// Its handle.
        handle: String,
    },
    /// A sync asked for a bounded scope on a collection of another kind
    /// than mail (SYNC §5).
    Scope {
        /// The collection the sync was asked for.
        collection: String,
        /// Its declared kind, empty when undeclared.
        kind: String,
    },
    /// A file was written to a collection of another kind than files
    /// (§14.3).
    NotFiles {
        /// The collection written to.
        collection: String,
        /// Its declared kind, empty when undeclared.
        kind: String,
    },
    /// An item was put into a collection a source syncs (§14.3): its
    /// items are the source's to write.
    Synced {
        /// The collection written to.
        collection: String,
    },
    /// A collection holding a pending or parked queue row was deleted
    /// (§14, `collections_keep_queued`): cancel the rows first.
    Queued {
        /// The collection the delete was for.
        collection: String,
    },
    /// A role outside the vocabulary of the collection's kind was set on
    /// it (§14), or the collection is not declared.
    Role {
        /// The collection the role was for.
        collection: String,
        /// The role refused.
        role: String,
    },
    /// An item was put into a collection of a kind no unbound write
    /// derives (§14.3): mail, files, or undeclared.
    NotUnbound {
        /// The collection written to.
        collection: String,
        /// Its declared kind, empty when undeclared.
        kind: String,
    },
    /// A create named a link id the collection already holds or has
    /// queued (§15.3), refused before anything was appended.
    Duplicate {
        /// The collection the create was for.
        collection: String,
        /// The link id it named.
        link_id: String,
        /// What holds it.
        existing: PimdirDuplicate,
    },
    /// The store is of another version of the format than this crate's
    /// [`FORMAT_VERSION`](crate::sql::FORMAT_VERSION) (STORAGE §4.2, §6):
    /// another draft's, a release's, or one lacking `format_version`,
    /// which every draft before storage-draft-08 wrote. Nothing in it is
    /// read, reconciled, migrated or written.
    Version {
        /// The store's `store_meta.format_version`, `None` when it lacks
        /// the column.
        found: Option<String>,
    },
    /// The store has no schema yet; only the owner's open creates one.
    Uncreated,
    /// The database is not a pimdir store (§3): its `store_meta.format` is
    /// another, or it declares tables at `user_version` 0. Nothing in it is
    /// read, reconciled or written.
    Foreign,
    /// The two schema stamps disagree, which §4.2 defines as corruption.
    VersionMismatch {
        /// The store's `PRAGMA user_version`.
        user_version: i64,
        /// The version its `store_meta` row records.
        store_meta: i64,
    },
    /// The store is not one [`PimdirStore::open_current`] owns as it
    /// stands: only [`PimdirStore::open`] creates a store (§6).
    NotCurrent(PimdirNotCurrent),
    /// The store names this crate's format version but its schema is not
    /// the canonical one (§4.1, §6): a table, an index or a trigger
    /// missing, added or declared otherwise, or a `user_version` other
    /// than the draft's. Nothing in it is read or written.
    Damaged {
        /// The first object found differing, by name, or `user_version`.
        object: String,
    },
    /// The store's `hash_algo` is not `blake3`, the one algorithm this
    /// crate computes (§5, §6).
    HashAlgo {
        /// The algorithm the store records.
        found: String,
    },
    /// The store's sync state was written by an engine of another version
    /// than this crate's [`SYNC_VERSION`]
    /// (STORAGE §4.3, SYNC §10): no verb runs on it.
    SyncVersion {
        /// The store's `store_meta.sync_version`.
        found: String,
    },
    /// Another writer holds the write lock (§8); retry once it releases.
    Busy,
    /// Another process owns the store (§8), reported at once rather than
    /// waited out.
    Owned(PathBuf),
    /// A producer is between its blob write and the enqueue that pins it
    /// (§8), so a collector cannot run.
    Staging(PathBuf),
    /// A verb of this process is between two of its chunks (§5), so a
    /// collector cannot run; retry once it has returned.
    InFlight(PathBuf),
    /// A path of the store is not its user's alone (§18): its directory or
    /// pimdir.db another user can read or write, or a link where a lock
    /// file belongs. Nothing in it is opened.
    Exposed(PathBuf),
    /// A declared source does not support a capability the action needs
    /// (§15.6), so it is not enqueued, or the owner parks it.
    Unsupported(PimdirRefusal),
    /// No source of the account declares the intent capability (§15.6).
    NoPerformer {
        /// The intent capability.
        capability: String,
    },
    /// Several sources of the account could perform the intent and the user
    /// has chosen none (§15.6): the producer does not pick.
    Ambiguous {
        /// The intent capability.
        capability: String,
        /// The sources declaring it.
        candidates: Vec<String>,
    },
    /// A flag set holds a flag the store cannot (STORAGE §13): empty, or
    /// holding a space, a control or a non-ASCII character. Refused before
    /// anything is written or queued.
    Flag(String),
    /// A purge cutoff is not an instant written as the store's stamps are,
    /// UTC with milliseconds (§11.2), which a textual comparison needs.
    Cutoff(String),
    /// A search query was refused (SEARCH §8.4), for a caller parsing
    /// with `?`.
    #[cfg(feature = "search")]
    Query(crate::search::query::PimdirQueryError),
    /// A tag's flag change was refused (SEARCH §10) before anything was
    /// read or staged.
    #[cfg(feature = "search")]
    Tag(crate::search::tag::PimdirSearchTagError),
    /// The SQLite linked in lacks what the index requires (SEARCH §2):
    /// FTS5, or the version 3.43 `contentless_delete` arrived in.
    #[cfg(feature = "search")]
    Fts5 {
        /// The SQLite version linked in.
        version: &'static str,
    },
    /// A query the grammar accepted could not be answered: the statement
    /// it compiled to failed, which is this implementation's fault, not
    /// the query's.
    #[cfg(feature = "search")]
    Unanswerable(PimdirSqlError),
    /// Another indexer holds index.lock (SEARCH §3) while this one has to
    /// create or rebuild the index.
    #[cfg(feature = "search")]
    Indexing(PathBuf),
    /// The store has no search index yet (SEARCH §2): a query client never
    /// builds one, the store's indexer does.
    #[cfg(feature = "search")]
    IndexMissing(PathBuf),
    /// The search index of this crate's shape and version was built under
    /// another tokenizer or schema text, or from another store or copy of
    /// it (SEARCH §2): a query client never rebuilds it, the store's
    /// indexer does.
    #[cfg(feature = "search")]
    IndexOutdated(PathBuf),
    /// The search index holds no `index_meta`, or names another shape or
    /// version than this crate's (SEARCH §2): an indexer never rebuilds,
    /// writes nor deletes one of another shape or a later version, and a
    /// query client answers from its own shape and version alone.
    #[cfg(feature = "search")]
    IndexVersion {
        /// The index file.
        path: PathBuf,
        /// Its `index_meta.format_version`, `None` without one.
        version: Option<String>,
        /// Its `index_meta.shape`, `None` without one.
        shape: Option<String>,
    },
    /// The zone an indexer states for floating times (SEARCH §7) names no
    /// zone of the IANA time zone database.
    #[cfg(feature = "search")]
    IndexZone(String),
    /// A refresh was asked of an index opened for queries alone (SEARCH
    /// §3).
    #[cfg(feature = "search")]
    IndexQueryOnly(PathBuf),
    /// The index refresh was resumed out of turn (SEARCH §4): a bug in
    /// `PimdirIndex::refresh`, never in the store or the index.
    #[cfg(feature = "search")]
    Refresh(crate::coroutine::PimdirArgError),
}

impl PimdirError {
    /// The failure's stable kebab-case code, for a consumer to map to its
    /// own output (a JSON error code, an exit status) instead of matching
    /// the message. A code never changes meaning; a new failure gets a new
    /// one.
    ///
    /// | Code | Failure |
    /// |---|---|
    /// | `sql` | [`Sql`](Self::Sql) |
    /// | `io` | [`Io`](Self::Io) |
    /// | `json` | [`Json`](Self::Json) |
    /// | `action-malformed` | [`Action`](Self::Action) |
    /// | `binding-rebind` | [`Rebind`](Self::Rebind) |
    /// | `placement-unnamed` | [`Unnamed`](Self::Unnamed) |
    /// | `conflict-unread` | [`Unread`](Self::Unread) |
    /// | `scope-not-mail` | [`Scope`](Self::Scope) |
    /// | `collection-not-files` | [`NotFiles`](Self::NotFiles) |
    /// | `collection-synced` | [`Synced`](Self::Synced) |
    /// | `collection-not-unbound` | [`NotUnbound`](Self::NotUnbound) |
    /// | `collection-role` | [`Role`](Self::Role) |
    /// | `collection-queued` | [`Queued`](Self::Queued) |
    /// | `item-duplicate` | [`Duplicate`](Self::Duplicate) |
    /// | `store-version` | [`Version`](Self::Version) |
    /// | `store-uncreated` | `Uncreated`, or an absent `NotCurrent` |
    /// | `store-foreign` | [`Foreign`](Self::Foreign) |
    /// | `store-corrupt` | [`VersionMismatch`](Self::VersionMismatch), [`Damaged`](Self::Damaged) |
    /// | `store-not-current` | [`NotCurrent`](Self::NotCurrent) otherwise |
    /// | `store-hash-algo` | [`HashAlgo`](Self::HashAlgo) |
    /// | `sync-version` | [`SyncVersion`](Self::SyncVersion) |
    /// | `store-busy` | [`Busy`](Self::Busy) |
    /// | `store-owned` | [`Owned`](Self::Owned) |
    /// | `store-staging` | [`Staging`](Self::Staging) |
    /// | `store-in-flight` | [`InFlight`](Self::InFlight) |
    /// | `store-exposed` | [`Exposed`](Self::Exposed) |
    /// | `unsupported` | [`Unsupported`](Self::Unsupported) |
    /// | `no-performer` | [`NoPerformer`](Self::NoPerformer) |
    /// | `ambiguous-performer` | [`Ambiguous`](Self::Ambiguous) |
    /// | `cutoff-invalid` | [`Cutoff`](Self::Cutoff) |
    /// | `flag-invalid` | [`Flag`](Self::Flag) |
    /// | `query-refused` | `Query` (feature `search`) |
    /// | `tag-refused` | `Tag` (feature `search`) |
    /// | `index-fts5-missing` | `Fts5` (feature `search`) |
    /// | `query-unanswerable` | `Unanswerable` (feature `search`) |
    /// | `index-busy` | `Indexing` (feature `search`) |
    /// | `index-missing` | `IndexMissing` (feature `search`) |
    /// | `index-outdated` | `IndexOutdated` (feature `search`) |
    /// | `index-version` | `IndexVersion` (feature `search`) |
    /// | `index-zone-unknown` | `IndexZone` (feature `search`) |
    /// | `index-query-only` | `IndexQueryOnly` (feature `search`) |
    /// | `index-refresh-broken` | `Refresh` (feature `search`) |
    pub fn code(&self) -> &'static str {
        match self {
            Self::Sql(_) => "sql",
            Self::Io(_) => "io",
            Self::Json(_) => "json",
            Self::Action(_) => "action-malformed",
            Self::Rebind { .. } => "binding-rebind",
            Self::Unnamed { .. } => "placement-unnamed",
            Self::Unread { .. } => "conflict-unread",
            Self::Scope { .. } => "scope-not-mail",
            Self::NotFiles { .. } => "collection-not-files",
            Self::Synced { .. } => "collection-synced",
            Self::NotUnbound { .. } => "collection-not-unbound",
            Self::Role { .. } => "collection-role",
            Self::Queued { .. } => "collection-queued",
            Self::Duplicate { .. } => "item-duplicate",
            Self::Version { .. } => "store-version",
            Self::Uncreated => "store-uncreated",
            Self::Foreign => "store-foreign",
            Self::VersionMismatch { .. } | Self::Damaged { .. } => "store-corrupt",
            Self::NotCurrent(PimdirNotCurrent::Absent) => "store-uncreated",
            Self::NotCurrent(_) => "store-not-current",
            Self::HashAlgo { .. } => "store-hash-algo",
            Self::SyncVersion { .. } => "sync-version",
            Self::Busy => "store-busy",
            Self::Owned(_) => "store-owned",
            Self::Staging(_) => "store-staging",
            Self::InFlight(_) => "store-in-flight",
            Self::Exposed(_) => "store-exposed",
            Self::Unsupported(_) => "unsupported",
            Self::NoPerformer { .. } => "no-performer",
            Self::Ambiguous { .. } => "ambiguous-performer",
            Self::Cutoff(_) => "cutoff-invalid",
            Self::Flag(_) => "flag-invalid",
            #[cfg(feature = "search")]
            Self::Query(_) => "query-refused",
            #[cfg(feature = "search")]
            Self::Tag(_) => "tag-refused",
            #[cfg(feature = "search")]
            Self::Fts5 { .. } => "index-fts5-missing",
            #[cfg(feature = "search")]
            Self::Unanswerable(_) => "query-unanswerable",
            #[cfg(feature = "search")]
            Self::Indexing(_) => "index-busy",
            #[cfg(feature = "search")]
            Self::IndexMissing(_) => "index-missing",
            #[cfg(feature = "search")]
            Self::IndexOutdated(_) => "index-outdated",
            #[cfg(feature = "search")]
            Self::IndexVersion { .. } => "index-version",
            #[cfg(feature = "search")]
            Self::IndexZone(_) => "index-zone-unknown",
            #[cfg(feature = "search")]
            Self::IndexQueryOnly(_) => "index-query-only",
            #[cfg(feature = "search")]
            Self::Refresh(_) => "index-refresh-broken",
        }
    }
}

impl fmt::Display for PimdirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sql(err) => write!(f, "Pimdir SQL error: {err}"),
            Self::Io(err) => write!(f, "Pimdir I/O error: {err}"),
            Self::Json(err) => write!(f, "Pimdir JSON error: {err}"),
            Self::Action(err) => write!(f, "Pimdir action error: {err}"),
            Self::Rebind {
                collection,
                link_id,
                source,
                bound,
                incoming,
            } => write!(
                f,
                "Pimdir binding {collection}/{link_id} on source {source} holds handle {bound} and this write carries {incoming}: a binding pins one handle"
            ),
            Self::Unnamed { collection, handle } => write!(
                f,
                "Pimdir write refused: placement {collection}/{handle} is named by no link id and its handle by no binding"
            ),
            Self::Unread { collection, handle } => write!(
                f,
                "Pimdir write refused: the conflict of {collection}/{handle} holds a body nobody has read, fetch it before overruling it"
            ),
            Self::Scope { collection, kind } => write!(
                f,
                "Pimdir sync refused: a scope bounds mail only, and collection {collection} holds {kind:?}"
            ),
            Self::NotFiles { collection, kind } => write!(
                f,
                "Pimdir file write refused: collection {collection} holds {kind:?}, not files"
            ),
            Self::Synced { collection } => write!(
                f,
                "Pimdir unbound write refused: a source syncs collection {collection}"
            ),
            Self::Queued { collection } => write!(
                f,
                "Pimdir collection delete refused: collection {collection} holds queued actions, cancel them first"
            ),
            Self::Role { collection, role } => write!(
                f,
                "Pimdir collection role refused: {role} is not a role of collection {collection}'s kind"
            ),
            Self::NotUnbound { collection, kind } => write!(
                f,
                "Pimdir unbound write refused: collection {collection} holds {kind:?}, not contacts or calendar items"
            ),
            Self::Duplicate {
                collection,
                link_id,
                existing: PimdirDuplicate::Live(seq),
            } => write!(
                f,
                "Pimdir create refused: collection {collection} already holds {link_id} as item {seq}"
            ),
            Self::Duplicate {
                collection,
                link_id,
                existing: PimdirDuplicate::Pending(id),
            } => write!(
                f,
                "Pimdir create refused: collection {collection} already has {link_id} queued as action {id}"
            ),
            Self::Version { found: Some(found) } => write!(
                f,
                "Pimdir store is {found}, not this program's {}: a store opens under its own version alone, so let the sync engine that wrote it push what it queued, then move it aside and sync a new one",
                sql::FORMAT_VERSION
            ),
            Self::Version { found: None } => write!(
                f,
                "Pimdir store names no format version, as a draft before {0} wrote it, and this program is {0}: a store opens under its own version alone, so let the sync engine that wrote it push what it queued, then move it aside and sync a new one",
                sql::FORMAT_VERSION
            ),
            Self::Uncreated => write!(
                f,
                "Pimdir store has no schema yet: its owner has to create it first"
            ),
            Self::Foreign => write!(
                f,
                "Pimdir store database is not a pimdir store: another program wrote it"
            ),
            Self::VersionMismatch {
                user_version,
                store_meta,
            } => write!(
                f,
                "Pimdir store is corrupt: PRAGMA user_version is {user_version} but store_meta records {store_meta}"
            ),
            Self::NotCurrent(PimdirNotCurrent::Absent) => write!(
                f,
                "Pimdir store does not exist yet: its sync engine has to create it first"
            ),
            Self::NotCurrent(PimdirNotCurrent::Version { found }) => write!(
                f,
                "Pimdir store holds no schema (version {found}): its sync engine has to create it first"
            ),
            Self::Damaged { object } => write!(
                f,
                "Pimdir store is damaged: its schema differs from {} at {object}",
                sql::FORMAT_VERSION
            ),
            Self::HashAlgo { found } => write!(
                f,
                "Pimdir store names its objects with {found}, which this program does not compute"
            ),
            Self::SyncVersion { found } => write!(
                f,
                "Pimdir store's sync state was written by {found}, not this program's {}: a store syncs under one engine version, so let the engine that wrote it push what it queued, then move it aside and sync a new one",
                SYNC_VERSION
            ),
            Self::Busy => write!(
                f,
                "Pimdir store is busy: another writer holds the write lock, retry once it releases"
            ),
            Self::Owned(store) => write!(
                f,
                "Pimdir store at {} is owned by another process",
                store.display()
            ),
            Self::Staging(store) => write!(
                f,
                "Pimdir store at {} has a producer staging a body",
                store.display()
            ),
            Self::InFlight(store) => write!(
                f,
                "Pimdir store at {} has a verb between two of its chunks",
                store.display()
            ),
            Self::Exposed(path) => write!(
                f,
                "Pimdir store path {} is a link or another user can read or write it: a store is its user's alone, so remove the link or restrict it (chmod 700 a directory, 600 a file)",
                path.display()
            ),
            Self::Unsupported(refusal) => write!(f, "{refusal}"),
            Self::NoPerformer { capability } => {
                write!(f, "No source of this account supports {capability}")
            }
            Self::Ambiguous {
                capability,
                candidates,
            } => write!(
                f,
                "Several sources can perform {capability} ({}): choose one",
                candidates.join(", ")
            ),
            Self::Cutoff(cutoff) => write!(
                f,
                "Pimdir purge cutoff {cutoff:?} is not an instant written as the stamps are, UTC with milliseconds (2026-06-01T00:00:00.000Z)"
            ),
            Self::Flag(flag) => write!(
                f,
                "Pimdir flag {flag:?} refused: a flag is printable ASCII without a space"
            ),
            #[cfg(feature = "search")]
            Self::Query(err) => write!(f, "Pimdir search query refused: {err}"),
            #[cfg(feature = "search")]
            Self::Tag(err) => write!(f, "Pimdir tag refused: {err}"),
            #[cfg(feature = "search")]
            Self::Fts5 { version } => write!(
                f,
                "Pimdir search needs SQLite 3.43 or later with FTS5, and SQLite {version} is linked in: build with the vendored feature"
            ),
            #[cfg(feature = "search")]
            Self::Unanswerable(err) => write!(
                f,
                "Pimdir search could not answer an accepted query, an implementation fault: {err}"
            ),
            #[cfg(feature = "search")]
            Self::Indexing(store) => write!(
                f,
                "Pimdir index at {} is being refreshed by another process",
                store.display()
            ),
            #[cfg(feature = "search")]
            Self::IndexMissing(index) => write!(
                f,
                "Pimdir search index {} does not exist yet: run the store's sync engine, which builds it",
                index.display()
            ),
            #[cfg(feature = "search")]
            Self::IndexOutdated(index) => write!(
                f,
                "Pimdir search index {} was built otherwise or from another store: run the store's sync engine, which rebuilds it",
                index.display()
            ),
            #[cfg(feature = "search")]
            Self::IndexVersion {
                path,
                version,
                shape,
            } => write!(
                f,
                "Pimdir search index {} is of shape {} at {}, and this build implements {} at {}: an index of its shape at an earlier version is rebuilt by the store's sync engine, any other is left as it is",
                path.display(),
                shape.as_deref().unwrap_or("none"),
                version.as_deref().unwrap_or("no version"),
                crate::sql::search::SHAPE,
                crate::sql::search::FORMAT_VERSION,
            ),
            #[cfg(feature = "search")]
            Self::IndexZone(zone) => write!(
                f,
                "Pimdir search cannot read floating times in {zone}: name a zone of the IANA time zone database"
            ),
            #[cfg(feature = "search")]
            Self::IndexQueryOnly(index) => write!(
                f,
                "Pimdir search index {} is open for queries alone: only its indexer refreshes it",
                index.display()
            ),
            #[cfg(feature = "search")]
            Self::Refresh(err) => write!(f, "Pimdir index refresh failed: {err}"),
        }
    }
}

impl std::error::Error for PimdirError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sql(err) => Some(err),
            Self::Io(err) => Some(err),
            Self::Json(err) => Some(err),
            #[cfg(feature = "search")]
            Self::Unanswerable(err) => Some(err),
            #[cfg(feature = "search")]
            Self::Refresh(err) => Some(err),
            _ => None,
        }
    }
}

#[doc(hidden)]
impl From<rusqlite::Error> for PimdirError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sql(PimdirSqlError(err))
    }
}

impl From<PimdirRefusal> for PimdirError {
    fn from(refusal: PimdirRefusal) -> Self {
        Self::Unsupported(refusal)
    }
}

impl From<io::Error> for PimdirError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

#[doc(hidden)]
impl From<serde_json::Error> for PimdirError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(PimdirJsonError(err))
    }
}

impl From<PimdirActionError> for PimdirError {
    fn from(err: PimdirActionError) -> Self {
        Self::Action(err)
    }
}

#[cfg(feature = "search")]
impl From<crate::search::tag::PimdirSearchTagError> for PimdirError {
    fn from(err: crate::search::tag::PimdirSearchTagError) -> Self {
        Self::Tag(err)
    }
}

#[cfg(feature = "search")]
impl From<crate::search::query::PimdirQueryError> for PimdirError {
    fn from(err: crate::search::query::PimdirQueryError) -> Self {
        Self::Query(err)
    }
}

/// Whether `value` is an instant written as the store's stamps are,
/// `%Y-%m-%dT%H:%M:%fZ` (STORAGE §11.2): what a textual comparison with
/// them needs.
pub(crate) fn is_stamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 24
        && bytes.iter().enumerate().all(|(at, byte)| match at {
            4 | 7 => *byte == b'-',
            10 => *byte == b'T',
            13 | 16 => *byte == b':',
            19 => *byte == b'.',
            23 => *byte == b'Z',
            _ => byte.is_ascii_digit(),
        })
}

#[cfg(test)]
mod tests {
    use rusqlite::{Connection, config::DbConfig};

    use crate::client::prepare_connection;

    /// Every connection syncs a commit fully, deletes securely and waits
    /// out a writer (STORAGE §4.1).
    #[test]
    fn a_connection_syncs_fully_deletes_securely_and_waits() {
        let conn = Connection::open_in_memory().unwrap();
        prepare_connection(&conn).unwrap();
        let pragma =
            |name: &str| -> i64 { conn.pragma_query_value(None, name, |r| r.get(0)).unwrap() };
        assert_eq!(pragma("foreign_keys"), 1);
        assert_eq!(pragma("synchronous"), 2, "FULL");
        assert_eq!(pragma("secure_delete"), 1);
        assert_eq!(pragma("busy_timeout"), 30_000);
    }

    /// Every connection opens its store as one from elsewhere (STORAGE
    /// §18): no function a schema names runs unless innocuous, and no
    /// write reaches the file behind SQLite's back.
    #[test]
    fn a_connection_trusts_no_schema_and_defends_its_file() {
        let conn = Connection::open_in_memory().unwrap();
        prepare_connection(&conn).unwrap();
        let trusted: i64 = conn
            .pragma_query_value(None, "trusted_schema", |r| r.get(0))
            .unwrap();
        assert_eq!(trusted, 0);
        assert!(conn.db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE).unwrap());

        conn.execute_batch("CREATE TABLE t (x); PRAGMA writable_schema = ON;")
            .unwrap();
        let edited = conn.execute("UPDATE sqlite_schema SET sql = '' WHERE name = 't'", []);
        assert!(edited.is_err(), "the schema table is not writable");
    }
}
