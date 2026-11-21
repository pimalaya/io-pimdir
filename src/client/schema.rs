//! # Schema
//!
//! The open contract (STORAGE §6): a fresh store created, and every
//! existing one checked before any profile reads or writes it, refused
//! when this crate does not open it: another program's database, a store
//! of another draft, another algorithm, a damaged schema. Drafts are
//! exact (§4.2): nothing is reconciled, migrated nor rebuilt.

use alloc::{string::String, vec::Vec};

use std::{collections::BTreeMap, sync::LazyLock};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, named_params};

use crate::{
    client::{PimdirError, PreparedCached, busy_or_sql, rows},
    hash::HASH_ALGO,
    sql,
    sync::SYNC_VERSION,
    version::{PimdirFormat, PimdirProfile, PimdirProgram, PimdirStoreVersion},
};

/// The Annex A revision this crate derives summaries, addresses and sort
/// keys under (STORAGE §6), recorded in `store_meta.derivation`.
pub(crate) const DERIVATION: i64 = 3;

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
pub(crate) fn check(conn: &Connection, profile: PimdirProfile) -> Result<(), PimdirError> {
    let user_version = user_version(conn)?;
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
    type Meta = (i64, String, String, i64, i64);
    let (version, hash_algo, format_version, read_version, produce_version): Meta = conn
        .query_row_cached(sql::LOAD_STORE_META, [], |r| {
            Ok((r.get(1)?, r.get(2)?, r.get(8)?, r.get(9)?, r.get(10)?))
        })?;
    let program = PimdirProgram {
        profile,
        version: sql::VERSION,
        format: PimdirFormat::Draft(sql::FORMAT_VERSION),
        hash_algos: &[HASH_ALGO],
    };
    let store = PimdirStoreVersion {
        format: format.as_deref(),
        format_version: Some(&format_version),
        version,
        user_version,
        read_version,
        produce_version,
        hash_algo: &hash_algo,
    };
    // NOTE: a draft is refused for its marker alone (§4.2): the six
    // columns a release judges by have no error of their own until one is
    // built, and the checks below are a draft opener's own.
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

    // NOTE: a draft's store holds version 1 alone (§4.2), and its schema
    // is 0001_init.sql's to the word: anything else is damage.
    if user_version != sql::VERSION {
        return Err(PimdirError::Damaged {
            object: String::from("user_version"),
        });
    }
    let held = objects(conn)?;
    let differs = CANONICAL
        .iter()
        .find(|(name, text)| held.get(*name) != Some(*text))
        .map(|(name, _)| name)
        .or_else(|| held.keys().find(|name| !CANONICAL.contains_key(*name)));
    match differs {
        Some(object) => Err(PimdirError::Damaged {
            object: object.clone(),
        }),
        None => Ok(()),
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

/// A declaring statement as the schema check compares it: every `--`
/// comment dropped and whitespace collapsed to single spaces.
pub(crate) fn normalised(sql: &str) -> String {
    let mut out = String::new();
    for line in sql.lines() {
        let code = line.split_once("--").map_or(line, |(code, _)| code);
        for word in code.split_whitespace() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(word);
        }
    }
    out
}

/// Every table, index and trigger the canonical migrations declare, by
/// name, its text [`normalised`].
static CANONICAL: LazyLock<BTreeMap<String, String>> = LazyLock::new(|| declared(sql::MIGRATIONS));

/// Every table, index, trigger and view `migrations` create, by name, the
/// text SQLite records for each [`normalised`]: read from the statements
/// themselves, so no database is built to learn them.
///
/// A statement runs from its first word to its `;`, a trigger's from
/// `CREATE` to the `;` after the `END` closing its `BEGIN`, a `CASE` in
/// its body nesting; comments, strings and quoted names hold no
/// boundary. SQLite records that text from `CREATE` on, which
/// [`normalised`] reads alike.
pub(crate) fn declared(migrations: &[&str]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for migration in migrations {
        for statement in statements(migration) {
            if let Some(name) = created(statement) {
                out.insert(name, normalised(statement));
            }
        }
    }
    out
}

/// The statements of a migration, each from its first word to before its
/// `;`.
fn statements(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    let mut start: Option<usize> = None;
    let mut words: Vec<String> = Vec::new();
    let mut depth = 0usize;
    while at < bytes.len() {
        let byte = bytes[at];
        match byte {
            b'-' if bytes.get(at + 1) == Some(&b'-') => {
                while at < bytes.len() && bytes[at] != b'\n' {
                    at += 1;
                }
                continue;
            }
            b'/' if bytes.get(at + 1) == Some(&b'*') => {
                at += 2;
                while at + 1 < bytes.len() && !(bytes[at] == b'*' && bytes[at + 1] == b'/') {
                    at += 1;
                }
                at += 2;
                continue;
            }
            b'\'' | b'"' | b'`' | b'[' => {
                start.get_or_insert(at);
                let close = if byte == b'[' { b']' } else { byte };
                at += 1;
                while at < bytes.len() {
                    if bytes[at] == close {
                        if close != b']' && bytes.get(at + 1) == Some(&close) {
                            at += 2;
                            continue;
                        }
                        break;
                    }
                    at += 1;
                }
                at += 1;
                continue;
            }
            b';' => {
                let trigger = words.len() >= 2 && words[1] == "TRIGGER"
                    || words.len() >= 3 && words[2] == "TRIGGER";
                if !trigger || (depth == 0 && words.iter().any(|word| word == "BEGIN")) {
                    if let Some(from) = start.take() {
                        out.push(&text[from..at]);
                    }
                    words.clear();
                    depth = 0;
                }
                at += 1;
                continue;
            }
            byte if byte.is_ascii_alphanumeric() || byte == b'_' => {
                let from = at;
                while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
                    at += 1;
                }
                start.get_or_insert(from);
                let word = text[from..at].to_ascii_uppercase();
                match word.as_str() {
                    "BEGIN" | "CASE" => depth += 1,
                    "END" => depth = depth.saturating_sub(1),
                    _ => (),
                }
                if words.len() < 3 || word == "BEGIN" {
                    words.push(word);
                }
                continue;
            }
            byte if byte.is_ascii_whitespace() => (),
            _ => {
                start.get_or_insert(at);
            }
        }
        at += 1;
    }
    out
}

/// The name a `CREATE` statement gives its table, index, trigger or view,
/// `None` for another statement.
fn created(statement: &str) -> Option<String> {
    let mut words = statement.split_whitespace();
    if !words.next()?.eq_ignore_ascii_case("CREATE") {
        return None;
    }
    let mut word = words.next()?;
    for skipped in ["UNIQUE", "TEMP", "TEMPORARY", "VIRTUAL"] {
        if word.eq_ignore_ascii_case(skipped) {
            word = words.next()?;
        }
    }
    if !["TABLE", "INDEX", "TRIGGER", "VIEW"]
        .iter()
        .any(|kind| word.eq_ignore_ascii_case(kind))
    {
        return None;
    }
    let mut name = words.next()?;
    if name.eq_ignore_ascii_case("IF") {
        words.next()?;
        words.next()?;
        name = words.next()?;
    }
    let name = name.split('(').next()?;
    let name = name.rsplit('.').next()?;
    Some(name.trim_matches(|c| c == '"' || c == '`').into())
}

/// Every table, index and trigger a database declares, by name, its text
/// [`normalised`]; the automatic indexes, which declare no statement, and
/// SQLite's own tables left out.
fn objects(conn: &Connection) -> Result<BTreeMap<String, String>, PimdirError> {
    let declared: Vec<(String, String)> = rows(
        conn,
        "SELECT name, sql FROM sqlite_schema \
         WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(declared
        .into_iter()
        .map(|(name, sql)| (name, normalised(&sql)))
        .collect())
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
    use crate::sql;

    /// The marker the refusal compares is the one `init_store_meta`
    /// writes, so a re-vendored draft moves both or fails here.
    /// What the statements of a migration declare is what SQLite records
    /// once it runs them, for the store's and the index's.
    #[test]
    fn the_declared_schema_is_the_one_sqlite_records() {
        for migrations in [sql::MIGRATIONS, sql::search::MIGRATIONS] {
            let conn = rusqlite::Connection::open_in_memory().unwrap();
            for migration in migrations {
                conn.execute_batch(migration).unwrap();
            }
            let recorded = super::objects(&conn).unwrap();
            let virtuals: alloc::vec::Vec<alloc::string::String> = recorded
                .iter()
                .filter(|(_, sql)| sql.starts_with("CREATE VIRTUAL TABLE"))
                .map(|(name, _)| alloc::format!("{name}_"))
                .collect();
            let recorded: super::BTreeMap<_, _> = recorded
                .into_iter()
                .filter(|(name, _)| !virtuals.iter().any(|prefix| name.starts_with(prefix)))
                .collect();
            assert_eq!(super::declared(migrations), recorded);
        }
    }

    #[test]
    fn the_marker_is_the_one_init_store_meta_writes() {
        let marker = alloc::format!("'{}'", sql::FORMAT_VERSION);
        assert!(sql::INIT_STORE_META.contains(&marker));
    }
}
