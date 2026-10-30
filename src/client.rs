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
};

use rusqlite::{
    Connection, ErrorCode, OpenFlags, OptionalExtension, Params, Row, TransactionBehavior,
    named_params,
};

use crate::{
    capability::{PimdirCapability, PimdirRefusal},
    client::{
        blobs::{PimdirBlobStage, PimdirBlobs},
        lock::PimdirLock,
        reader::{PimdirReader, collections_json, reference_from_row},
        write::kind_of,
    },
    codec::{self, PimdirAction, PimdirActionError},
    hash::PimdirHashAlgo,
    hub::{PimdirHub, PimdirHubConflict, PimdirSourceId},
    object::PimdirObject,
    placement::PimdirLinkId,
    reference::{PimdirEndpoint, PimdirReference, PimdirReferenceOrigin, PimdirReferenceRole},
    sql,
    summary::{
        self,
        file::{self, PimdirFileSummary},
    },
};

/// The local time collection's id (§14.3): `U+0001` and `time`, under no
/// account, a name no server gives a collection, so every owner of a store
/// finds the same one.
pub const TIME_COLLECTION: &str = "\u{1}time";

pub mod blobs;
mod capability;
pub mod diagnostics;
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
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PimdirGcReport {
    /// Object rows dropped: bodies nothing references any more.
    pub objects: usize,
    /// Blob files unlinked: those rows' bodies and the orphans a crash left.
    pub blobs: usize,
    /// The bytes those files freed.
    pub bytes: u64,
}

impl PimdirStore {
    /// Opens (creating if absent) the store rooted at `dir` as its owner.
    ///
    /// Takes the store's exclusive advisory lock (§8) and holds it until
    /// the handle drops; a store another process owns is
    /// [`PimdirError::Owned`] at once, never a wait. A fresh database is
    /// created by running the migrations (§6). An existing store is opened
    /// in §6's order, refused before anything is written when it is above
    /// this crate's version ([`PimdirError::Version`]), not a pimdir store
    /// ([`PimdirError::Foreign`]), missing a core table
    /// ([`PimdirError::Stale`]) or naming its objects by another algorithm;
    /// a version 1 store an earlier draft wrote is then reconciled to
    /// 0001_init.sql, and every migration above its version run.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, PimdirError> {
        Self::open_with_hash(dir, None)
    }

    /// Opens the existing store rooted at `dir` as its owner only as it
    /// stands: unlike [`open`](Self::open), it creates, migrates and
    /// reconciles nothing (§6), so a caller that owns the store for a
    /// moment (one local write) never rebuilds a store another owner of
    /// another revision keeps.
    ///
    /// Refused, writing nothing, as [`PimdirError::NotCurrent`]: no
    /// database under `dir` ([`PimdirNotCurrent::Absent`]), a
    /// `user_version` other than this crate's
    /// ([`PimdirNotCurrent::Version`]), or a schema whose text differs
    /// from the canonical one, which `open` would reconcile
    /// ([`PimdirNotCurrent::Shape`]). Otherwise as `open`: the same lock,
    /// [`PimdirError::Owned`] at once when another process holds it, and
    /// the refusals of a damaged store ([`PimdirError::Stale`],
    /// [`PimdirError::VersionMismatch`], [`PimdirError::HashAlgo`]).
    pub fn open_current(dir: impl AsRef<Path>) -> Result<Self, PimdirError> {
        let dir = dir.as_ref();
        let db = dir.join("pimdir.db");
        if !db.is_file() {
            return Err(PimdirError::NotCurrent(PimdirNotCurrent::Absent));
        }

        let lock = PimdirLock::own(dir)?;

        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(&db, flags)?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 30000;")?;
        schema::current(&conn)?;
        let hash = schema::hash_algo(&conn, None)?;

        Ok(Self {
            reader: PimdirReader::over(conn, dir.to_path_buf(), hash)?,
            lock,
            account: None,
        })
    }

    /// Opens the store rooted at `dir`, declaring the hash its objects
    /// are named by (§5) when it creates one; an existing store whose
    /// algorithm differs is [`PimdirError::HashAlgo`].
    pub fn open_with_hash(
        dir: impl AsRef<Path>,
        hash: Option<PimdirHashAlgo>,
    ) -> Result<Self, PimdirError> {
        let dir = dir.as_ref();
        fs::create_dir_all(dir)?;
        fs::create_dir_all(dir.join("objects"))?;

        let lock = PimdirLock::own(dir)?;

        let mut conn = Connection::open(dir.join("pimdir.db"))?;
        // NOTE: the journal mode is set by the schema runner once it knows
        // the database is a store it may write (§6).
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 30000;")?;
        let hash = schema::init(&mut conn, hash, dir)?;

        Ok(Self {
            reader: PimdirReader::over(conn, dir.to_path_buf(), hash)?,
            lock,
            account: None,
        })
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
            .execute(
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
            .execute(
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
            .execute(
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
            .query_row(
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
            .query_row(
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
            .query_row(sql::SEQ_BY_LINK, key, |row| row.get(0))
            .optional()?;
        let seq = match held {
            Some(seq) => {
                tx.execute(
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
            hash: self.hash.hash(body),
            size: body.len(),
        };
        let blobs = PimdirBlobs::open(&self.dir, self.hash);
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        let synced = tx
            .prepare(sql::COLLECTION_SOURCES)?
            .exists(named_params! { ":collection": collection })?;
        if synced {
            return Err(PimdirError::Synced {
                collection: collection.into(),
            });
        }
        // NOTE: inside the transaction, so no collector runs between the
        // blob landing and the row pinning it (§5).
        blobs.write(&object.hash, body)?;
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
        tx.execute(
            sql::SET_COLLECTION_KIND,
            named_params! {
                ":collection": TIME_COLLECTION,
                ":account": None::<&str>,
                ":kind": "text/calendar",
            },
        )?;
        tx.execute(
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
            .execute(
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
            .execute(
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
            .execute(
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
    /// one. A pending move or copy naming it as a target follows too, its
    /// payload being what no cascade reaches.
    pub fn rename_collection(
        &mut self,
        collection: impl AsRef<str>,
        new_id: &str,
    ) -> Result<(), PimdirError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        tx.execute(
            sql::RENAME_QUEUE_TARGETS,
            named_params! { ":collection": collection.as_ref(), ":new_id": new_id },
        )?;
        tx.execute(
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
        let pins: bool = tx.query_row(sql::COLLECTION_HOLDS_OBJECTS, key, |row| row.get(0))?;
        let deleted = match tx.execute(sql::DELETE_COLLECTION, key) {
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
            tx.execute(sql::RECOMPUTE_REFCOUNTS, [])?;
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
            .prepare(sql::DELETE_UNBOUND_ITEM)?
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
            .prepare(sql::PURGE_ITEM)?
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
        tx.execute(sql::RECOMPUTE_REFCOUNTS, [])?;
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
        let collections = collections_json(collections)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        tx.execute(
            sql::RELEASE_BASES_BEFORE,
            named_params! { ":collections": collections, ":until": until },
        )?;
        let seqs: Vec<i64> = rows(
            &tx,
            sql::RELEASE_BEFORE,
            named_params! { ":collections": collections, ":until": until },
            |row| row.get(0),
        )?;
        tx.execute(sql::RECOMPUTE_REFCOUNTS, [])?;
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
        tx.execute(sql::RECOMPUTE_REFCOUNTS, [])?;
        let objects = tx.execute(sql::DELETE_GARBAGE_OBJECTS, [])?;
        tx.commit().map_err(busy_or_sql)?;

        let mut report = PimdirGcReport {
            objects,
            ..Default::default()
        };
        let mut exists = self.conn.prepare(sql::OBJECT_EXISTS)?;
        for blob in self.blobs().files()? {
            if exists.exists(named_params! { ":hash": blob.hash })? {
                continue;
            }
            fs::remove_file(&blob.path)?;
            report.blobs += 1;
            report.bytes += blob.size;
        }
        drop(exists);

        Ok(report)
    }

    /// Recomputes every refcount from the five pointer columns (§7),
    /// returning how many rows disagreed.
    pub fn recompute_refcounts(&self) -> Result<usize, PimdirError> {
        self.conn
            .execute(sql::RECOMPUTE_REFCOUNTS, [])
            .map_err(busy_or_sql)
    }

    /// Deletes the bindings whose item is gone, returning how many: the
    /// one dangling row a repair clears without guessing.
    pub fn clear_dangling_bindings(&self) -> Result<usize, PimdirError> {
        self.conn
            .execute(sql::DELETE_DANGLING_BINDINGS, [])
            .map_err(busy_or_sql)
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
    /// owning the store for one write never migrates nor reconciles it, so
    /// a store of another revision is refused as
    /// [`PimdirError::NotCurrent`] and left as it was.
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
    /// not, nothing is enqueued.
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
        let hash = action.object_hash().cloned();
        if let Some(hash) = &hash {
            tx.execute(sql::PIN_OBJECT, named_params! { ":hash": hash.0 })?;
        }
        tx.execute(
            sql::ENQUEUE_ACTION,
            named_params! {
                ":producer": producer,
                ":collection": collection,
                ":action": action.kind(),
                ":payload": codec::action_to_payload(action),
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
    /// leaves the row pending, `Some(error)` parks it, the attempt counted.
    pub fn fail_action(&self, id: i64, error: Option<&str>) -> Result<(), PimdirError> {
        let Some(error) = error else {
            self.conn
                .execute(sql::BUMP_ATTEMPTS, named_params! { ":id": id })
                .map_err(busy_or_sql)?;
            return Ok(());
        };

        self.conn
            .execute(
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

/// Runs one statement and collects every row through `map`.
pub(crate) fn rows<T>(
    conn: &Connection,
    sql: &str,
    params: impl Params,
    map: impl FnMut(&Row) -> rusqlite::Result<T>,
) -> rusqlite::Result<Vec<T>> {
    conn.prepare(sql)?.query_map(params, map)?.collect()
}

/// Deletes one queue row by id (`cancel_action`, §15.5), answering the
/// pin it held, `None` when no row had that id.
fn cancel_queued(conn: &Connection, id: i64) -> Result<Option<Option<String>>, PimdirError> {
    Ok(conn
        .query_row(sql::CANCEL_ACTION, named_params! { ":id": id }, |r| {
            r.get(0)
        })
        .optional()?)
}

/// The collection the queue row `id` was queued on (`load_action`),
/// `None` when no row has that id.
fn queued_collection(conn: &Connection, id: i64) -> Result<Option<String>, PimdirError> {
    Ok(conn
        .query_row(sql::LOAD_ACTION, named_params! { ":id": id }, |r| {
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
    conn.execute(
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
    conn.execute(
        sql::RELEASE_PINS,
        named_params! { ":hashes": serde_json::to_string(&hashes)? },
    )?;
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
/// [`PimdirStore::open`] would have created, migrated or reconciled (§6).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirNotCurrent {
    /// No database under the directory.
    Absent,
    /// The database's `user_version` is not this crate's: `0` for one no
    /// migration ran on, above it for a later crate's.
    Version {
        /// The store's `PRAGMA user_version`.
        found: i64,
    },
    /// A table, index or trigger whose stored text is not the canonical
    /// one, or that only one side declares: a store an owner of another
    /// revision wrote or reconciled.
    Shape {
        /// The first such object, by name.
        object: String,
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
    /// The store's schema version is not one this crate services: above
    /// it, a later program wrote the store.
    Version {
        /// The store's `user_version`.
        found: i64,
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
    /// stands: only [`PimdirStore::open`] creates, migrates or reconciles
    /// a store (§6).
    NotCurrent(PimdirNotCurrent),
    /// The store was written by an earlier draft and lacks a table or a
    /// trigger the current schema declares (§6).
    Stale {
        /// The first table or trigger found missing.
        missing: &'static str,
    },
    /// The store's `hash_algo` is not one this crate computes, or not the
    /// one the caller declared (§5).
    HashAlgo {
        /// The algorithm the store records.
        found: String,
        /// The algorithm the caller declared, when it declared one.
        declared: Option<&'static str>,
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
    Tag(crate::client::search::tag::PimdirSearchTagError),
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
    /// The search index is at another schema version, tokenizer or build
    /// than this crate's (SEARCH §2): a query client never rebuilds it,
    /// the store's indexer does.
    #[cfg(feature = "search")]
    IndexOutdated(PathBuf),
    /// A refresh was asked of an index opened for queries alone (SEARCH
    /// §3).
    #[cfg(feature = "search")]
    IndexQueryOnly(PathBuf),
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
    /// | `store-newer` | [`Version`](Self::Version), or `NotCurrent` above this crate's |
    /// | `store-uncreated` | `Uncreated`, or an absent `NotCurrent` |
    /// | `store-foreign` | [`Foreign`](Self::Foreign) |
    /// | `store-corrupt` | [`VersionMismatch`](Self::VersionMismatch) |
    /// | `store-not-current` | [`NotCurrent`](Self::NotCurrent) otherwise |
    /// | `store-stale` | [`Stale`](Self::Stale) |
    /// | `store-hash-algo` | [`HashAlgo`](Self::HashAlgo) |
    /// | `store-busy` | [`Busy`](Self::Busy) |
    /// | `store-owned` | [`Owned`](Self::Owned) |
    /// | `store-staging` | [`Staging`](Self::Staging) |
    /// | `store-in-flight` | [`InFlight`](Self::InFlight) |
    /// | `unsupported` | [`Unsupported`](Self::Unsupported) |
    /// | `no-performer` | [`NoPerformer`](Self::NoPerformer) |
    /// | `ambiguous-performer` | [`Ambiguous`](Self::Ambiguous) |
    /// | `cutoff-invalid` | [`Cutoff`](Self::Cutoff) |
    /// | `query-refused` | `Query` (feature `search`) |
    /// | `tag-refused` | `Tag` (feature `search`) |
    /// | `index-fts5-missing` | `Fts5` (feature `search`) |
    /// | `query-unanswerable` | `Unanswerable` (feature `search`) |
    /// | `index-busy` | `Indexing` (feature `search`) |
    /// | `index-missing` | `IndexMissing` (feature `search`) |
    /// | `index-outdated` | `IndexOutdated` (feature `search`) |
    /// | `index-query-only` | `IndexQueryOnly` (feature `search`) |
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
            Self::Version { .. } => "store-newer",
            Self::Uncreated => "store-uncreated",
            Self::Foreign => "store-foreign",
            Self::VersionMismatch { .. } => "store-corrupt",
            Self::NotCurrent(PimdirNotCurrent::Absent) => "store-uncreated",
            Self::NotCurrent(PimdirNotCurrent::Version { found }) if *found > sql::VERSION => {
                "store-newer"
            }
            Self::NotCurrent(_) => "store-not-current",
            Self::Stale { .. } => "store-stale",
            Self::HashAlgo { .. } => "store-hash-algo",
            Self::Busy => "store-busy",
            Self::Owned(_) => "store-owned",
            Self::Staging(_) => "store-staging",
            Self::InFlight(_) => "store-in-flight",
            Self::Unsupported(_) => "unsupported",
            Self::NoPerformer { .. } => "no-performer",
            Self::Ambiguous { .. } => "ambiguous-performer",
            Self::Cutoff(_) => "cutoff-invalid",
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
            Self::IndexQueryOnly(_) => "index-query-only",
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
            Self::Version { found } => write!(
                f,
                "Pimdir store schema version {found} is newer than this program's {}: upgrade this program",
                sql::VERSION
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
            Self::NotCurrent(PimdirNotCurrent::Version { found }) if *found > sql::VERSION => {
                write!(
                    f,
                    "Pimdir store schema version {found} is newer than this program's {}: upgrade this program",
                    sql::VERSION
                )
            }
            Self::NotCurrent(PimdirNotCurrent::Version { found }) => write!(
                f,
                "Pimdir store schema version {found} is not this program's {}: run or upgrade the store's sync engine, which migrates it",
                sql::VERSION
            ),
            Self::NotCurrent(PimdirNotCurrent::Shape { object }) => write!(
                f,
                "Pimdir store schema differs from this program's at {object}: run or upgrade the store's sync engine, which reconciles it"
            ),
            Self::Stale { missing } => write!(
                f,
                "Pimdir store was written by an earlier draft and lacks {missing}: run or upgrade the store's sync engine"
            ),
            Self::HashAlgo {
                found,
                declared: Some(declared),
            } => write!(
                f,
                "Pimdir store names its objects with {found}, not the {declared} this handle declared"
            ),
            Self::HashAlgo { found, .. } => write!(
                f,
                "Pimdir store names its objects with {found}, which this crate does not compute"
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
                "Pimdir search index {} was built by another version: run the store's sync engine, which rebuilds it",
                index.display()
            ),
            #[cfg(feature = "search")]
            Self::IndexQueryOnly(index) => write!(
                f,
                "Pimdir search index {} is open for queries alone: only its indexer refreshes it",
                index.display()
            ),
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
impl From<crate::client::search::tag::PimdirSearchTagError> for PimdirError {
    fn from(err: crate::client::search::tag::PimdirSearchTagError) -> Self {
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
