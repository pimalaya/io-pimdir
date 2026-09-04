//! # Schema
//!
//! Creating the schema in a fresh database (STORAGE §6) and refusing a
//! store this crate does not read: another version, disagreeing stamps,
//! an earlier draft's shape, a foreign hash.

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

/// Creates the schema in a fresh database, stamping `user_version` and
/// `store_meta.version` in agreement (§4.2), or checks an existing one.
pub(crate) fn init(conn: &mut Connection, hash: PimdirHashAlgo) -> Result<(), PimdirError> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > sql::VERSION {
        return Err(PimdirError::Version { found: version });
    }
    if version == sql::VERSION {
        return check(conn);
    }

    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(busy_or_sql)?;
    tx.execute_batch(sql::MIGRATION_0001)?;
    tx.execute(
        sql::INIT_STORE_META,
        named_params! { ":version": sql::VERSION, ":hash_algo": hash.as_str() },
    )?;
    tx.pragma_update(None, "user_version", sql::VERSION)?;
    tx.commit().map_err(busy_or_sql)?;

    Ok(())
}

/// Refuses a store stamped at the current version that this crate does
/// not read: disagreeing stamps (§4.2), or an earlier draft's shape.
///
/// The spec is a draft with no migration path, so a store missing a
/// table is recreated by its owner, never reconciled here.
pub(crate) fn check(conn: &Connection) -> Result<(), PimdirError> {
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
