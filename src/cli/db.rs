//! The read-only diagnostic connection.
//!
//! `check` and the object figures of `store info` ask questions *about* the
//! index rather than through it: how many objects are indexed, how many bytes
//! they weigh, whether a refcount matches the references that justify it,
//! whether a binding still has its item. The library maintains those invariants
//! and has no reason to publish them as an API, so the operator tool reads the
//! index directly here, exactly as `sqlite3` would, and nowhere else.
//!
//! Every statement in this module is a `SELECT` over a connection opened with
//! `SQLITE_OPEN_READ_ONLY`.

use std::{collections::BTreeSet, path::Path};

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags, OptionalExtension, named_params};
use serde::Serialize;

/// A read-only SQLite handle on a store's index, for diagnostics.
pub struct PimdirDb {
    conn: Connection,
}

/// How many objects the index holds and what they weigh.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct PimdirObjectStats {
    /// Indexed objects.
    pub count: u64,
    /// Their total size in bytes.
    pub bytes: u64,
}

/// One object whose stored refcount disagrees with the references that justify
/// it (items, conflict copies, per-source bases and queue pins).
#[derive(Clone, Debug, Serialize)]
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
#[derive(Clone, Debug, Serialize)]
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
#[derive(Clone, Debug, Serialize)]
pub struct PimdirDangling {
    /// What kind of row dangles (`binding`, `item-object`, `queue-object`, …).
    pub kind: &'static str,
    /// The row, as an operator would name it.
    pub row: String,
    /// What it points at and cannot find.
    pub target: String,
}

impl PimdirDb {
    /// Opens `dir`'s index read-only.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(dir.join("pimdir.db"), flags)
            .with_context(|| format!("cannot read the index at {}", dir.display()))?;
        conn.execute_batch("PRAGMA busy_timeout = 30000;")?;

        Ok(Self { conn })
    }

    /// The schema version the store is stamped with.
    pub fn version(&self) -> Result<i64> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    /// How many objects are indexed and what they weigh in total.
    pub fn object_stats(&self) -> Result<PimdirObjectStats> {
        let (count, bytes) = self.conn.query_row(
            "SELECT count(*), coalesce(sum(size), 0) FROM objects",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )?;
        Ok(PimdirObjectStats {
            count: count.max(0) as u64,
            bytes: bytes.max(0) as u64,
        })
    }

    /// The bytes held by objects at least one live (non-retained) item still
    /// binds. An object shared by a live and a retained item counts here, since
    /// purging the retained one would not free it.
    pub fn live_bytes(&self) -> Result<u64> {
        let bytes: i64 = self.conn.query_row(
            "SELECT coalesce(sum(size), 0) FROM objects WHERE hash IN \
             (SELECT object_hash FROM items \
              WHERE object_hash IS NOT NULL AND retained_at IS NULL)",
            [],
            |r| r.get(0),
        )?;
        Ok(bytes.max(0) as u64)
    }

    /// One object's stored size.
    pub fn object_size(&self, hash: &str) -> Result<Option<u64>> {
        let size: Option<i64> = self
            .conn
            .query_row(
                "SELECT size FROM objects WHERE hash = :hash",
                named_params! { ":hash": hash },
                |r| r.get(0),
            )
            .optional()?;
        Ok(size.map(|size| size.max(0) as u64))
    }

    /// What a purge with this cutoff would destroy: how many retained items,
    /// and the bytes their bodies weigh. A preview, so a confirmation can say
    /// what is at stake; the store's own purge is the authority.
    pub fn retained_before(&self, cutoff: &str) -> Result<(u64, u64)> {
        let (count, bytes) = self.conn.query_row(
            "SELECT count(*), coalesce(sum(o.size), 0) FROM items i \
             LEFT JOIN objects o ON o.hash = i.object_hash \
             WHERE i.retained_at IS NOT NULL AND i.retained_at < :cutoff",
            named_params! { ":cutoff": cutoff },
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )?;
        Ok((count.max(0) as u64, bytes.max(0) as u64))
    }

    /// Every hash the index knows, to diff against the blob directory.
    pub fn hashes(&self) -> Result<BTreeSet<String>> {
        let mut stmt = self.conn.prepare("SELECT hash FROM objects")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut hashes = BTreeSet::new();
        for row in rows {
            hashes.insert(row?);
        }
        Ok(hashes)
    }

    /// The objects whose stored refcount disagrees with their references.
    ///
    /// The expected count mirrors what the write path maintains incrementally:
    /// an item's body, an item's conflict copy, each source's stored base, and
    /// each queue row pinning a body it enqueued.
    pub fn refcount_drift(&self) -> Result<Vec<PimdirRefcountDrift>> {
        let mut stmt = self.conn.prepare(
            "WITH refs(hash) AS ( \
                 SELECT object_hash FROM items WHERE object_hash IS NOT NULL \
                 UNION ALL SELECT conflict_object FROM items WHERE conflict_object IS NOT NULL \
                 UNION ALL SELECT base_object FROM bindings WHERE base_object IS NOT NULL \
                 UNION ALL SELECT object_hash FROM queue WHERE object_hash IS NOT NULL \
             ), counted(hash, n) AS (SELECT hash, count(*) FROM refs GROUP BY hash) \
             SELECT o.hash, o.refcount, coalesce(c.n, 0) FROM objects o \
             LEFT JOIN counted c ON c.hash = o.hash \
             WHERE o.refcount != coalesce(c.n, 0) ORDER BY o.hash",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(PimdirRefcountDrift {
                hash: r.get(0)?,
                stored: r.get(1)?,
                expected: r.get(2)?,
            })
        })?;
        let mut drifts = Vec::new();
        for row in rows {
            drifts.push(row?);
        }
        Ok(drifts)
    }

    /// The bindings holding an identity their source holds more than once, as
    /// `(collection, link_id, source, handle count)`.
    ///
    /// Not a defect: two copies of one message is redundancy, and the store
    /// records it rather than judging it. It is reported because it is the
    /// reason those items stop syncing, and an operator looking at a frozen
    /// item has no other way to see why.
    pub fn ambiguous_bindings(&self) -> Result<Vec<PimdirAmbiguous>> {
        let mut stmt = self.conn.prepare(
            "SELECT collection, link_id, source, \
                    json_array_length(ambiguous_handles) + 1 \
             FROM bindings WHERE ambiguous_handles IS NOT NULL \
             ORDER BY collection, link_id, source",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(PimdirAmbiguous {
                collection: r.get(0)?,
                link_id: r.get(1)?,
                source: r.get(2)?,
                copies: r.get(3)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Every row pointing at something absent: a binding whose item is gone, an
    /// item or a queue row whose object is not indexed.
    pub fn dangling(&self) -> Result<Vec<PimdirDangling>> {
        let mut dangling = Vec::new();

        let mut stmt = self.conn.prepare(
            "SELECT b.collection, b.link_id, b.source FROM bindings b \
             WHERE NOT EXISTS (SELECT 1 FROM items i \
                 WHERE i.collection = b.collection AND i.link_id = b.link_id) \
             ORDER BY b.collection, b.link_id, b.source",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (collection, link, source) = row?;
            dangling.push(PimdirDangling {
                kind: "binding",
                row: format!("{collection}/{link} @{source}"),
                target: format!("item {collection}/{link}"),
            });
        }

        let mut stmt = self.conn.prepare(
            "SELECT collection, link_id, object_hash FROM items \
             WHERE object_hash IS NOT NULL \
               AND object_hash NOT IN (SELECT hash FROM objects) \
             ORDER BY collection, link_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (collection, link, hash) = row?;
            dangling.push(PimdirDangling {
                kind: "item-object",
                row: format!("{collection}/{link}"),
                target: format!("object {hash}"),
            });
        }

        let mut stmt = self.conn.prepare(
            "SELECT id, collection, object_hash FROM queue \
             WHERE object_hash IS NOT NULL \
               AND object_hash NOT IN (SELECT hash FROM objects) ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (id, collection, hash) = row?;
            dangling.push(PimdirDangling {
                kind: "queue-object",
                row: format!("queue {id} ({collection})"),
                target: format!("object {hash}"),
            });
        }

        Ok(dangling)
    }
}
