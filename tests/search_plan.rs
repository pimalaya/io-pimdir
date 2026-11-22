//! The query planner on a synthetic store of a few thousand messages in
//! three collections of two accounts, some filed twice: every driver
//! answers each query alike, page by page in both dated orders, and the
//! statements a plan runs seek where a seek is the point, as their query
//! plans show (EXPLAIN QUERY PLAN), never scanning the store's items.
//!
//! One test, so the statements its trace log captures are its own.

#![cfg(all(feature = "search", feature = "indexer"))]

use std::{collections::BTreeSet, path::Path, sync::Mutex};

use io_pimdir::{
    change::PimdirWriteOp,
    client::{
        PimdirStore,
        search::{
            PimdirIndex, PimdirIndexRefreshOptions, PimdirSearchClock, PimdirSearchOptions,
            PimdirSearchSort,
        },
    },
    collection::PimdirCollectionId,
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirPlacement, PimdirStatus,
    },
    search::{plan::PimdirPlanForce, query::PimdirQuery},
    summary::{PimdirAddress, PimdirSummary, mail::PimdirMailSummary},
};
use log::{LevelFilter, Log, Metadata, Record};

/// The statements the search runs, as its trace log prints them.
static STATEMENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Held by each test of this binary: the log is the process's, so one
/// test's statements must not land among another's.
static SERIAL: Mutex<()> = Mutex::new(());

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

/// The collections, each with its account and how many messages it
/// holds.
const COLLECTIONS: [(&str, &str, usize); 3] = [
    ("home/INBOX", "home", 1_500),
    ("home/Archive", "home", 1_000),
    ("work/INBOX", "work", 500),
];

/// Message `i` of a collection: one every hour from 2026-01-01, a third
/// read, a seventh flagged, five senders, one in six hundred about kiwi.
/// Every tenth of home/Archive is home/INBOX's message `i` filed twice,
/// one hit of two placements; work/INBOX holds the first fifty of
/// home/INBOX's, hits of the same `seq` in another account.
fn mail(collection: &str, i: usize) -> (String, PimdirFlags, PimdirMailSummary) {
    let key = match collection {
        "home/INBOX" => format!("in{i}"),
        "home/Archive" if i.is_multiple_of(10) => format!("in{i}"),
        "work/INBOX" if i < 50 => format!("in{i}"),
        _ => format!("{collection}/{i}"),
    };
    let at = 1_767_225_600 + i as i64 * 3_600;
    let mut flags = BTreeSet::new();
    if i.is_multiple_of(3) {
        flags.insert(String::from("\\Seen"));
    }
    if i.is_multiple_of(7) {
        flags.insert(String::from("\\Flagged"));
    }
    let from = PimdirAddress {
        address: format!("user{}@example.org", i % 5),
        name: None,
    };
    let subject = match i % 600 {
        17 => format!("kiwi season {i}"),
        _ => format!("report {i}"),
    };
    let summary = PimdirMailSummary {
        message_id: Some(format!("{key}@x.test")),
        subject,
        sender: Some(from.address.clone()),
        date: Some(rfc3339(at)),
        from: vec![from],
        ..Default::default()
    };
    (key, PimdirFlags::Known(flags), summary)
}

/// An RFC 3339 instant of whole hours after 2026-01-01, a Unix time.
fn rfc3339(at: i64) -> String {
    let hours = (at - 1_767_225_600) / 3_600;
    let (days, hour) = (hours / 24, hours % 24);
    let (month, day) = match days {
        days if days < 31 => (1, days + 1),
        days if days < 59 => (2, days - 30),
        days if days < 90 => (3, days - 58),
        days if days < 120 => (4, days - 89),
        days => (5, days - 119),
    };
    format!("2026-{month:02}-{day:02}T{hour:02}:00:00Z")
}

/// Files every collection's messages as the `imap` source lists them.
fn seed(dir: &Path) {
    for (id, account, count) in COLLECTIONS {
        let owner = PimdirStore::open(dir).unwrap().for_account(account);
        owner.ensure_collection(id, "message/rfc822").unwrap();
        drop(owner);
        let mut store = PimdirStore::open(dir).unwrap().for_source("imap");
        let ops = (0..count)
            .map(|i| {
                let (handle, flags, summary) = mail(id, i);
                PimdirWriteOp::UpsertPlacement(PimdirPlacement {
                    collection: PimdirCollectionId(id.into()),
                    handle: PimdirHandle(handle),
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
                    server_object_id: None,
                })
            })
            .collect();
        store.write(ops).unwrap();
    }
}

/// The query plan of a statement, its parameters bound `NULL`, one line
/// per step.
fn plan(dir: &Path, sql: &str) -> Vec<String> {
    let conn = rusqlite::Connection::open(dir.join("index.db")).unwrap();
    let store = dir.join("pimdir.db");
    conn.execute("ATTACH DATABASE ?1 AS store", [store.to_str().unwrap()])
        .unwrap();
    conn.execute_batch(
        "CREATE TEMP TABLE term_match (term INTEGER NOT NULL, collection TEXT NOT NULL, \
         akey TEXT NOT NULL, seq INTEGER NOT NULL, src TEXT NOT NULL, rank REAL); \
         CREATE INDEX temp.term_match_by_term ON term_match(term, akey, seq); \
         CREATE TEMP TABLE term_source (term INTEGER NOT NULL, src INTEGER NOT NULL, \
         id INTEGER NOT NULL, PRIMARY KEY (term, src, id)) WITHOUT ROWID; \
         CREATE TEMP TABLE hit_page (akey TEXT NOT NULL, seq INTEGER NOT NULL);",
    )
    .unwrap();
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let nulls = vec![rusqlite::types::Null; stmt.parameter_count()];
    stmt.query_map(rusqlite::params_from_iter(nulls), |row| row.get(3))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// The statements one call runs, each with its plan.
fn statements(dir: &Path, run: impl FnOnce()) -> Vec<(String, Vec<String>)> {
    STATEMENTS.lock().unwrap().clear();
    run();
    let sqls: Vec<String> = STATEMENTS.lock().unwrap().drain(..).collect();
    sqls.into_iter()
        .map(|sql| {
            let plan = plan(dir, &sql);
            (sql, plan)
        })
        .collect()
}

/// Whether a plan step reads every row of the store's items, or of one
/// of their indexes whole: a `SCAN` of an alias the compiler gives them,
/// but of `items_unread`, which holds the unread rows alone, the set
/// `unread` answers (SEARCH §8).
fn scans_items(step: &str) -> bool {
    let Some(rest) = step.strip_prefix("SCAN ") else {
        return false;
    };
    if rest.ends_with("INDEX items_unread") {
        return false;
    }
    let alias = rest.split_whitespace().next().unwrap_or_default();
    ["y", "i", "s", "r", "ei", "items"].contains(&alias)
        || (alias.starts_with('j') && alias[1..].bytes().all(|b| b.is_ascii_digit()))
}

/// Asserts that no statement of `ran` scans the store's items.
fn seeks(query: &str, ran: &[(String, Vec<String>)]) {
    for (sql, plan) in ran {
        assert!(
            !plan.iter().any(|step| scans_items(step)),
            "{query}: a statement scans the items\n{sql}\n{}",
            plan.join("\n")
        );
    }
}

/// Every hit of `query` in `sort`, walked page by page of `limit`.
fn walked(
    index: &PimdirIndex,
    query: &PimdirQuery,
    sort: PimdirSearchSort,
    limit: usize,
) -> Vec<(String, i64)> {
    let mut options = PimdirSearchOptions {
        sort,
        limit,
        clock: PimdirSearchClock::default(),
        ..Default::default()
    };
    let mut hits = Vec::new();
    loop {
        let answer = index.search(query, &options).unwrap();
        hits.extend(
            answer
                .hits
                .iter()
                .map(|hit| (hit.account.clone().unwrap_or_default(), hit.seq)),
        );
        match answer.next {
            Some(next) => options.after = Some(next),
            None => return hits,
        }
    }
}

#[test]
fn every_driver_answers_alike_and_seeks() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    static LOGGER: Statements = Statements;
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(LevelFilter::Trace);

    let dir = tempfile::tempdir().unwrap();
    seed(dir.path());
    let mut index = PimdirIndex::open(dir.path()).unwrap();
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    std::fs::remove_file(dir.path().join("index.hits")).unwrap();

    let queries = [
        "kind:mail",
        "is:unread",
        "kiwi",
        "folder:home/INBOX",
        "from:user1@example.org",
        "kiwi or is:flagged",
        "account:home is:flagged",
        "report not is:read",
        "folder:home/Archive folder:home/INBOX",
    ];
    for query in queries {
        let parsed = PimdirQuery::parse(query).unwrap();
        let count = index
            .count(&parsed, &PimdirSearchClock::default())
            .unwrap()
            .count;
        for sort in [PimdirSearchSort::NewestFirst, PimdirSearchSort::OldestFirst] {
            let whole = walked(&index, &parsed, sort, 10_000);
            assert_eq!(whole.len() as u64, count, "{query}: count");
            let unique: BTreeSet<&(String, i64)> = whole.iter().collect();
            assert_eq!(unique.len(), whole.len(), "{query}: each hit once");
            for force in [
                None,
                Some(PimdirPlanForce::Set),
                Some(PimdirPlanForce::Walk),
                Some(PimdirPlanForce::Scope),
                Some(PimdirPlanForce::Fallback),
            ] {
                index.force_driver(force);
                let paged = walked(&index, &parsed, sort, 97);
                let counted = index
                    .count(&parsed, &PimdirSearchClock::default())
                    .unwrap()
                    .count;
                index.force_driver(None);
                assert_eq!(paged, whole, "{query} {sort:?} under {force:?}");
                assert_eq!(counted, count, "{query} count under {force:?}");
            }
        }
    }

    let page = PimdirSearchOptions::default();
    let clock = PimdirSearchClock::default();
    let mut shown = Vec::new();
    for (query, count) in [
        ("kiwi", true),
        ("kiwi", false),
        ("kind:mail", false),
        ("folder:home/INBOX", false),
        ("from:user1@example.org", true),
        ("from:user1@example.org", false),
        ("is:unread", true),
        ("is:unread", false),
        ("-is:flagged", true),
    ] {
        let parsed = PimdirQuery::parse(query).unwrap();
        let ran = statements(dir.path(), || match count {
            true => drop(index.count(&parsed, &clock).unwrap()),
            false => drop(index.search(&parsed, &page).unwrap()),
        });
        if query != "-is:flagged" {
            seeks(query, &ran);
        }
        for (sql, plan) in &ran {
            if sql.contains("INDEXED BY items_by_sort") {
                assert!(
                    plan.iter().any(|step| step.starts_with("SEARCH y USING")
                        && step.contains("items_by_sort (collection=?")),
                    "{query}: a step seeks its collection\n{}",
                    plan.join("\n")
                );
            }
        }
        shown.push((query, count, ran));
    }

    let walks = |query: &str| {
        shown
            .iter()
            .find(|(shown, count, _)| *shown == query && !count)
            .map(|(_, _, ran)| {
                ran.iter()
                    .filter(|(sql, _)| sql.contains("INDEXED BY items_by_sort"))
                    .count()
            })
            .unwrap()
    };
    assert!(
        (1..=6).contains(&walks("kind:mail")),
        "page 1 of all mail steps a few collections, not every row"
    );
    assert!(walks("folder:home/INBOX") >= 1, "one collection walked");
    assert_eq!(walks("kiwi"), 0, "five hits read as a set");

    let ran = |query: &str| {
        shown
            .iter()
            .find(|(shown, count, _)| *shown == query && *count)
            .map(|(_, _, ran)| ran)
            .unwrap()
    };
    let (_, plan) = ran("-is:flagged").last().unwrap();
    assert!(
        plan.iter()
            .any(|step| step.contains("y USING COVERING INDEX items_by_sort")),
        "a scope count reads the items by their sort index alone\n{}",
        plan.join("\n")
    );
    let (_, plan) = ran("is:unread").last().unwrap();
    assert!(
        plan.iter()
            .any(|step| step.contains("y USING COVERING INDEX items_by_sort")),
        "an unread count over a store two thirds unread passes over the \
         items by their sort index alone\n{}",
        plan.join("\n")
    );

    if std::env::var_os("PIMDIR_PLAN").is_some() {
        for (query, count, ran) in &shown {
            let what = if *count { "count" } else { "page 1" };
            for (sql, plan) in ran {
                eprintln!("-- {query} {what}\n{sql}\n{}\n", plan.join("\n"));
            }
        }
    }
}

/// The hit columns (index.hits) answer every count, every page walked to
/// its end in both dated orders and every first page of threads as
/// index.db alone does, over hits of two placements, two accounts, flags
/// and dates, and their terms read off each hit where few remain.
#[test]
fn the_hit_columns_answer_as_the_index_alone() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path());
    let mut index = PimdirIndex::open(dir.path()).unwrap();
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    assert!(dir.path().join("index.hits").exists());

    let queries = [
        "kiwi",
        "report",
        "report not kiwi",
        "is:flagged",
        "is:unread",
        "is:unread report",
        "is:flagged kiwi",
        "kiwi not is:read",
        "kiwi or is:flagged",
        "date:2026-02-01..2026-02-28",
        "date:2026-02-01..2026-02-28 report",
        "kiwi date:2026-01-01..2026-01-31",
        "kiwi date:2026-02-01..2026-05-31 is:flagged",
        "kind:mail is:flagged",
        "account:home is:flagged",
        "account:work kiwi",
        "from:user1 kiwi",
        "from:user1",
        "from:use*",
        "rep* not is:read",
        "kiwi season",
        "from:user1@example.org",
        "from:User2@Example.org kiwi",
        "with:user3@example.org not is:flagged",
        "to:user1@example.org",
        "account:work from:user4@example.org",
        "from:user1@example.org or from:user2@example.org",
    ];
    let answers = |index: &PimdirIndex| {
        let clock = PimdirSearchClock::default();
        queries
            .iter()
            .map(|query| {
                let parsed = PimdirQuery::parse(query).unwrap();
                let count = index.count(&parsed, &clock).unwrap().count;
                let newest = walked(index, &parsed, PimdirSearchSort::NewestFirst, 7);
                let oldest = walked(index, &parsed, PimdirSearchSort::OldestFirst, 7);
                let options = PimdirSearchOptions {
                    limit: 5,
                    ..Default::default()
                };
                let threads: Vec<Vec<(String, i64)>> = index
                    .threads(&parsed, &options)
                    .unwrap()
                    .threads
                    .iter()
                    .map(|thread| {
                        thread
                            .hits
                            .iter()
                            .map(|hit| (hit.account.clone().unwrap_or_default(), hit.seq))
                            .collect()
                    })
                    .collect();
                (query, count, newest, oldest, threads)
            })
            .collect::<Vec<_>>()
    };
    let columned = answers(&index);
    std::fs::remove_file(dir.path().join("index.hits")).unwrap();
    let plain = answers(&PimdirIndex::open(dir.path()).unwrap());
    for (columned, plain) in columned.iter().zip(&plain) {
        assert_eq!(columned, plain);
    }
    assert!(plain.iter().any(|(_, count, ..)| *count > 0));
}

/// A rebuilt index.db takes the hit columns with it: the filesystem may
/// give the new file the old one's inode, and a rebuild from an unchanged
/// store reaches the same change, purge and id counts the cache's stamp
/// names, so a cache left behind would read as the new index's.
#[test]
fn a_rebuild_drops_the_hit_columns() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path());
    PimdirIndex::open(dir.path())
        .unwrap()
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    let hits = dir.path().join("index.hits");
    assert!(hits.exists());

    rusqlite::Connection::open(dir.path().join("index.db"))
        .unwrap()
        .execute_batch("DROP TABLE pimdir_fields;")
        .unwrap();
    drop(PimdirIndex::open(dir.path()).unwrap());
    assert!(!hits.exists());
}
