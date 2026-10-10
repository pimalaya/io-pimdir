//! # Schema
//!
//! The migration runner (STORAGE §6) and the checks refusing a store this
//! crate does not read: a newer version, disagreeing stamps, an earlier
//! draft's shape, a foreign hash.

use alloc::{format, string::String, vec::Vec};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, named_params};

use crate::{
    client::{PimdirError, busy_or_sql},
    hash::PimdirHashAlgo,
    sql,
};

/// The tables and triggers the canonical schema declares, which a store
/// at the current version has to hold whole: the draft is edited in place
/// (§6) and the version stamp alone cannot tell an earlier draft's store
/// apart. The triggers are named since the last drafts moved the feed
/// into them, and a store lacking one stamps nothing.
const SCHEMA: [&str; 16] = [
    "bindings",
    "collections",
    "collections_restamp_items",
    "contact_summary",
    "event_summary",
    "item_address",
    "items",
    "items_stamp_request",
    "journal_summary",
    "mail_summary",
    "objects",
    "objects_count_collect",
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

    reconcile(conn)?;
    check(conn)
}

/// The tables a later draft added to version 1 that a store from an earlier
/// one can take as they are (§6): the owner creates them on open, from the
/// canonical DDL, instead of refusing the store. Readers and producers
/// read their absence as nothing declared, no receipt kept and no
/// reference recorded, no file summarised.
const RECONCILED: [&str; 5] = [
    "capabilities",
    "performers",
    "receipts",
    "item_reference",
    "file_summary",
];

/// The indexes and triggers a later draft added to version 1 (§6), each
/// `(kind, name)`, created on open when absent from the canonical DDL,
/// after the [`RECONCILED`] tables they hang off. A reader of a store
/// lacking one reads as before: `items_by_sort_global` orders a page
/// across collections (§9.3), slower without it; `item_reference_to`
/// and `items_drop_references` serve and hold the references (§14.2);
/// `item_reference_collects_files` collects the stand-ins (§14.3).
const RECONCILED_OBJECTS: [(&str, &str); 4] = [
    ("INDEX", "items_by_sort_global"),
    ("INDEX", "item_reference_to"),
    ("TRIGGER", "items_drop_references"),
    ("TRIGGER", "item_reference_collects_files"),
];

/// Creates the [`RECONCILED`] tables a store lacks, each with its key, and
/// adds `collections.role` with its index and triggers ([`reconcile_role`]),
/// the coverage and round columns ([`reconcile_rounds`]) and the
/// [`RECONCILED_OBJECTS`] ([`reconcile_objects`]), in one transaction.
fn reconcile(conn: &mut Connection) -> Result<(), PimdirError> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(busy_or_sql)?;
    for table in RECONCILED {
        let exists = tx
            .query_row(
                "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1",
                [table],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            tx.execute_batch(&ddl(table))?;
        }
    }
    reconcile_role(&tx)?;
    reconcile_rounds(&tx)?;
    reconcile_objects(&tx)?;
    tx.commit().map_err(busy_or_sql)
}

/// The role a source states (§14), added to version 1 by a later draft: the
/// column with its `CHECK`, the index keeping one holder, the trigger moving
/// a role, and the stamp trigger watching it, recreated when its body
/// predates the column (§6). All cut out of the canonical DDL.
fn reconcile_role(conn: &Connection) -> Result<(), PimdirError> {
    let schema = sql::MIGRATION_0001;
    let cut = |start: &str, end: &str| {
        let from = schema.find(start).expect("canonical DDL");
        let to = from + schema[from..].find(end).expect("canonical DDL") + end.len();
        &schema[from..to]
    };

    if !has_column(conn, "collections", "role")? {
        let column = cut("role        TEXT CHECK", "'default'))");
        conn.execute_batch(&format!("ALTER TABLE collections ADD COLUMN {column};"))?;
    }

    let declared = |name: &str| -> Result<Option<String>, PimdirError> {
        Ok(conn
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .optional()?)
    };
    if declared("collections_by_role")?.is_none() {
        conn.execute_batch(cut("CREATE UNIQUE INDEX collections_by_role", ";"))?;
    }
    if declared("collections_role_moves")?.is_none() {
        conn.execute_batch(cut("CREATE TRIGGER collections_role_moves", "END;"))?;
    }
    if declared("collections_stamp_update")?.is_some_and(|sql| !sql.contains("role")) {
        conn.execute_batch("DROP TRIGGER collections_stamp_update;")?;
        conn.execute_batch(cut("CREATE TRIGGER collections_stamp_update", "END;"))?;
    }

    Ok(())
}

/// The coverage and round a later draft added to version 1 (STORAGE §6,
/// SYNC §5): the columns of `sources` and `bindings.round`, each cut out
/// of the canonical DDL, the trigger restamping a collection whose
/// coverage moved, and the `probes` table the draft no longer has
/// dropped, its handles named again by the next round, a store from an
/// earlier draft holding no coverage.
fn reconcile_rounds(conn: &Connection) -> Result<(), PimdirError> {
    let schema = sql::MIGRATION_0001;
    let column = |table: &str, name: &str| {
        let body = &schema[schema
            .find(&format!("CREATE TABLE {table} ("))
            .expect("canonical DDL")..];
        let from = body.find(&format!("\n    {name} ")).expect("canonical DDL") + 5;
        let to = from + body[from..].find(',').expect("canonical DDL");
        String::from(&body[from..to])
    };

    // NOTE: in the canonical order, so a check naming an earlier column
    // finds it added already.
    for name in [
        "covered_since",
        "covered_until",
        "covered_at",
        "round",
        "round_since",
        "round_until",
        "round_cursor",
        "round_checkpoint",
        "round_started_at",
        "round_band",
    ] {
        if !has_column(conn, "sources", name)? {
            conn.execute_batch(&format!(
                "ALTER TABLE sources ADD COLUMN {};",
                column("sources", name)
            ))?;
        }
    }
    if !has_column(conn, "bindings", "round")? {
        conn.execute_batch(&format!(
            "ALTER TABLE bindings ADD COLUMN {};",
            column("bindings", "round")
        ))?;
    }

    let declared = conn
        .query_row(
            "SELECT 1 FROM sqlite_schema WHERE name = 'sources_stamp_coverage'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !declared {
        let from = schema
            .find("CREATE TRIGGER sources_stamp_coverage")
            .expect("canonical DDL");
        let to = from + schema[from..].find("END;").expect("canonical DDL") + 4;
        conn.execute_batch(&schema[from..to])?;
    }

    conn.execute_batch("DROP TABLE IF EXISTS probes;")?;
    Ok(())
}

/// Creates the [`RECONCILED_OBJECTS`] a store lacks, each cut out of the
/// canonical DDL: an index up to its `;`, a trigger up to its `END;`.
fn reconcile_objects(conn: &Connection) -> Result<(), PimdirError> {
    let schema = sql::MIGRATION_0001;
    for (kind, name) in RECONCILED_OBJECTS {
        let declared = conn
            .query_row(
                "SELECT 1 FROM sqlite_schema WHERE type = lower(?1) AND name = ?2",
                [kind, name],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !declared {
            let end = if kind == "TRIGGER" { "END;" } else { ";" };
            let from = schema
                .find(&format!("CREATE {kind} {name} "))
                .expect("canonical DDL");
            let to = from + schema[from..].find(end).expect("canonical DDL") + end.len();
            conn.execute_batch(&schema[from..to])?;
        }
    }
    Ok(())
}

/// The canonical statements creating `table` and its key index when it
/// has one, cut out of the first migration so they never drift from it.
fn ddl(table: &str) -> String {
    let schema = sql::MIGRATION_0001;
    let cut = |start: &str, end: &str| {
        let from = schema.find(start)?;
        let to = from + schema[from..].find(end)? + end.len();
        Some(String::from(&schema[from..to]))
    };
    let create = cut(&format!("CREATE TABLE {table} ("), ") STRICT;").expect("canonical DDL");
    match cut(&format!("CREATE UNIQUE INDEX {table}_key"), ";") {
        Some(key) => create + "\n" + &key,
        None => create,
    }
}

/// Whether the store holds `table`: one of the [`RECONCILED`] ones is
/// missing from a store whose owner predates it.
pub(crate) fn has_table(conn: &Connection, table: &str) -> Result<bool, PimdirError> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1",
            [table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Whether `table` has `column`: `collections.role` is missing from a
/// store its owner has not reconciled yet (§6), which a reader reads as
/// `NULL`.
pub(crate) fn has_column(
    conn: &Connection,
    table: &str,
    column: &str,
) -> Result<bool, PimdirError> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2",
            [table, column],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}
/// Refuses a store stamped at the current version that this crate does
/// not read: an earlier draft's shape, or disagreeing stamps (§4.2).
///
/// The shape is checked first, so a store lacking `store_meta` is named
/// as stale rather than failing the read of its stamp. The spec is a
/// draft with no migration path, so a store missing a table is recreated
/// by its owner, never reconciled here.
pub(crate) fn check(conn: &Connection) -> Result<(), PimdirError> {
    let mut stmt =
        conn.prepare("SELECT name FROM sqlite_schema WHERE type IN ('table', 'trigger')")?;
    let declared: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if let Some(missing) = SCHEMA
        .iter()
        .find(|name| !declared.iter().any(|d| d == *name))
    {
        return Err(PimdirError::Stale { missing });
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
