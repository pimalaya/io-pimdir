//! A store derived under Annex A revision 1, its `task_summary` of the
//! storage-draft-05 shape (no `created`) and no `contact` address, is read
//! with the capture `NULL` and no contact, then re-derived by its owner on
//! open from the held bodies: `created`, the `contact` rows, revision 2
//! recorded (STORAGE §6, Annex A.4).

use io_pimdir::{
    client::{PimdirStore, reader::PimdirReader},
    summary::{PimdirSummary, calendar::PimdirTaskSummary},
};
use rusqlite::Connection;

/// A task captured before its last stamp, waiting on a landlord.
const TASK: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\n\
BEGIN:VTODO\r\nUID:todo-1@example.org\r\nDTSTAMP:20261029T170000Z\r\n\
CREATED:20261028T090000Z\r\nSUMMARY:Lease signed\r\nCATEGORIES:waiting\r\n\
CONTACT;ALTREP=\"mailto:Landlord@Example.org\":Mr Landlord\r\n\
END:VTODO\r\nEND:VCALENDAR\r\n";

/// The `task_summary` of storage-draft-05, verbatim but for its comments.
const DRAFT_05: &str = "CREATE TABLE task_summary (
    collection    TEXT NOT NULL,
    link_id       TEXT NOT NULL,
    uid           TEXT,
    summary       TEXT NOT NULL,
    dtstart       TEXT,
    dtstart_tzid  TEXT,
    dtstart_value TEXT,
    due           TEXT,
    due_tzid      TEXT,
    due_value     TEXT,
    status        TEXT,
    completed     TEXT,
    percent       INTEGER,
    recurring     INTEGER,
    until         TEXT,
    priority      INTEGER CHECK (priority BETWEEN 0 AND 9),
    categories    TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(categories)),
    parent        TEXT,
    mid           TEXT,
    PRIMARY KEY (collection, link_id),
    FOREIGN KEY (collection, link_id) REFERENCES items(collection, link_id) ON UPDATE CASCADE ON DELETE CASCADE
) STRICT;";

/// A store holding [`TASK`], brought back to what a revision 1 owner at
/// storage-draft-05 left: the column gone, the contact row gone, the
/// derivation 1. Answers the task's seq.
fn revision_1_store() -> (tempfile::TempDir, i64) {
    let dir = tempfile::tempdir().unwrap();
    let seq = {
        let mut store = PimdirStore::open(dir.path()).unwrap();
        store.ensure_collection("tasks", "text/calendar").unwrap();
        store.put_item("tasks", TASK.as_bytes()).unwrap().1
    };

    let conn = Connection::open(dir.path().join("pimdir.db")).unwrap();
    let indexes: Vec<String> = conn
        .prepare("SELECT sql FROM sqlite_schema WHERE type = 'index' AND tbl_name = 'task_summary' AND sql IS NOT NULL")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    conn.execute_batch(&format!(
        "PRAGMA foreign_keys = OFF;
         ALTER TABLE task_summary RENAME TO task_summary_aside;
         {DRAFT_05}
         INSERT INTO task_summary SELECT collection, link_id, uid, summary,
             dtstart, dtstart_tzid, dtstart_value, due, due_tzid, due_value,
             status, completed, percent, recurring, until, priority,
             categories, parent, mid
         FROM task_summary_aside;
         DROP TABLE task_summary_aside;
         {};
         DELETE FROM item_address WHERE role = 'contact';
         UPDATE store_meta SET derivation = 1 WHERE id = 1;
         PRAGMA foreign_keys = ON;",
        indexes.join(";\n")
    ))
    .unwrap();
    drop(conn);

    (dir, seq)
}

fn task(summary: Option<PimdirSummary>) -> PimdirTaskSummary {
    match summary {
        Some(PimdirSummary::Task(task)) => task,
        other => panic!("a task summary, got {other:?}"),
    }
}

fn contacts(dir: &std::path::Path) -> Vec<(String, Option<String>)> {
    let conn = Connection::open(dir.join("pimdir.db")).unwrap();
    conn.prepare("SELECT address, name FROM item_address WHERE role = 'contact' ORDER BY position")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn a_reader_reads_a_revision_1_tasks_capture_as_null() {
    let (dir, seq) = revision_1_store();
    let reader = PimdirReader::open(dir.path()).unwrap();
    let read = task(reader.get_item("tasks", seq).unwrap().unwrap().summary);
    assert_eq!(read.summary, "Lease signed");
    assert_eq!(read.created, None);
    assert!(contacts(dir.path()).is_empty());
}

#[test]
fn the_owner_re_derives_a_revision_1_store_on_open() {
    let (dir, seq) = revision_1_store();
    let stamp = |dir: &std::path::Path| -> i64 {
        Connection::open(dir.join("pimdir.db"))
            .unwrap()
            .query_row("SELECT changed FROM items WHERE seq = ?1", [seq], |r| {
                r.get(0)
            })
            .unwrap()
    };
    let before = stamp(dir.path());

    drop(PimdirStore::open(dir.path()).unwrap());

    let reader = PimdirReader::open(dir.path()).unwrap();
    let read = task(reader.get_item("tasks", seq).unwrap().unwrap().summary);
    assert_eq!(read.created.as_deref(), Some("2026-10-28T09:00:00Z"));
    assert_eq!(
        contacts(dir.path()),
        [(
            "landlord@example.org".to_string(),
            Some("Mr Landlord".to_string())
        )]
    );
    let conn = Connection::open(dir.path().join("pimdir.db")).unwrap();
    let derivation: i64 = conn
        .query_row("SELECT derivation FROM store_meta WHERE id = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(derivation, 2, "revision 2 recorded with the last page");
    assert!(
        stamp(dir.path()) > before,
        "the item is stamped, so a refresh folds its new rows"
    );

    // NOTE: a store at revision 2 is left as it is on the next open.
    let after = stamp(dir.path());
    drop(PimdirStore::open(dir.path()).unwrap());
    assert_eq!(stamp(dir.path()), after);
}
