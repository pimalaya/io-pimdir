//! What a consistency check asks *about* the index rather than through it.
//!
//! These reads observe invariants the store maintains: whether a refcount
//! matches the references justifying it, whether a row points at something
//! absent, what the bodies weigh. They sat in the operator CLI on a second
//! read-only connection, on the reasoning that publishing them would publish
//! invariants the library maintains rather than observes. That stopped holding
//! when the library gained the repairs (`recompute_refcounts`,
//! `clear_dangling_bindings`): a repair that cannot report what it found is a
//! worse seam than a diagnostic that can.
//!
//! Every statement here is a `SELECT`, and each runs on the handle the caller
//! already holds, read-only or owning.

use alloc::{format, string::String, vec::Vec};
use std::collections::BTreeSet;

use rusqlite::{OptionalExtension, named_params};
use serde::Serialize;

use crate::{
    client::{PimdirError, PimdirStore, rows},
    sql,
};

/// How many objects the index holds and what they weigh.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct PimdirObjectStats {
    /// Indexed objects.
    pub count: u64,
    /// Their total size in bytes.
    pub bytes: u64,
}

/// One object whose stored refcount disagrees with the references that justify
/// it (items, conflict copies, per-source bases and queue pins).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PimdirRefcountDrift {
    /// The object's content hash.
    pub hash: String,
    /// The refcount the index stores.
    pub stored: i64,
    /// The refcount its references add up to.
    pub expected: i64,
}

/// One binding whose source holds its identity under more than one handle, so
/// the engine cannot say which copy a change belongs to and stops deriving.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PimdirAmbiguous {
    /// The owning collection.
    pub collection: String,
    /// The identity held more than once.
    pub link_id: String,
    /// The source holding it more than once.
    pub source: String,
    /// How many copies of it that source holds.
    pub copies: i64,
}

/// One row referencing something that is not there.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PimdirDangling {
    /// What kind of row dangles (`binding`, `item-object`, `queue-object`).
    pub kind: &'static str,
    /// The row, as an operator would name it.
    pub row: String,
    /// What it points at and cannot find.
    pub target: String,
}

impl PimdirStore {
    /// How many objects are indexed and what they weigh in total.
    pub fn object_stats(&self) -> Result<PimdirObjectStats, PimdirError> {
        let (count, bytes) = self.conn.query_row(sql::OBJECT_STATS, [], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })?;
        Ok(PimdirObjectStats {
            count: count.max(0) as u64,
            bytes: bytes.max(0) as u64,
        })
    }

    /// The bytes held by objects at least one **live** item still binds.
    ///
    /// An object a live and a retained item share counts here, since purging
    /// the retained one would not free it.
    pub fn live_bytes(&self) -> Result<u64, PimdirError> {
        let bytes: i64 = self.conn.query_row(sql::LIVE_BYTES, [], |r| r.get(0))?;
        Ok(bytes.max(0) as u64)
    }

    /// One object's stored size.
    pub fn object_size(&self, hash: &str) -> Result<Option<u64>, PimdirError> {
        let size: Option<i64> = self
            .conn
            .query_row(sql::OBJECT_SIZE, named_params! { ":hash": hash }, |r| {
                r.get(0)
            })
            .optional()?;
        Ok(size.map(|size| size.max(0) as u64))
    }

    /// What a purge with this cutoff would retire: how many retained items, and
    /// the bytes their bodies weigh.
    ///
    /// A preview, so a confirmation can say what is at stake; the purge itself
    /// is the authority, and the collector is what frees the bytes.
    pub fn retained_before(&self, cutoff: &str) -> Result<(u64, u64), PimdirError> {
        let (count, bytes) = self.conn.query_row(
            sql::COUNT_RETAINED_BEFORE,
            named_params! { ":cutoff": cutoff },
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )?;
        Ok((count.max(0) as u64, bytes.max(0) as u64))
    }

    /// Every hash the index knows, to diff against the blob directory: the
    /// index half of what [`PimdirBlobs::files`](crate::client::PimdirBlobs::files)
    /// reads from disk.
    pub fn indexed_hashes(&self) -> Result<BTreeSet<String>, PimdirError> {
        Ok(rows(&self.conn, sql::LIST_OBJECT_HASHES, [], |r| r.get(0))?
            .into_iter()
            .collect())
    }

    /// The objects whose stored refcount disagrees with their references.
    ///
    /// The expected count is exactly what the write path maintains
    /// incrementally: an item's body, an item's conflict copy, each source's
    /// stored base, and each queue row pinning a body it enqueued.
    /// [`recompute_refcounts`](PimdirStore::recompute_refcounts) is what
    /// settles what this reports.
    pub fn refcount_drift(&self) -> Result<Vec<PimdirRefcountDrift>, PimdirError> {
        Ok(rows(&self.conn, sql::REFCOUNT_DRIFT, [], |r| {
            Ok(PimdirRefcountDrift {
                hash: r.get(0)?,
                stored: r.get(1)?,
                expected: r.get(2)?,
            })
        })?)
    }

    /// The bindings holding an identity their source holds more than once.
    ///
    /// Not a defect: two copies of one message is redundancy, and the store
    /// records it rather than judging it. It is reported because it is the
    /// reason those items stop syncing, and an operator looking at a frozen
    /// item has no other way to see why.
    pub fn ambiguous_bindings(&self) -> Result<Vec<PimdirAmbiguous>, PimdirError> {
        Ok(rows(&self.conn, sql::AMBIGUOUS_BINDINGS, [], |r| {
            Ok(PimdirAmbiguous {
                collection: r.get(0)?,
                link_id: r.get(1)?,
                source: r.get(2)?,
                copies: r.get(3)?,
            })
        })?)
    }

    /// Every row pointing at something absent: a binding whose item is gone, an
    /// item or a queue row whose object is not indexed.
    ///
    /// Only the first is repairable ([`clear_dangling_bindings`](PimdirStore::clear_dangling_bindings));
    /// the other two still hold data, so they are reported and left alone.
    pub fn dangling(&self) -> Result<Vec<PimdirDangling>, PimdirError> {
        let mut dangling = rows(&self.conn, sql::DANGLING_BINDINGS, [], |r| {
            Ok(PimdirDangling {
                kind: "binding",
                row: format!(
                    "{}/{} @{}",
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?
                ),
                target: format!("item {}/{}", r.get::<_, String>(0)?, r.get::<_, String>(1)?),
            })
        })?;

        dangling.extend(rows(&self.conn, sql::DANGLING_ITEM_OBJECTS, [], |r| {
            Ok(PimdirDangling {
                kind: "item-object",
                row: format!("{}/{}", r.get::<_, String>(0)?, r.get::<_, String>(1)?),
                target: format!("object {}", r.get::<_, String>(2)?),
            })
        })?);

        dangling.extend(rows(&self.conn, sql::DANGLING_QUEUE_OBJECTS, [], |r| {
            Ok(PimdirDangling {
                kind: "queue-object",
                row: format!("queue {} ({})", r.get::<_, i64>(0)?, r.get::<_, String>(1)?),
                target: format!("object {}", r.get::<_, String>(2)?),
            })
        })?);

        Ok(dangling)
    }
}
