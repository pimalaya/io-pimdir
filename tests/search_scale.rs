//! The search index at scale: a store of 50,000 summary-only messages,
//! built through the owner as a mail connector lists them, indexed whole,
//! then asked the queries a mail client asks most, a page of 50 and a
//! count each, with the query plan of each compiled statement.
//!
//! Ignored by default: it takes minutes in a debug build.
//!
//! ```sh
//! cargo test --release --features search --test search_scale -- --ignored --nocapture
//! ```
//!
//! `PIMDIR_SCALE` sets another number of messages.

#![cfg(all(feature = "client", feature = "search"))]

use std::{
    collections::BTreeSet,
    path::Path,
    sync::Mutex,
    time::{Duration, Instant},
};

use io_pimdir::{
    change::PimdirWriteOp,
    client::{
        PimdirStore,
        search::{PimdirIndex, PimdirIndexRefreshOptions, PimdirSearchClock, PimdirSearchOptions},
    },
    collection::PimdirCollectionId,
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirPlacement, PimdirStatus,
    },
    search::query::PimdirQuery,
    summary::{PimdirAddress, PimdirSummary, mail::PimdirMailSummary},
};
use log::{LevelFilter, Log, Metadata, Record};

/// The statements the search compiles, as its trace log prints them.
static STATEMENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Statements;

impl Log for Statements {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.target().starts_with("io_pimdir")
    }

    fn log(&self, record: &Record) {
        let message = record.args().to_string();
        if let Some(sql) = message.strip_prefix("search statement: ") {
            STATEMENTS.lock().unwrap().push(sql.to_string());
        }
    }

    fn flush(&self) {}
}

/// `n` messages, one every ten minutes from 2026-01-01, a
/// third read, a seventh flagged, one in eleven with flags never read,
/// five senders, four subjects.
fn mails(n: usize) -> Vec<(String, PimdirFlags, PimdirMailSummary)> {
    let words = ["release", "Invoice", "météo", "hello"];
    (0..n)
        .map(|i| {
            let at = 1_767_225_600 + (i as i64) * 600;
            let (days, secs) = (at.div_euclid(86_400), at.rem_euclid(86_400));
            let date = rfc3339(days, secs);
            let mut flags = BTreeSet::new();
            if i % 3 == 0 {
                flags.insert(String::from("\\Seen"));
            }
            if i % 7 == 0 {
                flags.insert(String::from("\\Flagged"));
            }
            let flags = match i % 11 {
                0 => PimdirFlags::Unknown,
                _ => PimdirFlags::Known(flags),
            };
            let from = PimdirAddress {
                address: format!("user{}@example.org", i % 5),
                name: (i % 2 == 0).then(|| String::from("Alice Martin")),
            };
            let summary = PimdirMailSummary {
                message_id: Some(format!("m{i}@x.test")),
                subject: format!("{} {i}", words[i % words.len()]),
                sender: Some(from.address.clone()),
                sender_name: from.name.clone(),
                date: Some(date),
                size: Some(100 + i as u64),
                attachment: Some(i % 2 == 0),
                from: vec![from],
                to: vec![PimdirAddress {
                    address: String::from("team@example.org"),
                    name: None,
                }],
                ..Default::default()
            };
            (format!("h{i}"), flags, summary)
        })
        .collect()
}

/// An RFC 3339 instant from days since the epoch and seconds in the day.
fn rfc3339(days: i64, secs: i64) -> String {
    // NOTE: Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

/// Files the messages in `imap/INBOX` as the `imap` source lists them, at
/// the `Meta` tier, a thousand per write.
fn seed(dir: &Path, n: usize) {
    let mut store = PimdirStore::open(dir).unwrap().for_source("imap");
    store
        .ensure_collection("imap/INBOX", "message/rfc822")
        .unwrap();
    for batch in mails(n).chunks(1000) {
        let ops = batch
            .iter()
            .map(|(handle, flags, summary)| {
                PimdirWriteOp::UpsertPlacement(PimdirPlacement {
                    collection: PimdirCollectionId(String::from("imap/INBOX")),
                    handle: PimdirHandle(handle.clone()),
                    link_id: Some(summary.link_id()),
                    object: None,
                    level: PimdirLevel::Meta,
                    sort_key: summary.sort_key(),
                    summary: Some(PimdirSummary::Mail(summary.clone())),
                    flags: flags.clone(),
                    status: PimdirStatus::Clean,
                    conflict_revision: None,
                    conflict_object: None,
                    base: Some(PimdirBase {
                        flags: flags.clone(),
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

fn median(mut run: impl FnMut() -> usize) -> (Duration, usize) {
    let mut answer = 0;
    let mut times: Vec<Duration> = (0..3)
        .map(|_| {
            let at = Instant::now();
            answer = std::hint::black_box(run());
            at.elapsed()
        })
        .collect();
    times.sort();
    (times[1], answer)
}

/// The query plan of a compiled statement, its parameters bound `NULL`.
fn plan(dir: &Path, sql: &str) -> String {
    let conn = rusqlite::Connection::open(dir.join("index.db")).unwrap();
    let store = dir.join("pimdir.db");
    conn.execute("ATTACH DATABASE ?1 AS store", [store.to_str().unwrap()])
        .unwrap();
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let nulls = vec![rusqlite::types::Null; stmt.parameter_count()];
    let rows: Vec<(i64, i64, String)> = stmt
        .query_map(rusqlite::params_from_iter(nulls), |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(3)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let mut depth: Vec<(i64, usize)> = Vec::new();
    let mut out = String::new();
    for (id, parent, detail) in rows {
        while depth.last().is_some_and(|(at, _)| *at != parent) {
            depth.pop();
        }
        let level = depth.last().map_or(0, |(_, level)| level + 1);
        out += &format!("{}{detail}\n", "  ".repeat(level));
        depth.push((id, level));
    }
    out
}

#[test]
#[ignore = "seeds 50,000 messages"]
fn fifty_thousand_summaries_index_and_answer() {
    static LOGGER: Statements = Statements;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(LevelFilter::Trace);

    let n = std::env::var("PIMDIR_SCALE")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(50_000);
    let dir = tempfile::tempdir().unwrap();

    let at = Instant::now();
    seed(dir.path(), n);
    eprintln!("seeded {n} messages in {:?}", at.elapsed());

    let at = Instant::now();
    let report = PimdirIndex::open(dir.path())
        .unwrap()
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    eprintln!("indexed {} rows in {:?}", report.folded, at.elapsed());
    assert_eq!(report.folded, n as u64);

    let index = PimdirIndex::open_query(dir.path()).unwrap();
    let options = PimdirSearchOptions {
        clock: PimdirSearchClock {
            now: Some(1_798_761_600),
        },
        ..Default::default()
    };
    for query in [
        "is:unread",
        "not is:unread",
        "is:flagged",
        "date:2026-08-01..",
        "from:user1",
        "subject:release",
        "is:unread from:user1",
    ] {
        let parsed = PimdirQuery::parse(query).unwrap();
        STATEMENTS.lock().unwrap().clear();
        let (page, hits) = median(|| index.search(&parsed, &options).unwrap().hits.len());
        let sql = STATEMENTS.lock().unwrap().first().cloned().unwrap();
        let (count, total) =
            median(|| index.count(&parsed, &options.clock).unwrap().count as usize);
        eprintln!("`{query}`: page of {hits} in {page:?}, count {total} in {count:?}");
        if std::env::var_os("PIMDIR_PLAN").is_some() {
            eprintln!("{sql}\n{}", plan(dir.path(), &sql));
        }
    }
}
