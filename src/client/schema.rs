//! # Schema
//!
//! The open contract (STORAGE §6): a fresh store created, and every
//! existing one checked before any profile reads or writes it, refused
//! when this crate does not open it: another program's database, a store
//! of another draft, another algorithm, a damaged schema. Drafts are
//! exact (§4.2): nothing is reconciled, migrated nor rebuilt.

#[cfg(test)]
pub(crate) mod declared;

use alloc::string::String;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, named_params};

use crate::{
    client::{PimdirError, PreparedCached, busy_or_sql},
    hash::HASH_ALGO,
    sql,
    summary::DERIVATION,
    sync::SYNC_VERSION,
    version::{PimdirFormat, PimdirProfile, PimdirProgram, PimdirStoreVersion},
};

/// Opens a store as its owner: a fresh database is created, an existing
/// one goes through [`check`], so a store this crate does not open is
/// refused before anything is written, the journal mode included.
///
/// The creation is 0001_init.sql and `init_store_meta` in one transaction
/// (§6 step 3); while the format is a draft no migration runs on an
/// existing store.
pub(crate) fn init(conn: &mut Connection) -> Result<(), PimdirError> {
    if user_version(conn)? != 0 || holds_schema(conn)? {
        return check(conn, PimdirProfile::Owner);
    }

    // NOTE: a deferred transaction, the owner lock keeping every other
    // writer out: 0001_init.sql opens on `PRAGMA auto_vacuum`, which an
    // immediate one, holding the write lock already, leaves unset (§4.1).
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(busy_or_sql)?;
    tx.execute_batch(sql::MIGRATION_0001)?;
    tx.execute_cached(
        sql::INIT_STORE_META,
        named_params! { ":version": 1, ":derivation": DERIVATION },
    )?;
    tx.pragma_update(None, "user_version", 1)?;
    tx.commit().map_err(busy_or_sql)?;

    // NOTE: the journal mode is chosen once, at creation: a store its
    // operator switched to a rollback journal for a network filesystem
    // (§8) stays on it.
    conn.execute_batch("PRAGMA journal_mode = WAL;")?;
    Ok(())
}

/// Refuses, writing nothing, an existing database this crate does not
/// open as `profile` (STORAGE §6 steps 1 and 2): in order, a
/// database declaring tables at `user_version` 0 or naming another
/// `format` ([`PimdirError::Foreign`]), one holding no schema
/// ([`PimdirError::Uncreated`]), a store whose `format_version` is not
/// [`sql::FORMAT_VERSION`] or which lacks the column
/// ([`PimdirError::Version`], the judgement of
/// [`PimdirStoreVersion::opens`] for this crate's draft, which no profile
/// moves), disagreeing schema stamps
/// ([`PimdirError::VersionMismatch`]), an algorithm other than BLAKE3
/// ([`PimdirError::HashAlgo`]), then a schema whose text is not the
/// canonical one ([`PimdirError::Damaged`]).
///
/// A store at this version holding the canonical schema, every store this
/// crate opens, is judged by its `store_meta` row alone, read once: its
/// columns are then the canonical ones, so the steps asking whether they
/// exist answer yes, and the schema is not the damage, whatever else is.
pub(crate) fn check(conn: &Connection, profile: PimdirProfile) -> Result<(), PimdirError> {
    let user_version = user_version(conn)?;
    if user_version == sql::VERSION && canonical(conn)? {
        let row = conn
            .query_row_cached(sql::LOAD_STORE_META, [], |r| {
                Ok((r.get::<_, String>(0)?, meta(r)?))
            })
            .optional()?;
        return match row {
            Some((format, meta)) if format == "pimdir" => judge(profile, user_version, meta),
            _ => Err(PimdirError::Foreign),
        };
    }
    if user_version == 0 {
        return match holds_schema(conn)? {
            true => Err(PimdirError::Foreign),
            false => Err(PimdirError::Uncreated),
        };
    }

    let format: Option<String> = match has_column(conn, "store_meta", "format")? {
        true => conn
            .query_row_cached("SELECT format FROM store_meta WHERE id = 1", [], |r| {
                r.get(0)
            })
            .optional()?,
        false => None,
    };
    if format.as_deref() != Some("pimdir") {
        return Err(PimdirError::Foreign);
    }

    // NOTE: a store a draft before storage-draft-08 wrote lacks the
    // marker, so `load_store_meta` would fail on it: the column is looked
    // for first.
    if !has_column(conn, "store_meta", "format_version")? {
        return Err(PimdirError::Version { found: None });
    }
    let meta = conn.query_row_cached(sql::LOAD_STORE_META, [], meta)?;
    judge(profile, user_version, meta)?;

    // NOTE: a draft's store holds version 1 alone (§4.2), and its schema
    // is 0001_init.sql's to the word: anything else is damage.
    if user_version != sql::VERSION {
        return Err(PimdirError::Damaged {
            object: String::from("user_version"),
        });
    }
    match differs(conn)? {
        Some(object) => Err(PimdirError::Damaged { object }),
        None => Ok(()),
    }
}

/// The columns of `load_store_meta` a `pimdir` store is judged by: its
/// `version`, `hash_algo`, `format_version`, `read_version` and
/// `produce_version`.
type Meta = (i64, String, String, i64, i64);

/// Reads [`Meta`] off a `load_store_meta` row.
fn meta(r: &rusqlite::Row<'_>) -> rusqlite::Result<Meta> {
    Ok((r.get(1)?, r.get(2)?, r.get(8)?, r.get(9)?, r.get(10)?))
}

/// Refuses a `pimdir` store by its `store_meta` row (STORAGE §6 step 2),
/// in order: a `format_version` this crate's draft does not open,
/// disagreeing schema stamps, another algorithm. A draft is refused for
/// its marker alone (§4.2): the six columns a release judges by have no
/// error of their own until one is built, and the checks after the first
/// are a draft opener's own.
fn judge(profile: PimdirProfile, user_version: i64, meta: Meta) -> Result<(), PimdirError> {
    let (version, hash_algo, format_version, read_version, produce_version) = meta;
    let program = PimdirProgram {
        profile,
        version: sql::VERSION,
        format: PimdirFormat::Draft(sql::FORMAT_VERSION),
        hash_algos: &[HASH_ALGO],
    };
    let store = PimdirStoreVersion {
        format: Some("pimdir"),
        format_version: Some(&format_version),
        version,
        user_version,
        read_version,
        produce_version,
        hash_algo: &hash_algo,
    };
    if store.opens(&program).is_err() {
        return Err(PimdirError::Version {
            found: Some(format_version),
        });
    }
    if version != user_version {
        return Err(PimdirError::VersionMismatch {
            user_version,
            store_meta: version,
        });
    }
    if hash_algo != HASH_ALGO {
        return Err(PimdirError::HashAlgo { found: hash_algo });
    }
    Ok(())
}

/// The first object, by name, whose text is not the canonical one: a
/// canonical one missing or recorded otherwise, else one no migration
/// declares; `None` when the schema is the canonical one. The objects are
/// read in name order beside [`sql::DECLARED`], their text borrowed.
fn differs(conn: &Connection) -> Result<Option<String>, PimdirError> {
    if canonical(conn)? {
        return Ok(None);
    }
    let mut statement = conn.prepare_cached(
        "SELECT name, sql FROM sqlite_schema \
         WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let mut rows = statement.query([])?;
    let mut declared = sql::DECLARED.iter().peekable();
    let mut extra: Option<String> = None;
    while let Some(row) = rows.next()? {
        let name = row.get_ref(0)?.as_str().map_err(rusqlite::Error::from)?;
        let text = row.get_ref(1)?.as_str().map_err(rusqlite::Error::from)?;
        if let Some((known, _)) = declared.peek()
            && *known < name
        {
            return Ok(Some(String::from(*known)));
        }
        match declared.peek() {
            Some((known, normalised)) if *known == name => {
                if !same(text, normalised) {
                    return Ok(Some(String::from(name)));
                }
                declared.next();
            }
            _ => {
                extra.get_or_insert_with(|| String::from(name));
            }
        }
    }
    Ok(declared
        .next()
        .map(|(known, _)| String::from(*known))
        .or(extra))
}

/// Whether the schema is the canonical one, read in `sqlite_schema`'s own
/// order with each name sought in [`sql::DECLARED`], no row sorted: every
/// object declared and with its text, none other. What [`differs`] asks
/// first, walking the names in order only to name a damaged object.
fn canonical(conn: &Connection) -> Result<bool, PimdirError> {
    let mut statement = conn.prepare_cached(
        "SELECT name, sql FROM sqlite_schema \
         WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'",
    )?;
    let mut rows = statement.query([])?;
    let mut held = 0;
    while let Some(row) = rows.next()? {
        let name = row.get_ref(0)?.as_str().map_err(rusqlite::Error::from)?;
        let Ok(at) = sql::DECLARED.binary_search_by(|(known, _)| (*known).cmp(name)) else {
            return Ok(false);
        };
        let text = row.get_ref(1)?.as_str().map_err(rusqlite::Error::from)?;
        if !same(text, sql::DECLARED[at].1) {
            return Ok(false);
        }
        held += 1;
    }
    Ok(held == sql::DECLARED.len())
}

/// Whether the text SQLite records for a statement, every `--` comment
/// dropped and whitespace collapsed to single spaces, is `normalised`:
/// compared byte by byte in one pass, nothing allocated.
///
/// Whitespace is ASCII's (a Unicode space is a byte like any other, so a
/// text holding one where the migrations hold none is not the canonical
/// one), a comment runs from `--` to the end of its line, and a line
/// break separates as whitespace does.
pub(crate) fn same(sql: &str, normalised: &str) -> bool {
    let want = normalised.as_bytes();
    let bytes = sql.as_bytes();
    let (mut at, mut i) = (0, 0);
    let mut space = false;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' | b' ' | b'\t' | b'\r' | 0x0B | 0x0C => {
                space = at > 0;
                i += 1;
            }
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                space = at > 0;
                i = bytes[i..]
                    .iter()
                    .position(|b| *b == b'\n')
                    .map_or(bytes.len(), |end| i + end);
            }
            byte => {
                if space {
                    if want.get(at) != Some(&b' ') {
                        return false;
                    }
                    at += 1;
                    space = false;
                }
                if want.get(at) != Some(&byte) {
                    return false;
                }
                at += 1;
                i += 1;
            }
        }
    }
    at == want.len()
}

/// [`same`] as it read before, word by word, which the byte comparison
/// is held to.
#[cfg(test)]
fn same_by_words(sql: &str, normalised: &str) -> bool {
    let mut words = normalised.split(' ').filter(|word| !word.is_empty());
    sql.lines()
        .flat_map(|line| {
            line.split_once("--")
                .map_or(line, |(code, _)| code)
                .split_whitespace()
        })
        .all(|word| words.next() == Some(word))
        && words.next().is_none()
}

#[cfg(test)]
mod same_tests {
    use alloc::format;

    use crate::client::schema::{same, same_by_words};

    /// The byte comparison answers the word comparison over every object
    /// the migrations declare, as recorded and perturbed.
    #[test]
    fn the_byte_comparison_answers_the_word_comparison() {
        let texts = crate::client::schema::declared::declared(&[crate::sql::MIGRATION_0001]);
        assert!(!texts.is_empty());
        for normalised in texts.values() {
            let spaced = normalised.replace(' ', " \n\t ");
            let commented = normalised.replacen(' ', " -- a note\n ", 1);
            let other = normalised.replacen(' ', "  x ", 1);
            let short = &normalised[..normalised.len() - 1];
            let long = format!("{normalised} x");
            let crlf = normalised.replace(' ', "\r\n");
            let lead = format!("\n  {normalised}\n");
            let candidates: [&str; 8] = [
                normalised, &spaced, &commented, &other, short, &long, &crlf, &lead,
            ];
            for text in candidates {
                assert_eq!(
                    same(text, normalised),
                    same_by_words(text, normalised),
                    "{text:?}"
                );
            }
            assert!(same(&spaced, normalised));
            assert!(!same(&other, normalised));
        }
    }
}

/// Records this crate's Annex A revision in the caller's transaction, the
/// one of a summary, address or sort-key write, when the store stands
/// under a later one (STORAGE §6): the rows are then this revision's, and
/// the next owner at a later one re-derives them.
///
/// A store at an earlier revision keeps its own until its re-derivation
/// lands.
pub(crate) fn record_derivation(conn: &Connection) -> rusqlite::Result<()> {
    let stored: i64 = conn.query_row_cached(sql::LOAD_STORE_META, [], |r| r.get(7))?;
    if stored > DERIVATION {
        conn.execute_cached(
            sql::SET_DERIVATION,
            named_params! { ":derivation": DERIVATION },
        )?;
    }
    Ok(())
}

/// The SYNC version whose engine wrote the store's sync state, `None`
/// before any engine wrote (`load_store_meta`), refusing one naming
/// another than this crate's [`SYNC_VERSION`] (STORAGE §4.3, SYNC §10).
pub(crate) fn sync_version(conn: &Connection) -> Result<Option<String>, PimdirError> {
    let found: Option<String> = conn.query_row_cached(sql::LOAD_STORE_META, [], |r| r.get(13))?;
    match found {
        Some(found) if found != SYNC_VERSION => Err(PimdirError::SyncVersion { found }),
        found => Ok(found),
    }
}

/// The store's `PRAGMA user_version`, `0` for a fresh database.
fn user_version(conn: &Connection) -> Result<i64, PimdirError> {
    Ok(conn.pragma_query_value(None, "user_version", |r| r.get(0))?)
}

/// Whether a database declares anything: a pimdir store is created with
/// its version in one transaction, so one declaring tables at
/// `user_version` 0 is another program's and is never written (STORAGE
/// §3).
fn holds_schema(conn: &Connection) -> Result<bool, PimdirError> {
    Ok(conn
        .query_row_cached(
            "SELECT 1 FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' LIMIT 1",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Whether `table` has `column`, `false` for a table the database lacks.
fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool, PimdirError> {
    Ok(conn
        .query_row_cached(
            "SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2",
            [table, column],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

#[cfg(test)]
mod tests {
    use alloc::{string::String, vec::Vec};

    use std::collections::BTreeMap;

    use rusqlite::Connection;

    use crate::{
        client::{PimdirError, rows},
        sql,
    };

    /// What the statements of a migration declare is what SQLite records
    /// once it runs them, for the store's and the index's.
    #[test]
    fn the_declared_schema_is_the_one_sqlite_records() {
        #[allow(unused_mut)]
        let mut sets = alloc::vec![sql::MIGRATIONS];
        #[cfg(any(feature = "search", feature = "indexer"))]
        sets.push(sql::search::MIGRATIONS);
        for migrations in sets {
            let conn = rusqlite::Connection::open_in_memory().unwrap();
            for migration in migrations {
                conn.execute_batch(migration).unwrap();
            }
            let recorded: BTreeMap<_, _> = objects(&conn)
                .unwrap()
                .into_iter()
                .map(|(name, sql)| (name, super::declared::normalised(&sql)))
                .collect();
            let virtuals: Vec<String> = recorded
                .iter()
                .filter(|(_, sql)| sql.starts_with("CREATE VIRTUAL TABLE"))
                .map(|(name, _)| alloc::format!("{name}_"))
                .collect();
            let recorded: BTreeMap<_, _> = recorded
                .into_iter()
                .filter(|(name, _)| !virtuals.iter().any(|prefix| name.starts_with(prefix)))
                .collect();
            assert_eq!(super::declared::declared(migrations), recorded);
        }
    }

    /// The constant the build script derives is what the migrations
    /// declare, and each one's recorded text compares equal to it.
    #[test]
    fn the_built_schema_is_the_declared_one() {
        let built = |declared: &[(&str, &str)]| -> BTreeMap<_, _> {
            declared
                .iter()
                .map(|(name, text)| (String::from(*name), String::from(*text)))
                .collect()
        };
        #[cfg(any(feature = "search", feature = "indexer"))]
        assert_eq!(
            super::declared::declared(sql::search::MIGRATIONS),
            built(sql::search::DECLARED)
        );
        let built = built(sql::DECLARED);
        assert_eq!(super::declared::declared(sql::MIGRATIONS), built);
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(sql::MIGRATION_0001).unwrap();
        for (name, raw) in objects(&conn).unwrap() {
            if let Some(text) = built.get(&name) {
                assert!(super::same(&raw, text), "{name}");
                assert!(!super::same(&raw, &alloc::format!("{text} x")), "{name}");
            }
        }
    }

    /// The marker the refusal compares is the one `init_store_meta`
    /// writes, so a re-vendored draft moves both or fails here.
    #[test]
    fn the_marker_is_the_one_init_store_meta_writes() {
        let marker = alloc::format!("'{}'", sql::FORMAT_VERSION);
        assert!(sql::INIT_STORE_META.contains(&marker));
    }

    /// Every table, index and trigger a database declares, by name, the text
    /// SQLite records for it; the automatic indexes, which declare no
    /// statement, and SQLite's own tables left out.
    fn objects(conn: &Connection) -> Result<BTreeMap<String, String>, PimdirError> {
        let declared: Vec<(String, String)> = rows(
            conn,
            "SELECT name, sql FROM sqlite_schema \
             WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(declared.into_iter().collect())
    }
}
