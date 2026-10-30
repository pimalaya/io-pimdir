//! # Schema
//!
//! The migration runner (STORAGE §6) and the checks refusing a store this
//! crate does not read: a newer version, disagreeing stamps, an earlier
//! draft's shape, a foreign hash.

use alloc::{borrow::Cow, format, string::String, vec::Vec};

use std::{collections::BTreeMap, path::Path, sync::LazyLock};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, named_params};

use crate::{
    client::{PimdirError, PimdirNotCurrent, blobs::PimdirBlobs, busy_or_sql, rows},
    codec,
    hash::PimdirHashAlgo,
    object::PimdirHash,
    sql,
    summary::{PimdirSummary, calendar, mail},
};

/// The core tables and triggers every draft's store holds: one missing is
/// a damaged store, refused before anything is reconciled, where any other
/// difference from the canonical schema is an earlier draft's, reconciled
/// on open (§6). The triggers are named since the last drafts moved the
/// feed into them, and a store lacking one stamps nothing.
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

/// The Annex A revision this crate derives summaries, addresses and sort
/// keys under (STORAGE §6), recorded in `store_meta.derivation`.
///
/// NOTE: raising it owes a re-derivation of the columns the revisions in
/// between changed, from every held body, before [`derive`] records it.
pub(crate) const DERIVATION: i64 = 2;

/// Opens a store as its owner, in STORAGE §6's order: a store above this
/// crate's version and a database that is not a pimdir store are refused
/// before anything is written, the algorithm checked against the declared
/// one, a version 1 store reconciled to 0001_init.sql alone, then every
/// migration above the store's version run, each bumping both stamps in
/// its transaction. Answers the store's algorithm.
pub(crate) fn init(
    conn: &mut Connection,
    declared: Option<PimdirHashAlgo>,
    dir: &Path,
) -> Result<PimdirHashAlgo, PimdirError> {
    migrate(conn, declared, dir, sql::MIGRATIONS)
}

/// [`init`] over a given migration list, the first being 0001_init.sql,
/// so a test proves the runner with a migration the spec has not written.
fn migrate(
    conn: &mut Connection,
    declared: Option<PimdirHashAlgo>,
    dir: &Path,
    migrations: &[&str],
) -> Result<PimdirHashAlgo, PimdirError> {
    let latest = migrations.len() as i64;
    let version = user_version(conn)?;
    if version > latest {
        return Err(PimdirError::Version { found: version });
    }

    let algo = match version {
        0 if holds_schema(conn)? => return Err(PimdirError::Foreign),
        0 => declared.unwrap_or_default(),
        _ => {
            // NOTE: a core table or trigger missing is a damaged store,
            // not an earlier one: refused before anything is reconciled.
            check(conn)?;
            format(conn)?;
            hash_algo(conn, declared)?
        }
    };

    // NOTE: the journal mode is a write to the database header, so it
    // waits until the store is known to be one this crate may write, and
    // it is chosen once, at creation: a store its operator switched to a
    // rollback journal for a network filesystem (§8) stays on it.
    if version == 0 {
        conn.execute_batch("PRAGMA journal_mode = WAL;")?;
    }

    if version == 1 {
        reconcile(conn, &PimdirBlobs::open(dir, algo), migrations[0])?;
    }

    for (index, migration) in migrations.iter().enumerate() {
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
                named_params! {
                    ":version": reached,
                    ":hash_algo": algo.as_str(),
                    ":derivation": DERIVATION,
                },
            )?;
        } else {
            tx.execute(
                sql::SET_STORE_VERSION,
                named_params! { ":version": reached },
            )?;
        }
        tx.pragma_update(None, "user_version", reached)?;
        tx.commit().map_err(busy_or_sql)?;
    }

    check(conn)?;
    derive(conn, &PimdirBlobs::open(dir, algo))?;
    Ok(algo)
}

/// Brings a store derived under an earlier Annex A revision to this
/// crate's (STORAGE §6): the columns the revisions in between changed
/// re-derived from every held body, then the revision recorded with
/// `set_derivation`, which never lowers it. A store at the same or a later
/// revision is left as it is, written by no statement.
fn derive(conn: &mut Connection, blobs: &PimdirBlobs) -> Result<(), PimdirError> {
    let stored: i64 =
        conn.query_row("SELECT derivation FROM store_meta WHERE id = 1", [], |r| {
            r.get(0)
        })?;
    if stored >= DERIVATION {
        return Ok(());
    }
    // NOTE: revision 2 is the only one a re-derivation is owed for; its
    // last page records the revision, so a store at 1 resumes from the
    // top on the next open until it lands.
    rederive_task_capture(conn, blobs)
}

/// Re-derives revision 2's columns (Annex A.4): `task_summary.created`
/// and every task's addresses, its `contact` rows among them, from every
/// held task body, a page of `list_held_tasks` to a transaction, recording
/// the revision in the transaction writing the last page (§6). A body the
/// blob directory does not hold leaves its row as it was.
fn rederive_task_capture(conn: &mut Connection, blobs: &PimdirBlobs) -> Result<(), PimdirError> {
    const PAGE: i64 = 500;
    let mut after: Option<(String, String)> = None;
    loop {
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        let page: Vec<(String, String, String)> = rows(
            &tx,
            sql::LIST_HELD_TASKS,
            named_params! {
                ":after_collection": after.as_ref().map(|(collection, _)| collection.as_str()),
                ":after_link_id": after.as_ref().map(|(_, link_id)| link_id.as_str()),
                ":limit": PAGE,
            },
            |row| Ok((row.get(0)?, row.get(1)?, row.get(3)?)),
        )?;
        let last = (page.len() as i64) < PAGE;
        after = page
            .last()
            .map(|(collection, link_id, _)| (collection.clone(), link_id.clone()));
        for (collection, link_id, hash) in page {
            let Some(body) = blobs.get(&PimdirHash(hash))? else {
                continue;
            };
            let Some(summary) = calendar::derive(&body).summary else {
                continue;
            };
            let PimdirSummary::Task(task) = &summary else {
                continue;
            };
            let key = named_params! { ":collection": collection, ":link_id": link_id };
            tx.execute(
                sql::BACKFILL_TASK_CAPTURE,
                named_params! {
                    ":collection": collection,
                    ":link_id": link_id,
                    ":created": task.created,
                },
            )?;
            tx.execute(sql::REPLACE_ADDRESSES, key)?;
            let mut position: BTreeMap<&str, i64> = BTreeMap::new();
            for (role, address) in summary.addresses() {
                let at = position.entry(role.as_str()).or_insert(0);
                tx.execute(
                    sql::INSERT_ADDRESS,
                    named_params! {
                        ":collection": collection,
                        ":link_id": link_id,
                        ":role": role.as_str(),
                        ":position": *at,
                        ":address": address.address,
                        ":name": address.name,
                    },
                )?;
                *at += 1;
            }
            tx.execute(sql::STAMP_ITEM, key)?;
        }
        if last {
            tx.execute(
                sql::SET_DERIVATION,
                named_params! { ":derivation": DERIVATION },
            )?;
        }
        tx.commit().map_err(busy_or_sql)?;
        if last {
            return Ok(());
        }
    }
}

/// The store's `PRAGMA user_version`, `0` for a fresh database.
fn user_version(conn: &Connection) -> Result<i64, PimdirError> {
    Ok(conn.pragma_query_value(None, "user_version", |r| r.get(0))?)
}

/// Whether a database at `user_version` 0 declares anything: a pimdir
/// store is created with its version in one transaction, so such a
/// database is another program's and is never written (STORAGE §3).
fn holds_schema(conn: &Connection) -> Result<bool, PimdirError> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' LIMIT 1",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Refuses a database whose `store_meta.format` is not `pimdir`: a
/// directory is a pimdir store if and only if its database says so
/// (STORAGE §3), and nothing in another one is reconciled away.
fn format(conn: &Connection) -> Result<(), PimdirError> {
    let format: Option<String> = conn
        .query_row("SELECT format FROM store_meta WHERE id = 1", [], |r| {
            r.get(0)
        })
        .optional()?;
    match format.as_deref() {
        Some("pimdir") | None => Ok(()),
        Some(_) => Err(PimdirError::Foreign),
    }
}

/// One table, index or trigger as `sqlite_schema` declares it.
struct PimdirSchemaObject {
    /// `table`, `index` or `trigger`.
    kind: String,
    /// The table it belongs to.
    table: String,
    /// Its declaring statement as stored.
    sql: String,
}

/// Reconciles a version 1 store against 0001_init.sql's own text (§6), the
/// one normalisation and only a version 1 store's: the script is applied
/// to an empty in-memory database, and every table, index and trigger
/// whose stored text differs from its canonical one, comments dropped and
/// whitespace collapsed, is brought to it in one transaction; a store
/// already current is left untouched.
fn reconcile(conn: &mut Connection, blobs: &PimdirBlobs, init: &str) -> Result<(), PimdirError> {
    let canonical = Connection::open_in_memory()?;
    canonical.execute_batch(init)?;
    let wanted = objects(&canonical)?;
    let held = objects(conn)?;
    if first_difference(&wanted, &held).is_none() {
        return Ok(());
    }

    conn.execute_batch("PRAGMA foreign_keys = OFF; PRAGMA legacy_alter_table = ON;")?;
    let reconciled = rebuild(conn, &canonical, &wanted, &held, blobs);
    conn.execute_batch("PRAGMA legacy_alter_table = OFF; PRAGMA foreign_keys = ON;")?;
    reconciled
}

/// The four steps of §6 in one transaction: the data a new constraint
/// would refuse repaired and the refcounts settled, every table whose text
/// differs rebuilt from the canonical one, what is missing created, what
/// differs recreated, what the canonical schema lacks dropped; then the
/// columns a rebuild added backfilled, the refcounts settled again, and no
/// foreign key left dangling.
fn rebuild(
    conn: &mut Connection,
    canonical: &Connection,
    wanted: &BTreeMap<String, PimdirSchemaObject>,
    held: &BTreeMap<String, PimdirSchemaObject>,
    blobs: &PimdirBlobs,
) -> Result<(), PimdirError> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(busy_or_sql)?;
    let without_invitation =
        has_table(&tx, "mail_summary")? && !has_column(&tx, "mail_summary", "invitation")?;
    let without_organisation =
        has_table(&tx, "task_summary")? && !has_column(&tx, "task_summary", "priority")?;
    let without_shared_object =
        has_table(&tx, "bindings")? && !has_column(&tx, "bindings", "shared_object")?;
    let pointers = ["objects", "items", "bindings", "queue"];
    let counted = pointers.iter().all(|table| held.contains_key(*table));
    // NOTE: both name bindings.shared_object, so a store from before it
    // runs them once bindings is rebuilt.
    if counted && !without_shared_object {
        tx.execute(sql::REPAIR_SHARED_OBJECTS, [])?;
        tx.execute(sql::RECOMPUTE_REFCOUNTS, [])?;
    }
    if held.contains_key("items") {
        tx.execute(sql::REPAIR_LEVELS, [])?;
    }
    if held.contains_key("collections") {
        tx.execute(sql::REPAIR_KINDS, [])?;
    }

    for (name, object) in wanted.iter().filter(|(_, o)| o.kind == "table") {
        let Some(stored) = held.get(name) else {
            continue;
        };
        if normalised(&stored.sql) == normalised(&object.sql) {
            continue;
        }
        for (attached, other) in held {
            if other.table == *name && other.kind != "table" {
                tx.execute_batch(&format!("DROP {} {attached};", other.kind.to_uppercase()))?;
            }
        }
        let columns: Vec<String> = rows(
            &tx,
            "SELECT name FROM pragma_table_info(?1)",
            [name],
            |row| row.get(0),
        )?;
        let canonical_columns: Vec<String> = rows(
            canonical,
            "SELECT name FROM pragma_table_info(?1)",
            [name],
            |row| row.get(0),
        )?;
        let shared: Vec<&String> = columns
            .iter()
            .filter(|column| canonical_columns.contains(column))
            .collect();
        let shared = shared
            .iter()
            .map(|column| column.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        tx.execute_batch(&format!(
            "ALTER TABLE {name} RENAME TO {name}_aside;\n{};\n\
             INSERT INTO {name} ({shared}) SELECT {shared} FROM {name}_aside;",
            object.sql
        ))?;
        if has_table(&tx, "sqlite_sequence")? {
            tx.execute_batch(&format!(
                "UPDATE sqlite_sequence SET seq = max(seq, (SELECT seq FROM sqlite_sequence \
                 WHERE name = '{name}_aside')) WHERE name = '{name}';\n\
                 INSERT INTO sqlite_sequence(name, seq) SELECT '{name}', seq FROM sqlite_sequence \
                 WHERE name = '{name}_aside' \
                 AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = '{name}');"
            ))?;
        }
        tx.execute_batch(&format!("DROP TABLE {name}_aside;"))?;
    }

    for (name, object) in wanted.iter().filter(|(_, o)| o.kind == "table") {
        if !held.contains_key(name) {
            tx.execute_batch(&object.sql)?;
        }
    }
    for name in held
        .iter()
        .filter(|(_, o)| o.kind == "table")
        .map(|(name, _)| name)
    {
        if !wanted.contains_key(name) {
            tx.execute_batch(&format!("DROP TABLE {name};"))?;
        }
    }

    let now = objects(&tx)?;
    for (name, object) in now.iter().filter(|(_, o)| o.kind != "table") {
        let stale = wanted
            .get(name)
            .is_none_or(|wanted| normalised(&wanted.sql) != normalised(&object.sql));
        if stale {
            tx.execute_batch(&format!("DROP {} {name};", object.kind.to_uppercase()))?;
        }
    }
    let now = objects(&tx)?;
    for (name, object) in wanted.iter().filter(|(_, o)| o.kind != "table") {
        if !now.contains_key(name) {
            tx.execute_batch(&object.sql)?;
        }
    }

    if without_shared_object {
        tx.execute(sql::BACKFILL_SHARED_OBJECT, [])?;
        tx.execute(sql::REPAIR_SHARED_OBJECTS, [])?;
    }
    if without_invitation {
        backfill_invitation(&tx, blobs)?;
    }
    if without_organisation {
        backfill_organisation(&tx, blobs)?;
    }
    // NOTE: after every rebuild and backfill, so the counts are this
    // schema's, bindings.shared_object and a backfill's pointers counted.
    tx.execute(sql::RECOMPUTE_REFCOUNTS, [])?;

    let dangling = tx
        .query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
        .optional()?;
    if dangling.is_some() {
        return Err(PimdirError::Stale {
            missing: "a foreign key the reconciliation left dangling",
        });
    }
    tx.commit().map_err(busy_or_sql)
}

/// Re-derives `mail_summary.invitation` from every held body, a page of
/// `list_held_mail` at a time, then records the invitations they imply
/// (`link_invitations_of` over every mail): a held message is never read
/// again (§6, Annex A.1). A body the blob directory does not hold leaves
/// its row `NULL`.
fn backfill_invitation(conn: &Connection, blobs: &PimdirBlobs) -> Result<(), PimdirError> {
    let mut after: Option<(String, String)> = None;
    loop {
        let page: Vec<(String, String, String)> = rows(
            conn,
            sql::LIST_HELD_MAIL,
            named_params! {
                ":after_collection": after.as_ref().map(|(collection, _)| collection.as_str()),
                ":after_link_id": after.as_ref().map(|(_, link_id)| link_id.as_str()),
                ":limit": 500,
            },
            |row| Ok((row.get(0)?, row.get(1)?, row.get(3)?)),
        )?;
        let Some((collection, link_id, _)) = page.last() else {
            break;
        };
        after = Some((collection.clone(), link_id.clone()));
        for (collection, link_id, hash) in page {
            let Some(body) = blobs.get(&PimdirHash(hash))? else {
                continue;
            };
            let Some(PimdirSummary::Mail(mail)) = mail::derive(&body).summary else {
                continue;
            };
            conn.execute(
                sql::BACKFILL_INVITATION,
                named_params! {
                    ":collection": collection,
                    ":link_id": link_id,
                    ":invitation": mail.invitation,
                },
            )?;
        }
    }
    conn.execute(
        sql::LINK_INVITATIONS_OF,
        named_params! { ":link_id": None::<String> },
    )?;
    Ok(())
}

/// Re-derives `task_summary`'s `priority`, `categories`, `parent` and `mid`
/// from every held task body, a page of `list_held_tasks` at a time, then
/// records the references they imply (§6, §14.2): the four task rules run
/// over every item of their end. A body the blob directory does not hold
/// leaves its row empty.
fn backfill_organisation(conn: &Connection, blobs: &PimdirBlobs) -> Result<(), PimdirError> {
    let mut after: Option<(String, String)> = None;
    loop {
        let page: Vec<(String, String, String)> = rows(
            conn,
            sql::LIST_HELD_TASKS,
            named_params! {
                ":after_collection": after.as_ref().map(|(collection, _)| collection.as_str()),
                ":after_link_id": after.as_ref().map(|(_, link_id)| link_id.as_str()),
                ":limit": 500,
            },
            |row| Ok((row.get(0)?, row.get(1)?, row.get(3)?)),
        )?;
        let Some((collection, link_id, _)) = page.last() else {
            break;
        };
        after = Some((collection.clone(), link_id.clone()));
        for (collection, link_id, hash) in page {
            let Some(body) = blobs.get(&PimdirHash(hash))? else {
                continue;
            };
            let Some(PimdirSummary::Task(task)) = calendar::derive(&body).summary else {
                continue;
            };
            conn.execute(
                sql::BACKFILL_TASK_ORGANISATION,
                named_params! {
                    ":collection": collection,
                    ":link_id": link_id,
                    ":priority": task.priority,
                    ":categories": codec::ids_to_json(&task.categories),
                    ":parent": task.parent,
                    ":mid": task.mid,
                },
            )?;
        }
    }
    for rule in [
        sql::LINK_PARENTS_OF,
        sql::LINK_SOURCES_OF,
        sql::LINK_PARENTS_TO,
        sql::LINK_SOURCES_TO,
    ] {
        conn.execute(rule, named_params! { ":link_id": None::<String> })?;
    }
    Ok(())
}

/// Every table, index and trigger a database declares, by name; the
/// automatic indexes, which declare no statement, and SQLite's own tables
/// left out.
fn objects(conn: &Connection) -> Result<BTreeMap<String, PimdirSchemaObject>, PimdirError> {
    let declared = rows(
        conn,
        "SELECT name, type, tbl_name, sql FROM sqlite_schema \
         WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY rowid",
        [],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                PimdirSchemaObject {
                    kind: row.get(1)?,
                    table: row.get(2)?,
                    sql: row.get(3)?,
                },
            ))
        },
    )?;
    Ok(declared.into_iter().collect())
}

/// The first table, index or trigger by name whose stored text differs
/// from its canonical one, or that only one side holds: what [`reconcile`]
/// would rebuild, create or drop (§6); `None` for a current store.
fn first_difference(
    wanted: &BTreeMap<String, PimdirSchemaObject>,
    held: &BTreeMap<String, PimdirSchemaObject>,
) -> Option<String> {
    let differs = wanted.iter().find(|(name, object)| {
        held.get(*name)
            .is_none_or(|held| normalised(&held.sql) != normalised(&object.sql))
    });
    let extra = || held.keys().find(|name| !wanted.contains_key(*name));
    differs.map(|(name, _)| name).or_else(extra).cloned()
}

/// Checks, changing nothing, that the store is the one this crate writes
/// as it stands: at the current `user_version`, every core table present
/// with agreeing stamps ([`check`]), and its schema text the canonical one,
/// so that [`init`] would migrate and reconcile nothing (§6).
pub(crate) fn current(conn: &Connection) -> Result<(), PimdirError> {
    let found: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if found == 0 && holds_schema(conn)? {
        return Err(PimdirError::Foreign);
    }
    if found != sql::VERSION {
        return Err(PimdirError::NotCurrent(PimdirNotCurrent::Version { found }));
    }
    check(conn)?;
    format(conn)?;

    let canonical = Connection::open_in_memory()?;
    for migration in sql::MIGRATIONS {
        canonical.execute_batch(migration)?;
    }
    match first_difference(&objects(&canonical)?, &objects(conn)?) {
        Some(object) => Err(PimdirError::NotCurrent(PimdirNotCurrent::Shape { object })),
        None => Ok(()),
    }
}

/// A declaring statement as §6 compares it: every `--` comment dropped and
/// whitespace collapsed to single spaces.
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

/// Whether the store holds `table`: one a later draft added is
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

/// Each canonical table's columns in order, each with the value a reader
/// reads it as on a store lacking it (§6): its `DEFAULT` where the schema
/// declares it `NOT NULL` with one, else `NULL`.
static CANONICAL_COLUMNS: LazyLock<BTreeMap<String, Vec<(String, String)>>> = LazyLock::new(|| {
    let conn = Connection::open_in_memory().expect("an in-memory database opens");
    for migration in sql::MIGRATIONS {
        conn.execute_batch(migration)
            .expect("the canonical schema applies to an empty database");
    }
    let tables: Vec<String> = rows(
        &conn,
        "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        [],
        |row| row.get(0),
    )
    .expect("the canonical tables read");
    tables
        .into_iter()
        .map(|table| {
            let columns = rows(
                &conn,
                "SELECT name, \"notnull\", dflt_value FROM pragma_table_info(?1)",
                [&table],
                |row| {
                    let name: String = row.get(0)?;
                    let value = match (row.get::<_, bool>(1)?, row.get(2)?) {
                        (true, Some(default)) => default,
                        _ => String::from("NULL"),
                    };
                    Ok((name, value))
                },
            )
            .expect("the canonical columns read");
            (table, columns)
        })
        .collect()
});

/// The words that may follow a table in a `FROM` or `JOIN` clause and are
/// not its alias.
const NOT_ALIAS: &[&str] = &[
    "WHERE",
    "ON",
    "JOIN",
    "LEFT",
    "RIGHT",
    "INNER",
    "CROSS",
    "FULL",
    "NATURAL",
    "ORDER",
    "GROUP",
    "LIMIT",
    "USING",
    "UNION",
    "EXCEPT",
    "INTERSECT",
    "HAVING",
    "WINDOW",
    "INDEXED",
    "NOT",
];

/// A canonical statement as the store `schema` (`main`, or an attached
/// one) answers it, read as its reconciliation would leave it before any
/// backfill (§6): each canonical table the statement reads that the store
/// lacks, or holds without some column, is read through a subquery
/// answering the canonical columns, one the store lacks as its `NOT NULL`
/// default or `NULL`, a missing table answering no row. The statement's
/// parameters and result columns are kept; on a current store it is the
/// statement itself.
///
/// A table is one named after `FROM` or `JOIN`, bare or qualified by
/// `schema`; comments and string literals are copied as they are.
pub(crate) fn as_reconciled(
    conn: &Connection,
    schema: &str,
    statement: &str,
) -> Result<String, PimdirError> {
    let qualifier = match schema {
        "main" => String::new(),
        schema => format!("{schema}."),
    };
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out = String::with_capacity(statement.len());
    let mut rest = statement;
    let mut after_from = false;

    while let Some(c) = rest.chars().next() {
        if rest.starts_with("--") {
            let end = rest.find('\n').unwrap_or(rest.len());
            out.push_str(&rest[..end]);
            rest = &rest[end..];
            continue;
        }
        if c == '\'' {
            let end = rest[1..].find('\'').map_or(rest.len(), |at| at + 2);
            out.push_str(&rest[..end]);
            rest = &rest[end..];
            after_from = false;
            continue;
        }
        if !word(c) {
            if !c.is_whitespace() {
                after_from = false;
            }
            out.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        }

        let mut end = rest.find(|c: char| !word(c)).unwrap_or(rest.len());
        // NOTE: a qualified name reads as one word, the schema and the
        // table, so `store.items` is a table and `c.kind` is not one.
        if rest[end..].starts_with('.') {
            let after = &rest[end + 1..];
            end += 1 + after.find(|c: char| !word(c)).unwrap_or(after.len());
        }
        let token = &rest[..end];
        rest = &rest[end..];

        let table = match after_from {
            true => Some(token.strip_prefix(qualifier.as_str()).unwrap_or(token))
                .filter(|table| !table.contains('.')),
            false => None,
        };
        after_from = token.eq_ignore_ascii_case("FROM") || token.eq_ignore_ascii_case("JOIN");

        let replaced = match table {
            Some(table) => reconciled_table(conn, schema, table)?,
            None => None,
        };
        let Some(replaced) = replaced else {
            out.push_str(token);
            continue;
        };
        out.push_str(&replaced);

        let next = rest.trim_start();
        let next = &next[..next.find(|c: char| !word(c)).unwrap_or(next.len())];
        let aliased = !next.is_empty()
            && !NOT_ALIAS
                .iter()
                .any(|keyword| next.eq_ignore_ascii_case(keyword));
        if !aliased {
            out.push_str(" AS ");
            out.push_str(table.unwrap_or_default());
        }
    }

    Ok(out)
}

/// Whether a store holds every canonical table with every canonical
/// column, read once when a handle opens it, so the statements it runs are
/// rewritten ([`as_reconciled`]) only while it does not.
///
/// A store found whole stays whole: only its owner's reconciliation
/// changes its schema, and that adds what it lacked (§6).
#[derive(Clone, Copy, Debug)]
pub(crate) struct StoreShape {
    /// The database the store is, `main` or the name it is attached as.
    schema: &'static str,
    /// Whether it held every canonical table and column when read.
    whole: bool,
}

impl StoreShape {
    /// Reads the shape of the store `schema` (`main`, or an attached one).
    pub(crate) fn read(conn: &Connection, schema: &'static str) -> Result<Self, PimdirError> {
        let held: Vec<(String, String)> = rows(
            conn,
            &format!(
                "SELECT s.name, p.name FROM \"{schema}\".sqlite_schema s \
                 JOIN pragma_table_info(s.name, '{schema}') p WHERE s.type = 'table'"
            ),
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let whole = CANONICAL_COLUMNS.iter().all(|(table, columns)| {
            columns
                .iter()
                .all(|(column, _)| held.iter().any(|(t, c)| t == table && c == column))
        });
        Ok(Self { schema, whole })
    }

    /// A canonical statement as the store answers it ([`as_reconciled`]),
    /// the statement itself on a whole store.
    pub(crate) fn sql<'a>(
        &self,
        conn: &Connection,
        statement: &'a str,
    ) -> Result<Cow<'a, str>, PimdirError> {
        match self.whole {
            true => Ok(Cow::Borrowed(statement)),
            false => Ok(Cow::Owned(as_reconciled(conn, self.schema, statement)?)),
        }
    }
}

/// The subquery a canonical `table` of the store `schema` is read
/// through (§6), `None` when the store holds it whole or it is not a
/// canonical table.
fn reconciled_table(
    conn: &Connection,
    schema: &str,
    table: &str,
) -> Result<Option<String>, PimdirError> {
    let Some(columns) = CANONICAL_COLUMNS.get(table) else {
        return Ok(None);
    };
    let held: Vec<String> = rows(
        conn,
        "SELECT name FROM pragma_table_info(?1, ?2)",
        [table, schema],
        |row| row.get(0),
    )?;
    let whole = columns
        .iter()
        .all(|(column, _)| held.iter().any(|held| held == column));
    if whole {
        return Ok(None);
    }

    let select: Vec<String> = columns
        .iter()
        .map(
            |(column, value)| match held.iter().any(|held| held == column) {
                true => format!("\"{column}\""),
                false => format!("{value} AS \"{column}\""),
            },
        )
        .collect();
    Ok(Some(match held.is_empty() {
        true => format!("(SELECT {} WHERE 0)", select.join(", ")),
        false => format!("(SELECT {} FROM {schema}.\"{table}\")", select.join(", ")),
    }))
}
/// Refuses a store stamped at the current version that this crate does
/// not read: a core table or trigger missing, or disagreeing stamps (§4.2).
///
/// The shape is checked first, so a store lacking `store_meta` is named
/// as stale rather than failing the read of its stamp. Only a core table
/// or trigger missing is refused ([`SCHEMA`]): any other difference is an
/// earlier draft's, which the owner open reconciles ([`reconcile`]).
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
    let user_version = user_version(conn)?;
    if let Some(store_meta) = stamped
        && store_meta != user_version
    {
        return Err(PimdirError::VersionMismatch {
            user_version,
            store_meta,
        });
    }

    Ok(())
}

/// Opens an existing database read-only or as a producer, refusing what
/// [`check`] refuses, an unstamped store, one above this crate's version
/// and a foreign database (§3, §6). A store below this crate's version, or
/// a version 1 store its owner has not reconciled yet, is read as it
/// stands: a column it lacks as `NULL`, a table it lacks as empty.
pub(crate) fn check_version(conn: &Connection) -> Result<(), PimdirError> {
    match user_version(conn)? {
        0 if holds_schema(conn)? => Err(PimdirError::Foreign),
        0 => Err(PimdirError::Uncreated),
        found if found > sql::VERSION => Err(PimdirError::Version { found }),
        _ => {
            check(conn)?;
            format(conn)
        }
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

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use rusqlite::named_params;

    use super::as_reconciled;

    /// A store whose `sources` predates the coverage columns, whose
    /// `task_summary` predates `categories` and which has no
    /// `supersessions`, as an older release wrote it.
    fn older() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        for migration in crate::sql::MIGRATIONS {
            conn.execute_batch(migration).unwrap();
        }
        conn.execute_batch(
            "PRAGMA foreign_keys = OFF;
             DROP TRIGGER sources_stamp_coverage;
             DROP TABLE sources;
             CREATE TABLE sources (collection TEXT NOT NULL, source TEXT NOT NULL,
                 checkpoint BLOB, PRIMARY KEY (collection, source)) STRICT;
             INSERT INTO sources VALUES ('INBOX', 'imap', NULL);
             ALTER TABLE task_summary DROP COLUMN categories;
             INSERT INTO task_summary(collection, link_id, summary) VALUES ('Tasks', 't', 'T');
             DROP TABLE supersessions;",
        )
        .unwrap();
        conn
    }

    /// A missing column reads as its `NOT NULL` default or `NULL`, the
    /// table aliased or not, and a missing table answers no row (§6).
    #[test]
    fn a_statement_reads_an_older_store_as_its_reconciliation_leaves_it() {
        let conn = older();

        let coverage = as_reconciled(&conn, "main", crate::sql::LIST_COVERAGE).unwrap();
        let row: (String, Option<String>, Option<String>) = conn
            .query_row(&coverage, named_params! { ":collection": "INBOX" }, |r| {
                Ok((r.get(0)?, r.get(3)?, r.get(4)?))
            })
            .unwrap();
        assert_eq!(row, ("imap".into(), None, None), "{coverage}");

        let band = as_reconciled(&conn, "main", "SELECT s.round_band FROM sources s").unwrap();
        let band: i64 = conn.query_row(&band, [], |r| r.get(0)).unwrap();
        assert_eq!(band, 0, "the NOT NULL default");

        let categories = as_reconciled(
            &conn,
            "main",
            "SELECT categories FROM task_summary -- categories",
        )
        .unwrap();
        let read: String = conn.query_row(&categories, [], |r| r.get(0)).unwrap();
        assert_eq!(read, "[]", "{categories}");

        let gone = as_reconciled(
            &conn,
            "main",
            "SELECT count(*) FROM items i JOIN supersessions s ON s.link_id = i.link_id",
        )
        .unwrap();
        let count: i64 = conn.query_row(&gone, [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0, "{gone}");
    }

    /// On a current store the statement is run as it is.
    #[test]
    fn a_current_store_runs_the_statement_unchanged() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        for migration in crate::sql::MIGRATIONS {
            conn.execute_batch(migration).unwrap();
        }
        for (_, statement) in crate::sql::CANONICAL {
            assert_eq!(as_reconciled(&conn, "main", statement).unwrap(), *statement);
        }
    }
}

/// The runner of §6 over a migration the spec has not written yet: a
/// synthetic 0002 proves the order, the stamps and the rows kept.
#[cfg(test)]
mod migration {
    use alloc::{format, string::String, vec::Vec};

    use rusqlite::Connection;

    use super::migrate;
    use crate::{
        client::{PimdirError, PimdirStore, producer::PimdirProducer},
        codec::PimdirAction,
        placement::PimdirFlags,
        sql,
    };

    /// Every table the store declares, with its row count.
    fn rows(conn: &Connection) -> Vec<(String, i64)> {
        let tables: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_schema WHERE type = 'table' \
                 AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        tables
            .into_iter()
            .map(|table| {
                let count = conn
                    .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                    .unwrap();
                (table, count)
            })
            .collect()
    }

    fn stamps(conn: &Connection) -> (i64, i64) {
        let user_version = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        let store_meta = conn
            .query_row("SELECT version FROM store_meta", [], |r| r.get(0))
            .unwrap();
        (user_version, store_meta)
    }

    #[test]
    fn a_migration_runs_after_the_reconciliation_and_keeps_every_row() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = PimdirStore::open(dir.path()).unwrap();
            store.ensure_collection("INBOX", "message/rfc822").unwrap();
            let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
            let object = producer
                .stage_body(b"Message-ID: <a@x>\r\nSubject: a\r\n\r\nbody")
                .unwrap();
            producer
                .enqueue(
                    "INBOX",
                    &PimdirAction::Add {
                        link_id: None,
                        flags: PimdirFlags::default(),
                        object: Some(object.hash.clone()),
                        supersedes: None,
                    },
                    Some(&object),
                )
                .unwrap();
            producer
                .enqueue("INBOX", &PimdirAction::Remove { seq: 9999 }, None)
                .unwrap();
            drop(producer);
            let mut store = store.for_source("local");
            store.drain().unwrap();
            let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
            producer
                .enqueue("INBOX", &PimdirAction::Remove { seq: 9998 }, None)
                .unwrap();
        }

        // An earlier draft's version 1 store lacks an index; a 0002 that
        // assumes the canonical version 1 shape fails unless the
        // reconciliation ran first.
        let mut conn = Connection::open(dir.path().join("pimdir.db")).unwrap();
        conn.execute_batch("DROP INDEX items_by_sort_global")
            .unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        let before = rows(&conn);

        let migrations = [
            sql::MIGRATION_0001,
            "DROP INDEX items_by_sort_global;\n\
             ALTER TABLE items ADD COLUMN later TEXT;\n\
             CREATE TABLE later_things (x INTEGER) STRICT;",
        ];
        migrate(&mut conn, None, dir.path(), &migrations).unwrap();

        assert_eq!(stamps(&conn), (2, 2), "both stamps, canonically");
        let mut after = rows(&conn);
        after.retain(|(table, _)| table != "later_things");
        assert_eq!(after, before, "every row of every table kept");
        let later: i64 = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('items') WHERE name = 'later'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(later, 1);

        // A version 2 store is no longer reconciled to 0001: opened again
        // by the same runner, nothing it added is dropped.
        migrate(&mut conn, None, dir.path(), &migrations).unwrap();
        let tables = rows(&conn);
        assert!(tables.iter().any(|(table, _)| table == "later_things"));
        drop(conn);

        // And this crate, at version 1, refuses it without writing it.
        let found = PimdirStore::open(dir.path()).err().unwrap();
        assert!(
            matches!(found, PimdirError::Version { found: 2 }),
            "{found:?}"
        );
    }
}
