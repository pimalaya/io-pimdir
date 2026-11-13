//! The canonical statements the audit of 2026-11-12 rewrote for a large
//! collection (its lines P1 to P3), timed over a store of 60,000 mail
//! placements in one collection, built through the owner as a connector
//! lists them: `list_owing_bindings`, `list_superseded_bindings`,
//! `list_superseded_pending`, `collection_sources` and `count_unread`,
//! as vendored, on the linked SQLite.
//!
//! `PIMDIR_BEFORE` names a directory of statement files of an earlier
//! revision of the specification (`<name>.sql`), timed beside the
//! vendored ones over the same store, and compared row for row:
//!
//! ```sh
//! q=queries/storage/owner/list_owing_bindings.sql
//! git -C ../pimdir show 234bad2:$q > before/list_owing_bindings.sql
//! PIMDIR_BEFORE=before cargo test --release --test statement_scale \
//!     -- --ignored --nocapture
//! ```
//!
//! Ignored by default: it measures, and asserts only that both texts
//! answer the same rows. The numbers are indicative, one machine's.
//! `PIMDIR_SCALE` sets another number of messages.

#![cfg(feature = "client")]

use std::{collections::BTreeSet, env, fs, path::Path, time::Instant};

use io_pimdir::{
    change::PimdirWriteOp,
    client::{PimdirStore, reader::PimdirReader},
    collection::PimdirCollectionId,
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirPlacement, PimdirStatus,
    },
    sql,
    summary::{PimdirSummary, mail::PimdirMailSummary},
};
use rusqlite::{Connection, types::Value};

const COLLECTION: &str = "imap/INBOX";

/// Files `n` messages in the collection as the `imap` source lists them,
/// at the `Meta` tier, a thousand per write: a third read, a seventh
/// flagged, half carrying a keyword and one in thirteen another, one in
/// eleven with flags never read.
fn seed(dir: &Path, n: usize) {
    let mut store = PimdirStore::open(dir).unwrap().for_source("imap");
    store
        .ensure_collection(COLLECTION, "message/rfc822")
        .unwrap();
    let indexes: Vec<usize> = (0..n).collect();
    for batch in indexes.chunks(1000) {
        let ops = batch
            .iter()
            .map(|i| {
                let mut flags = BTreeSet::new();
                if i % 3 == 0 {
                    flags.insert(String::from("\\Seen"));
                }
                if i % 7 == 0 {
                    flags.insert(String::from("\\Flagged"));
                }
                if i % 2 == 0 {
                    flags.insert(String::from("work"));
                }
                if i % 13 == 0 {
                    flags.insert(String::from("$label1"));
                }
                let flags = match i % 11 {
                    0 => PimdirFlags::Unknown,
                    _ => PimdirFlags::Known(flags),
                };
                let summary = PimdirMailSummary {
                    message_id: Some(format!("m{i}@x.test")),
                    subject: format!("subject {i}"),
                    sender: Some(format!("user{}@example.org", i % 5)),
                    date: Some(format!(
                        "2026-{:02}-{:02}T{:02}:{:02}:00Z",
                        1 + i % 12,
                        1 + i % 28,
                        i % 24,
                        i % 60
                    )),
                    size: Some(100 + *i as u64),
                    attachment: Some(i % 2 == 0),
                    ..Default::default()
                };
                PimdirWriteOp::UpsertPlacement(PimdirPlacement {
                    collection: PimdirCollectionId(String::from(COLLECTION)),
                    handle: PimdirHandle(format!("h{i:06}")),
                    link_id: Some(summary.link_id()),
                    object: None,
                    level: PimdirLevel::Meta,
                    sort_key: summary.sort_key(),
                    summary: Some(PimdirSummary::Mail(summary)),
                    flags: flags.clone(),
                    status: PimdirStatus::Clean,
                    conflict_revision: None,
                    conflict_object: None,
                    base: Some(PimdirBase {
                        flags,
                        revision: None,
                        object: None,
                    }),
                    origin: None,
                })
            })
            .collect();
        store.write(ops).unwrap();
    }
}

/// Runs `text` with the parameters it names, answering its rows and the
/// median of `runs` timings in milliseconds.
fn timed(conn: &Connection, text: &str, runs: usize) -> (Vec<Vec<Value>>, f64) {
    let mut statement = conn.prepare(text).unwrap();
    let columns = statement.column_count();
    let bound: Vec<(String, Option<&str>)> = (1..=statement.parameter_count())
        .map(|at| {
            let name = statement.parameter_name(at).unwrap().to_string();
            let value = match name.as_str() {
                ":collection" => Some(COLLECTION),
                ":source" => Some("imap"),
                _ => None,
            };
            (name, value)
        })
        .collect();

    let mut rows = Vec::new();
    let mut timings = Vec::new();
    for _ in 0..runs {
        let params: Vec<(&str, &dyn rusqlite::ToSql)> = bound
            .iter()
            .map(|(name, value)| (name.as_str(), value as &dyn rusqlite::ToSql))
            .collect();
        let started = Instant::now();
        rows = statement
            .query_map(params.as_slice(), |row| {
                (0..columns).map(|at| row.get::<_, Value>(at)).collect()
            })
            .unwrap()
            .collect::<Result<Vec<Vec<Value>>, _>>()
            .unwrap();
        timings.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    timings.sort_by(f64::total_cmp);
    (rows, timings[timings.len() / 2])
}

#[test]
#[ignore = "a measurement: run with --release -- --ignored --nocapture"]
fn the_rewritten_statements_at_sixty_thousand_placements() {
    let n: usize = env::var("PIMDIR_SCALE")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(60_000);
    let before = env::var_os("PIMDIR_BEFORE");
    let dir = tempfile::tempdir().unwrap();

    let started = Instant::now();
    seed(dir.path(), n);
    println!(
        "{n} placements written in {:.1} s",
        started.elapsed().as_secs_f64()
    );

    let reader = PimdirReader::open(dir.path()).unwrap();
    drop(reader);
    let conn = Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch(sql::CREATE_PARAM).unwrap();
    conn.execute(sql::CLEAR_PARAM, []).unwrap();
    conn.execute(
        sql::BIND_PARAM,
        rusqlite::named_params! { ":slot": "collections", ":position": 0, ":value": COLLECTION },
    )
    .unwrap();
    let version: String = conn
        .query_row("SELECT sqlite_version()", [], |r| r.get(0))
        .unwrap();
    println!("SQLite {version}");

    let statements = [
        ("list_owing_bindings", sql::LIST_OWING_BINDINGS),
        ("list_superseded_bindings", sql::LIST_SUPERSEDED_BINDINGS),
        ("list_superseded_pending", sql::LIST_SUPERSEDED_PENDING),
        ("collection_sources", sql::COLLECTION_SOURCES),
        ("count_unread", sql::COUNT_UNREAD),
    ];
    for (name, vendored) in statements {
        let (rows, after) = timed(&conn, vendored, 9);
        let earlier = before
            .as_ref()
            .map(|dir| Path::new(dir).join(format!("{name}.sql")))
            .filter(|path| path.is_file())
            .map(|path| fs::read_to_string(path).unwrap());
        match earlier {
            Some(text) => {
                let (earlier_rows, earlier) = timed(&conn, &text, 9);
                assert_eq!(earlier_rows, rows, "{name}: the two texts answer alike");
                println!(
                    "{name}: {earlier:.3} ms before, {after:.3} ms vendored, {} row(s)",
                    rows.len()
                );
            }
            None => println!("{name}: {after:.3} ms vendored, {} row(s)", rows.len()),
        }
    }
}
