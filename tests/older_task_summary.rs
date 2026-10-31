//! A store whose `task_summary` predates the task columns (`priority`,
//! `parent`, `mid`) and holds no `task_category`, as an owner at io-pimdir
//! 3b4bed8 writes and reconciles it, is read with those columns `NULL` and
//! no categories (STORAGE §6): a reader never fails a listing on a column
//! the owner has not added yet.

#![cfg(feature = "client")]

use io_pimdir::{
    client::{PimdirStore, reader::PimdirReader},
    summary::{PimdirSummary, calendar::PimdirTaskSummary},
};

/// A task stating every column the older shape lacks.
const TASK: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\n\
BEGIN:VTODO\r\nUID:todo-1@example.org\r\nDTSTAMP:20261028T090000Z\r\n\
SUMMARY:Call the plumber\r\nPRIORITY:1\r\nCATEGORIES:inbox,home\r\n\
RELATED-TO:project-1@example.org\r\nATTACH:mid:msg-1@example.org\r\n\
END:VTODO\r\nEND:VCALENDAR\r\n";

/// A second task, deleted, for the trash view.
const GONE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\n\
BEGIN:VTODO\r\nUID:todo-2@example.org\r\nDTSTAMP:20261028T090000Z\r\n\
SUMMARY:Old errand\r\nPRIORITY:5\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";

/// The `task_summary` of io-pimdir 3b4bed8, verbatim.
const OLDER: &str = "CREATE TABLE task_summary (
    collection    TEXT NOT NULL,
    link_id       TEXT NOT NULL,
    uid           TEXT,
    summary       TEXT NOT NULL,
    dtstart       TEXT,
    dtstart_tzid  TEXT,
    dtstart_value TEXT,
    due           TEXT,                    -- the DUE value verbatim, or NULL
    due_tzid      TEXT,
    due_value     TEXT,
    status        TEXT,                    -- STATUS uppercased verbatim, or NULL
    completed     TEXT,                    -- COMPLETED verbatim (always UTC per RFC 5545), or NULL
    percent       INTEGER,                 -- PERCENT-COMPLETE, or NULL
    recurring     INTEGER,
    until         TEXT,
    PRIMARY KEY (collection, link_id),
    FOREIGN KEY (collection, link_id) REFERENCES items(collection, link_id) ON UPDATE CASCADE ON DELETE CASCADE
) STRICT;";

/// A store holding [`TASK`] live and [`GONE`] in the trash, its
/// `task_summary` brought back to the older shape, the rows kept.
fn older_store() -> (tempfile::TempDir, i64, i64) {
    let dir = tempfile::tempdir().unwrap();
    let (live, gone) = {
        let mut store = PimdirStore::open(dir.path()).unwrap();
        store.ensure_collection("tasks", "text/calendar").unwrap();
        let (_, live) = store.put_item("tasks", TASK.as_bytes()).unwrap();
        let (_, gone) = store.put_item("tasks", GONE.as_bytes()).unwrap();
        (live, gone)
    };

    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch(&format!(
        "PRAGMA foreign_keys = OFF;
         DROP TABLE task_category;
         DROP INDEX task_summary_by_parent;
         DROP INDEX task_summary_by_mid;
         ALTER TABLE task_summary RENAME TO task_summary_aside;
         {OLDER}
         INSERT INTO task_summary SELECT collection, link_id, uid, summary,
             dtstart, dtstart_tzid, dtstart_value, due, due_tzid, due_value,
             status, completed, percent, recurring, until
         FROM task_summary_aside;
         DROP TABLE task_summary_aside;
         UPDATE items SET deleted = 1, retained_at = '2026-10-28T09:00:00Z'
         WHERE collection = 'tasks' AND seq = {gone};
         PRAGMA foreign_keys = ON;"
    ))
    .unwrap();
    let priority: i64 = conn
        .query_row(
            "SELECT count(*) FROM pragma_table_info('task_summary') WHERE name = 'priority'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(priority, 0, "the older shape holds no priority");
    drop(conn);

    (dir, live, gone)
}

fn task(summary: Option<PimdirSummary>) -> PimdirTaskSummary {
    match summary {
        Some(PimdirSummary::Task(task)) => task,
        other => panic!("a task summary, got {other:?}"),
    }
}

#[test]
fn a_reader_reads_the_missing_task_columns_as_null() {
    let (dir, live, gone) = older_store();
    let reader = PimdirReader::open(dir.path()).unwrap();

    let page = reader.list_summaries("tasks", None, 10).unwrap();
    assert_eq!(page.len(), 1);
    let listed = task(page.into_iter().next().unwrap().summary);
    assert_eq!(listed.summary, "Call the plumber");
    assert_eq!(listed.priority, None);
    assert!(listed.categories.is_empty());
    assert_eq!(listed.parent, None);
    assert_eq!(listed.mid, None);

    let one = task(reader.get_item("tasks", live).unwrap().unwrap().summary);
    assert_eq!(one, listed, "one item reads as its listing");

    let trash = reader.list_retained("tasks", None, 10).unwrap();
    assert_eq!(trash.len(), 1);
    assert_eq!(trash[0].seq, gone);
    let retained = task(trash.into_iter().next().unwrap().summary);
    assert_eq!(retained.summary, "Old errand");
    assert_eq!(retained.priority, None);
}

/// The owner, opening the same store, reconciles the table and re-derives
/// the columns from the held bodies (§6): the reader then reads them.
#[test]
fn the_owner_backfills_what_the_reader_read_as_null() {
    let (dir, live, _) = older_store();
    drop(PimdirStore::open(dir.path()).unwrap());

    let reader = PimdirReader::open(dir.path()).unwrap();
    let one = task(reader.get_item("tasks", live).unwrap().unwrap().summary);
    assert_eq!(one.priority, Some(1));
    assert_eq!(one.categories, ["inbox", "home"]);
    assert_eq!(one.parent.as_deref(), Some("project-1@example.org"));
    assert_eq!(one.mid.as_deref(), Some("msg-1@example.org"));
}

/// A query client attaches the same store: a task term reads the missing
/// columns as `NULL` too, so no task carries a category or a priority and
/// every one has none (SEARCH §8, STORAGE §6).
#[cfg(feature = "search")]
#[test]
fn a_query_reads_the_missing_task_columns_as_null() {
    use io_pimdir::{
        client::search::{PimdirIndex, PimdirIndexRefreshOptions, PimdirSearchOptions},
        search::query::PimdirQuery,
    };

    let (dir, live, _) = older_store();
    let mut index = PimdirIndex::open(dir.path()).unwrap();
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    let hits = |query: &str| -> Vec<i64> {
        index
            .search(
                &PimdirQuery::parse(query).unwrap(),
                &PimdirSearchOptions::default(),
            )
            .unwrap()
            .hits
            .iter()
            .map(|hit| hit.seq)
            .collect()
    };

    assert_eq!(hits("kind:task"), [live]);
    assert!(hits("kind:task category:inbox").is_empty());
    assert_eq!(hits("kind:task -category:inbox"), [live]);
    assert!(hits("kind:task priority:high").is_empty());
    assert_eq!(hits("kind:task priority:none"), [live]);
}

/// An item whose body is gone is read from its summary (SEARCH §6), the
/// missing task columns `NULL` there too.
#[cfg(feature = "search")]
#[test]
fn an_item_without_its_body_reads_its_summary_with_the_columns_null() {
    use io_pimdir::client::search::{PimdirIndex, PimdirIndexRefreshOptions, PimdirSearchMatched};

    let (dir, live, _) = older_store();
    let mut index = PimdirIndex::open(dir.path()).unwrap();
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    let hash = index
        .reader()
        .get_item("tasks", live)
        .unwrap()
        .unwrap()
        .object
        .unwrap();
    std::fs::remove_file(index.reader().blobs().path(&hash)).unwrap();

    let item = index.item(None, live).unwrap().unwrap();
    assert_eq!(item.seq, live);
    assert_eq!(item.source, PimdirSearchMatched::Summary);
}
