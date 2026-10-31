//! The crate's vendored SQL against the canonical pimdir specification.
//!
//! spec/ is a byte-for-byte copy of the specification's migrations/storage/,
//! queries/storage/ and, read by the `search` feature, migrations/search/
//! and queries/search/, and `sql` is generated from it, so a consumer
//! holding its own SQLite binding runs the format's own statements by
//! name. The copy is only worth something if it is checked: the crate and
//! the spec live in separate repositories, so nothing else notices when
//! one moves and the other does not.
//!
//! Three things are checked. The vendored files must be the spec's, byte
//! for byte, both ways, which is what makes the generated schema the
//! canonical one. Every canonical statement must be reachable by name.
//! Every statement, canonical or this crate's own, must prepare against
//! the schema, this being the only place the spec's own SQL is ever
//! loaded.
//!
//! The spec is a sibling checkout, so the comparisons skip when it is
//! absent and run whenever the two sit side by side.

#![cfg(feature = "client")]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use io_pimdir::sql;
use rusqlite::Connection;

/// The canonical spec checkout, beside this one.
fn spec_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .join("pimdir");

    dir.join("migrations/storage/0001_init.sql")
        .is_file()
        .then_some(dir)
}

fn applied(ddl: &str) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(ddl)
        .expect("the schema applies to an empty database");
    // NOTE: the list parameters' table, per connection (STORAGE §4.4).
    conn.execute_batch(sql::CREATE_PARAM)
        .expect("the list parameters' table creates");
    conn
}

#[test]
fn every_canonical_statement_is_inlined() {
    let Some(spec) = spec_dir() else {
        eprintln!("skipped: no pimdir spec checkout beside this one");
        return;
    };

    let canonical: BTreeSet<String> = canonical_statements(&spec)
        .into_iter()
        .map(|(name, _)| name)
        .collect();

    let inlined: BTreeSet<String> = sql::all().map(|(name, _)| name.to_string()).collect();
    let missing: Vec<&String> = canonical.difference(&inlined).collect();

    assert!(
        missing.is_empty(),
        "canonical statements with no constant here, unreachable by name: {missing:?}"
    );
}

/// Every canonical statement the spec checkout carries, as
/// `(CONSTANT_NAME, sql)`: one file per statement under the profile
/// directories of queries/storage/, named after it.
fn canonical_statements(spec: &Path) -> Vec<(String, String)> {
    let mut statements = Vec::new();

    let profiles = ["read", "queue", "owner"];
    let files = profiles
        .iter()
        .flat_map(|profile| fs::read_dir(spec.join("queries/storage").join(profile)).unwrap());
    for entry in files {
        let path = entry.unwrap().path();
        let name = path.file_stem().unwrap().to_str().unwrap().to_uppercase();
        let sql: String = fs::read_to_string(&path)
            .unwrap()
            .lines()
            .filter(|line| !line.trim_start().starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n");
        statements.push((name, sql.trim().to_string()));
    }

    statements.sort();
    statements
}

/// Every canonical statement prepares against the canonical schema.
///
/// The name check above says the crate reaches each one; this says the
/// format's own SQL is SQL and agrees with the format's own schema.
/// Nothing else checks it: this repository holds the only toolchain that
/// ever loads those files.
#[test]
fn every_canonical_statement_prepares() {
    let Some(spec) = spec_dir() else {
        eprintln!("skipped: no pimdir spec checkout beside this one");
        return;
    };

    let migration = fs::read_to_string(spec.join("migrations/storage/0001_init.sql")).unwrap();
    let conn = applied(&migration);

    for (name, sql) in canonical_statements(&spec) {
        if let Err(err) = conn.prepare(&sql) {
            panic!("canonical `{name}` does not prepare against the canonical schema: {err:?}");
        }
    }
}

/// Canonical statements this crate inlines under their own name with an
/// equivalent text (STORAGE §4.4): the descending pages bind a `NULL`
/// cursor for the first page rather than a key above every other one.
#[test]
fn every_inlined_statement_prepares() {
    // a statement naming a column the schema does not have is a drift the
    // name check cannot see, surfacing only when a consumer runs it
    let conn = applied(sql::MIGRATION_0001);

    for (name, text) in sql::all() {
        assert!(
            conn.prepare(text).is_ok(),
            "`{name}` does not prepare against the inlined schema: {:?}",
            conn.prepare(text).unwrap_err()
        );
    }
}

/// Every file under `dir`, relative path to bytes.
fn tree(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let relative = path.strip_prefix(dir).unwrap().to_path_buf();
                files.insert(relative, fs::read(&path).unwrap());
            }
        }
    }
    files
}

#[test]
fn the_vendored_copy_is_the_spec_byte_for_byte() {
    let Some(spec) = spec_dir() else {
        eprintln!("skipped: no pimdir spec checkout beside this one");
        return;
    };
    let vendored = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("spec");

    for dir in [
        "migrations/storage",
        "queries/storage",
        "migrations/search",
        "queries/search",
    ] {
        let theirs = tree(&spec.join(dir));
        let ours = tree(&vendored.join(dir));
        let differing: Vec<&PathBuf> = theirs
            .keys()
            .chain(ours.keys())
            .filter(|path| theirs.get(*path) != ours.get(*path))
            .collect();
        assert!(
            differing.is_empty(),
            "spec/{dir} differs from the spec checkout, re-vendor it: {differing:?}"
        );
    }
}

/// Every canonical search statement the spec checkout carries, as
/// `(CONSTANT_NAME, sql)`, on [`canonical_statements`]' terms.
#[cfg(feature = "search")]
fn search_statements(spec: &Path) -> Vec<(String, String)> {
    let mut statements = Vec::new();
    for entry in fs::read_dir(spec.join("queries/search")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_stem().unwrap().to_str().unwrap().to_uppercase();
        statements.push((name, fs::read_to_string(&path).unwrap()));
    }
    statements.sort();
    statements
}

/// An index database with a store attached as `store`, each at the given
/// schema, as SEARCH §2 prepares every search statement.
#[cfg(feature = "search")]
fn index(store: &str, index: &str) -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let path = |name: &str| dir.path().join(name);
    Connection::open(path("pimdir.db"))
        .unwrap()
        .execute_batch(store)
        .expect("the store schema applies to an empty database");
    let conn = Connection::open(path("index.db")).unwrap();
    conn.execute_batch(index)
        .expect("the index schema applies to an empty database");
    conn.execute(
        "ATTACH DATABASE ?1 AS store",
        [path("pimdir.db").to_str().unwrap()],
    )
    .unwrap();
    conn.execute_batch(sql::CREATE_PARAM).unwrap();
    (dir, conn)
}

/// Every canonical search statement is reachable by name and prepares on
/// the canonical index schema with the canonical store attached.
#[cfg(feature = "search")]
#[test]
fn every_canonical_search_statement_is_inlined_and_prepares() {
    let Some(spec) = spec_dir() else {
        eprintln!("skipped: no pimdir spec checkout beside this one");
        return;
    };

    let inlined: BTreeSet<&str> = sql::search::all().map(|(name, _)| name).collect();
    let store = fs::read_to_string(spec.join("migrations/storage/0001_init.sql")).unwrap();
    let schema = fs::read_to_string(spec.join("migrations/search/0001_init.sql")).unwrap();
    let (_dir, conn) = index(&store, &schema);

    for (name, sql) in search_statements(&spec) {
        assert!(
            inlined.contains(name.as_str()),
            "canonical search `{name}` has no constant here, unreachable by name"
        );
        if let Err(err) = conn.prepare(&sql) {
            panic!(
                "canonical search `{name}` does not prepare against the canonical schemas: {err:?}"
            );
        }
    }
}

/// Every search statement, canonical or this crate's own, prepares on
/// the inlined schemas.
#[cfg(feature = "search")]
#[test]
fn every_inlined_search_statement_prepares() {
    let (_dir, conn) = index(sql::MIGRATION_0001, sql::search::MIGRATION_0001);

    for (name, text) in sql::search::all() {
        assert!(
            conn.prepare(text).is_ok(),
            "`{name}` does not prepare against the inlined schemas: {:?}",
            conn.prepare(text).unwrap_err()
        );
    }
}
