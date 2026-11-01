//! The crate's vendored copy against the canonical pimdir specification.
//!
//! spec/ is a byte-for-byte copy of the specification's migrations/,
//! queries/, vectors/ and payloads/ at the revision spec/REVISION names.
//! `sql` is generated from its storage statements and, read by the `search`
//! feature, its search statements, so a consumer holding its own SQLite
//! binding runs the format's own statements by name; the suites read the
//! vectors from it. The copy is only worth something if it is checked:
//! the crate and the spec live in separate repositories, so nothing else
//! notices when one moves and the other does not.
//!
//! Four things are checked. The vendored files must be the spec's at the
//! pinned revision, byte for byte, both ways, which is what makes the
//! generated schema the canonical one. vectors/ and payloads/ must be what
//! vectors/manifest.json lists, file by file and case by case. Every
//! canonical statement must be reachable by name. Every statement,
//! canonical or this crate's own, must prepare against the schema, this
//! being the only place the spec's own SQL is ever loaded.
//!
//! The first check reads the pinned revision from the git history of a
//! pimdir checkout beside this one, whatever its working tree holds, and
//! compares nothing when there is none holding it. The others read spec/
//! and always run.

#![cfg(feature = "client")]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
};

use io_pimdir::sql;
use rusqlite::Connection;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// The directories of spec/ copied whole from the specification.
const VENDORED: [&str; 4] = ["migrations", "queries", "vectors", "payloads"];

/// The files of vectors/ and payloads/ the manifest does not list: itself
/// and the README.
const UNLISTED: [&str; 2] = ["vectors/manifest.json", "vectors/README.md"];

/// The vendored copy, at the revision spec/REVISION names.
fn spec_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("spec")
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
    let spec = spec_dir();

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

/// Prepares every statement `text` holds: one, or several as
/// `optimize_text`, each ending a line with its semicolon.
fn prepare_all(conn: &Connection, text: &str) -> rusqlite::Result<()> {
    let mut statement = String::new();
    for line in text.lines() {
        statement.push_str(line);
        statement.push('\n');
        if line.trim_end().ends_with(';') && !line.trim_start().starts_with("--") {
            conn.prepare(&statement)?;
            statement.clear();
        }
    }
    if !statement.trim().is_empty() {
        conn.prepare(&statement)?;
    }
    Ok(())
}

/// Every canonical statement the vendored copy carries, as
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
    let spec = spec_dir();

    let migration = fs::read_to_string(spec.join("migrations/storage/0001_init.sql")).unwrap();
    let conn = applied(&migration);

    for (name, sql) in canonical_statements(&spec) {
        if let Err(err) = prepare_all(&conn, &sql) {
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
            prepare_all(&conn, text).is_ok(),
            "`{name}` does not prepare against the inlined schema: {:?}",
            prepare_all(&conn, text).unwrap_err()
        );
    }
}

/// Every file under [`VENDORED`] in spec/, path relative to spec/ to bytes.
fn vendored_tree() -> BTreeMap<PathBuf, Vec<u8>> {
    let spec = spec_dir();
    let mut files = BTreeMap::new();
    let mut pending: Vec<PathBuf> = VENDORED.iter().map(|dir| spec.join(dir)).collect();
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let relative = path.strip_prefix(&spec).unwrap().to_path_buf();
                files.insert(relative, fs::read(&path).unwrap());
            }
        }
    }
    files
}

/// Every file under [`VENDORED`] at `revision` of the git repository at
/// `checkout`, path to bytes, read from its history and never from its
/// working tree; `None` when there is no repository there holding the
/// revision.
fn revision_tree(checkout: &Path, revision: &str) -> Option<BTreeMap<PathBuf, Vec<u8>>> {
    let git = |args: &[&str]| {
        let mut command = Command::new("git");
        command.arg("-C").arg(checkout).args(args);
        command
    };

    let commit = format!("{revision}^{{commit}}");
    let held = git(&["cat-file", "-e", &commit])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !held {
        return None;
    }

    let listing = git(&["ls-tree", "-r", "-z", "--name-only", revision, "--"])
        .args(VENDORED)
        .output()
        .unwrap();
    assert!(listing.status.success(), "git ls-tree fails at {revision}");
    let paths: Vec<String> = listing
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8(path.to_vec()).unwrap())
        .collect();

    // NOTE: one `git cat-file --batch` answers every path, each answer a
    // `<oid> blob <size>` line, the bytes and a newline.
    let mut batch = git(&["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let requests: String = paths
        .iter()
        .map(|path| format!("{revision}:{path}\n"))
        .collect();
    let mut stdin = batch.stdin.take().unwrap();
    let writer = thread::spawn(move || stdin.write_all(requests.as_bytes()).unwrap());
    let output = batch.wait_with_output().unwrap();
    writer.join().unwrap();
    assert!(output.status.success(), "git cat-file fails at {revision}");

    let mut files = BTreeMap::new();
    let mut rest = output.stdout.as_slice();
    for path in paths {
        let end = rest.iter().position(|byte| *byte == b'\n').unwrap();
        let header = std::str::from_utf8(&rest[..end]).unwrap();
        let size: usize = header.rsplit(' ').next().unwrap().parse().unwrap();
        let body = &rest[end + 1..];
        files.insert(PathBuf::from(path), body[..size].to_vec());
        rest = &body[size + 1..];
    }

    Some(files)
}

/// vectors/ and payloads/ are what vectors/manifest.json lists (STORAGE
/// §16): every listed file present under its SHA-256, none present and
/// unlisted, and every case file holding the number of cases the manifest
/// counts, sync/ included. Needs no pimdir checkout, so a copy edited by
/// hand fails here whatever sits beside it.
#[test]
fn the_vendored_vectors_are_the_manifests() {
    let spec = spec_dir();
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(spec.join("vectors/manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["v"], 1, "manifest.json at another version");

    let listed = manifest["files"].as_object().unwrap();
    let held: BTreeMap<String, Vec<u8>> = vendored_tree()
        .into_iter()
        .map(|(path, bytes)| (path.to_string_lossy().replace('\\', "/"), bytes))
        .filter(|(path, _)| path.starts_with("vectors/") || path.starts_with("payloads/"))
        .filter(|(path, _)| !UNLISTED.contains(&path.as_str()))
        .collect();

    let mut departures = Vec::new();
    for (path, digest) in listed {
        match held.get(path) {
            None => departures.push(format!("{path}: listed, absent")),
            Some(bytes) => {
                let found: String = Sha256::digest(bytes)
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
                if Some(found.as_str()) != digest.as_str() {
                    departures.push(format!("{path}: SHA-256 {found}, listed {digest}"));
                }
            }
        }
    }
    for path in held.keys().filter(|path| !listed.contains_key(*path)) {
        departures.push(format!("{path}: present, unlisted"));
    }

    for (path, lists) in manifest["cases"].as_object().unwrap() {
        let file: Value = serde_json::from_slice(&held[path]).unwrap();
        for (pointer, count) in lists.as_object().unwrap() {
            let found = file
                .pointer(pointer)
                .and_then(Value::as_array)
                .map(Vec::len);
            if found != count.as_u64().map(|count| count as usize) {
                departures.push(format!("{path}{pointer}: {found:?} cases, listed {count}"));
            }
        }
    }
    let sync = held
        .keys()
        .filter(|path| path.starts_with("vectors/sync/") && path.ends_with(".json"))
        .count();
    if Some(sync as u64) != manifest["sync"].as_u64() {
        departures.push(format!(
            "vectors/sync/: {sync} cases, listed {}",
            manifest["sync"]
        ));
    }

    assert!(
        departures.is_empty(),
        "spec/ differs from vectors/manifest.json, re-vendor it:\n{}",
        departures.join("\n")
    );
}

/// spec/ holds the specification's migrations/, queries/, vectors/ and
/// payloads/ at the revision spec/REVISION names, file by file and both
/// ways.
#[test]
fn the_vendored_copy_is_the_spec_byte_for_byte() {
    let revision = fs::read_to_string(spec_dir().join("REVISION")).unwrap();
    let revision = revision.trim();
    let checkout = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("pimdir");

    let Some(theirs) = revision_tree(&checkout, revision) else {
        eprintln!("no pimdir checkout holding {revision} beside this one: nothing compared");
        return;
    };
    let ours = vendored_tree();

    assert!(
        !theirs.is_empty(),
        "pimdir holds no vendored file at {revision}"
    );
    let differing: Vec<&PathBuf> = theirs
        .keys()
        .chain(ours.keys())
        .filter(|path| theirs.get(*path) != ours.get(*path))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    assert!(
        differing.is_empty(),
        "spec/ differs from pimdir at {revision}, re-vendor it: {differing:?}"
    );
}

/// Every canonical search statement the vendored copy carries, as
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
    let spec = spec_dir();

    let inlined: BTreeSet<&str> = sql::search::all().map(|(name, _)| name).collect();
    let store = fs::read_to_string(spec.join("migrations/storage/0001_init.sql")).unwrap();
    let schema = fs::read_to_string(spec.join("migrations/search/0001_init.sql")).unwrap();
    let (_dir, conn) = index(&store, &schema);

    for (name, sql) in search_statements(&spec) {
        assert!(
            inlined.contains(name.as_str()),
            "canonical search `{name}` has no constant here, unreachable by name"
        );
        if let Err(err) = prepare_all(&conn, &sql) {
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
            prepare_all(&conn, text).is_ok(),
            "`{name}` does not prepare against the inlined schemas: {:?}",
            prepare_all(&conn, text).unwrap_err()
        );
    }
}
