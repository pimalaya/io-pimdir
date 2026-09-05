//! # Schema
//!
//! The migration runner (STORAGE §6) and the checks refusing a store this
//! crate does not read: a newer version, disagreeing stamps, an earlier
//! draft's shape, a foreign hash.

use alloc::{string::String, vec::Vec};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, named_params};

use crate::{
    client::{PimdirError, busy_or_sql},
    hash::PimdirHashAlgo,
    sql,
};

/// The tables the canonical schema declares, which a store at the current
/// version has to hold whole: the draft is edited in place (§6) and the
/// version stamp alone cannot tell an earlier draft's store apart.
const TABLES: [&str; 14] = [
    "bindings",
    "collections",
    "contact_summary",
    "event_summary",
    "item_address",
    "items",
    "journal_summary",
    "mail_summary",
    "objects",
    "probes",
    "queue",
    "sources",
    "store_meta",
    "task_summary",
];

/// Runs every migration above `user_version` in order, each in its own
/// transaction setting the version it reaches (§6), the first one
/// seeding `store_meta` (§4.2); then checks the store as [`check`] does.
/// A store above the current version is refused.
pub(crate) fn init(conn: &mut Connection, hash: PimdirHashAlgo) -> Result<(), PimdirError> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > sql::VERSION {
        return Err(PimdirError::Version { found: version });
    }

    for (index, migration) in sql::MIGRATIONS.iter().enumerate() {
        let reached = index as i64 + 1;
        if reached <= version {
            continue;
        }
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        tx.execute_batch(migration)?;
        if reached == 1 {
            tx.execute(
                sql::INIT_STORE_META,
                named_params! { ":version": reached, ":hash_algo": hash.as_str() },
            )?;
        } else {
            tx.execute(
                "UPDATE store_meta SET version = :version WHERE id = 1",
                named_params! { ":version": reached },
            )?;
        }
        tx.pragma_update(None, "user_version", reached)?;
        tx.commit().map_err(busy_or_sql)?;
    }

    check(conn)
}

/// Refuses a store stamped at the current version that this crate does
/// not read: an earlier draft's shape, or disagreeing stamps (§4.2).
///
/// The shape is checked first, so a store lacking `store_meta` is named
/// as stale rather than failing the read of its stamp. The spec is a
/// draft with no migration path, so a store missing a table is recreated
/// by its owner, never reconciled here.
pub(crate) fn check(conn: &Connection) -> Result<(), PimdirError> {
    let mut stmt = conn.prepare("SELECT name FROM sqlite_schema WHERE type = 'table'")?;
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if let Some(table) = TABLES
        .iter()
        .find(|table| !tables.iter().any(|t| t == *table))
    {
        return Err(PimdirError::Stale { table });
    }

    let stamped: Option<i64> = conn
        .query_row("SELECT version FROM store_meta WHERE id = 1", [], |row| {
            row.get(0)
        })
        .optional()?;
    if let Some(store_meta) = stamped
        && store_meta != sql::VERSION
    {
        return Err(PimdirError::VersionMismatch {
            user_version: sql::VERSION,
            store_meta,
        });
    }

    Ok(())
}

/// Opens an existing database read-only or as a producer, refusing what
/// [`check`] refuses plus an unstamped or foreign version.
pub(crate) fn check_version(conn: &Connection) -> Result<(), PimdirError> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    match version {
        version if version == sql::VERSION => check(conn),
        0 => Err(PimdirError::Uncreated),
        found => Err(PimdirError::Version { found }),
    }
}

/// The algorithm the store records, checked against the one declared: a
/// handle computing another would name bodies no reader finds (§5).
pub(crate) fn hash_algo(
    conn: &Connection,
    declared: Option<PimdirHashAlgo>,
) -> Result<PimdirHashAlgo, PimdirError> {
    let stored: Option<String> = conn
        .query_row("SELECT hash_algo FROM store_meta WHERE id = 1", [], |row| {
            row.get(0)
        })
        .optional()?;

    let Some(stored) = stored else {
        return Ok(declared.unwrap_or_default());
    };
    let Some(algo) = PimdirHashAlgo::parse(&stored) else {
        return Err(PimdirError::HashAlgo {
            found: stored,
            declared: declared.map(|a| a.as_str()),
        });
    };
    match declared {
        Some(declared) if declared != algo => Err(PimdirError::HashAlgo {
            found: stored,
            declared: Some(declared.as_str()),
        }),
        _ => Ok(algo),
    }
}
