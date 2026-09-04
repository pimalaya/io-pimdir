//! # Canonical SQL
//!
//! The pimdir schema and statements, generated at build time from the
//! specification vendored under spec/ (migrations/storage/ and
//! queries/storage/, one file per statement under read/, queue/ and
//! owner/), so the crate is self-contained and every canonical statement
//! is reachable by name for an implementation holding its own SQLite
//! binding.
//!
//! The statements of this crate's own are the operator tool's, behind
//! `pimdir check` and `store info`: nothing on a profile's path, so an
//! implementation of the standard needs none of them.

include!(concat!(env!("OUT_DIR"), "/canonical.rs"));

/// Declares the crate's own statements and the [`OWN`] index in one
/// expansion, so a statement is added in one place.
macro_rules! statements {
    ($($(#[$doc:meta])* $name:ident = $sql:expr;)*) => {
        $($(#[$doc])* pub const $name: &str = $sql;)*

        /// Every statement of this crate's own, paired with its constant name.
        pub const OWN: &[(&str, &str)] = &[$((stringify!($name), $name)),*];
    };
}

/// Every statement, canonical then this crate's own, paired with its name.
pub fn all() -> impl Iterator<Item = (&'static str, &'static str)> {
    CANONICAL.iter().chain(OWN).copied()
}

statements! {
/// How many objects are indexed and what they weigh.
OBJECT_STATS = r#"SELECT count(*), coalesce(sum(size), 0) FROM objects;"#;

/// The bytes held by objects at least one live item binds.
LIVE_BYTES = r#"SELECT coalesce(sum(size), 0) FROM objects WHERE hash IN
(SELECT object_hash FROM items WHERE object_hash IS NOT NULL AND retained_at IS NULL);"#;

/// One object's stored size.
OBJECT_SIZE = r#"SELECT size FROM objects WHERE hash = :hash;"#;

/// What a purge with this cutoff would retire, and what its bodies weigh.
COUNT_RETAINED_BEFORE = r#"SELECT count(*), coalesce(sum(o.size), 0) FROM items i
LEFT JOIN objects o ON o.hash = i.object_hash
WHERE i.retained_at IS NOT NULL AND i.retained_at < :cutoff;"#;

/// The objects whose stored refcount disagrees with the five pointer columns.
REFCOUNT_DRIFT = r#"WITH refs(hash) AS (
  SELECT object_hash FROM items WHERE object_hash IS NOT NULL
  UNION ALL SELECT conflict_object FROM items WHERE conflict_object IS NOT NULL
  UNION ALL SELECT base_object FROM bindings WHERE base_object IS NOT NULL
  UNION ALL SELECT conflict_object FROM bindings WHERE conflict_object IS NOT NULL
  UNION ALL SELECT object_hash FROM queue WHERE object_hash IS NOT NULL
), counted(hash, n) AS (SELECT hash, count(*) FROM refs GROUP BY hash)
SELECT o.hash, o.refcount, coalesce(c.n, 0) FROM objects o
LEFT JOIN counted c ON c.hash = o.hash
WHERE o.refcount != coalesce(c.n, 0) ORDER BY o.hash;"#;

/// How many minted keys (§9) each collection holds; `GLOB` since `LIKE`
/// is case-insensitive over ASCII.
MINTED_KEYS = r#"SELECT collection, count(*) FROM items
WHERE link_id GLOB 'dup:*' AND deleted = 0 AND retained_at IS NULL
GROUP BY collection ORDER BY collection;"#;

/// The bindings whose item is gone, the one dangling row a repair clears.
DANGLING_BINDINGS = r#"SELECT b.collection, b.link_id, b.source FROM bindings b
WHERE NOT EXISTS (SELECT 1 FROM items i
  WHERE i.collection = b.collection AND i.link_id = b.link_id)
ORDER BY b.collection, b.link_id, b.source;"#;

/// The items whose body is not indexed; reported, never repaired.
DANGLING_ITEM_OBJECTS = r#"SELECT collection, link_id, object_hash FROM items
WHERE object_hash IS NOT NULL AND object_hash NOT IN (SELECT hash FROM objects)
ORDER BY collection, link_id;"#;

/// The queue rows whose body is not indexed; reported, never repaired.
DANGLING_QUEUE_OBJECTS = r#"SELECT id, collection, object_hash FROM queue
WHERE object_hash IS NOT NULL AND object_hash NOT IN (SELECT hash FROM objects)
ORDER BY id;"#;

/// Deletes the bindings whose item is gone: unreachable rows, and the one
/// dangling row a repair can clear without guessing.
DELETE_DANGLING_BINDINGS = r#"DELETE FROM bindings WHERE NOT EXISTS (
  SELECT 1 FROM items i
  WHERE i.collection = bindings.collection AND i.link_id = bindings.link_id);"#;
}

#[cfg(test)]
mod tests {
    use super::all;

    #[test]
    fn no_statement_is_empty_and_every_name_is_unique() {
        let mut names = alloc::collections::BTreeSet::new();
        for (name, sql) in all() {
            assert!(!sql.trim().is_empty(), "{name} is empty");
            assert!(names.insert(name), "{name} is declared twice");
        }
    }
}
