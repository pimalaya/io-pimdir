//! The index's own rules (SEARCH §6, §7), ported from pimdir's retired
//! checks/invariants.sh: a placement names its body by the index's id and
//! never a redrawn one, a body's categories go with it, an unchanged tz
//! database re-expands nothing and another re-expands every zoned item,
//! and a status the index does not know counts as unparseable.

#![cfg(all(feature = "search", feature = "indexer"))]

mod invariants_common;

use std::path::Path;

use io_pimdir::{
    client::{
        PimdirStore,
        search::{PimdirIndex, PimdirIndexRefreshOptions, PimdirSearchClock},
    },
    search::query::PimdirQuery,
};
use rusqlite::Connection;

use invariants_common::*;

/// 2026-11-22T00:00:00Z.
const NOW: i64 = 1_795_305_600;

const CARD: &[u8] = b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:alice\r\nFN:Alice\r\n\
    CATEGORIES:work,friends\r\nEND:VCARD\r\n";

const ZONED: &[u8] = b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//x//x//EN\r\n\
    BEGIN:VEVENT\r\nUID:zoned\r\nDTSTAMP:20261101T100000Z\r\n\
    DTSTART;TZID=Europe/Paris:20261201T100000\r\n\
    DTEND;TZID=Europe/Paris:20261201T110000\r\nSUMMARY:Zoned\r\n\
    END:VEVENT\r\nEND:VCALENDAR\r\n";

/// A store keeping a card and a zoned single event on the device, its
/// index refreshed once.
fn indexed(dir: &Path) -> PimdirIndex {
    let mut store = PimdirStore::open(dir).unwrap();
    store.ensure_collection("Cards", "text/vcard").unwrap();
    store.ensure_collection("Cal", "text/calendar").unwrap();
    store.put_item("Cards", CARD).unwrap();
    store.put_item("Cal", ZONED).unwrap();
    drop(store);
    let mut index = PimdirIndex::open(dir).unwrap();
    let report = index.refresh(&options()).unwrap();
    assert!(report.is_current());
    assert_eq!(report.expanded, 1, "the event expanded once");
    index
}

fn options() -> PimdirIndexRefreshOptions {
    PimdirIndexRefreshOptions {
        now: Some(NOW),
        ..Default::default()
    }
}

fn index_db(dir: &Path) -> Connection {
    let conn = Connection::open(dir.join("index.db")).unwrap();
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    conn
}

/// L3150: a placement names its body by a positive object id, and an
/// object id is never drawn again once its row went.
#[test]
fn l3150_a_placement_names_a_body_by_an_id_never_redrawn() {
    let dir = tempfile::tempdir().unwrap();
    let _index = indexed(dir.path());
    let conn = index_db(dir.path());
    assert_eq!(
        int(&conn, "SELECT count(*) FROM placement WHERE object > 0"),
        2
    );
    let write = conn.execute("UPDATE placement SET object = 0", []);
    assert!(refused(write, "object > 0"), "no id below one");

    let highest = int(&conn, "SELECT max(id) FROM object");
    conn.execute("DELETE FROM object WHERE id = ?1", [highest])
        .unwrap();
    conn.execute(
        "INSERT INTO object(hash, status, attachment, tel, adr, bday, photo)
         VALUES(?1, 'ok', 0, 0, 0, 0, 0)",
        ["z".repeat(52)],
    )
    .unwrap();
    assert!(
        int(&conn, "SELECT max(id) FROM object") > highest,
        "the deleted body's id is not drawn again"
    );
}

/// L3368: a card's categories go with its object.
#[test]
fn l3368_a_cards_categories_go_with_its_object() {
    let dir = tempfile::tempdir().unwrap();
    let _index = indexed(dir.path());
    let conn = index_db(dir.path());
    let held = "SELECT count(*) FROM object_category";
    assert_eq!(int(&conn, held), 2, "work and friends");

    conn.execute(
        "DELETE FROM object WHERE id IN (SELECT object FROM object_category)",
        [],
    )
    .unwrap();
    assert_eq!(int(&conn, held), 0, "the cascade took them");
}

/// L3253, L3255: the tz database the occurrences stand under re-expands
/// nothing; another re-expands every zoned item, a single one included.
#[test]
fn l3253_l3255_only_another_tzdb_version_re_expands() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = indexed(dir.path());

    let report = index.refresh(&options()).unwrap();
    assert_eq!(report.expanded, 0, "L3253: the same version");

    index_db(dir.path())
        .execute("UPDATE index_meta SET tzdb_version = '1970a'", [])
        .unwrap();
    let report = index.refresh(&options()).unwrap();
    assert_eq!(report.expanded, 1, "L3255: the single zoned event");
}

/// L3282: an object status outside the index's vocabulary is counted as
/// unparseable.
#[test]
fn l3282_an_unknown_status_counts_as_unparseable() {
    let dir = tempfile::tempdir().unwrap();
    let index = indexed(dir.path());
    let unread = |index: &PimdirIndex| {
        index
            .count(
                &PimdirQuery::parse("collection:Cards").unwrap(),
                &PimdirSearchClock::default(),
            )
            .unwrap()
            .coverage
            .local
            .kinds
            .get("text/vcard")
            .copied()
            .unwrap_or_default()
    };
    assert_eq!(unread(&index).unparseable, 0);

    index_db(dir.path())
        .execute(
            "UPDATE object SET status = 'decrypted' WHERE id IN (SELECT object FROM object_category)",
            [],
        )
        .unwrap();
    let counted = unread(&index);
    assert_eq!((counted.unparseable, counted.encrypted), (1, 0));
}

/// L3193, L3678: the index names a body by 52 characters of lowercase
/// base32, never a path, and its flag table refuses the five system flags
/// by their exact bytes.
#[test]
fn l3193_l3678_the_index_refuses_a_path_and_a_system_flag() {
    let dir = tempfile::tempdir().unwrap();
    let _index = indexed(dir.path());
    let conn = index_db(dir.path());
    for name in ["../x".to_string(), "A".repeat(52), "a".repeat(51)] {
        let write = conn.execute(
            "INSERT INTO object(hash, status, attachment, tel, adr, bday, photo)
             VALUES(?1, 'ok', 0, 0, 0, 0, 0)",
            [&name],
        );
        assert!(refused(write, "hash"), "L3193: {name}");
    }

    let (collection, seq): (String, i64) = conn
        .query_row("SELECT collection, seq FROM placement LIMIT 1", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    let flag = |flag: &str| {
        conn.execute(
            "INSERT INTO flag(flag, collection, seq) VALUES(?1, ?2, ?3)",
            rusqlite::params![flag, collection, seq],
        )
    };
    for system in ["\\Seen", "\\Answered", "\\Flagged", "\\Draft", "\\Deleted"] {
        assert!(refused(flag(system), "flag NOT IN"), "L3678: {system}");
    }
    assert_eq!(flag("\\seen").unwrap(), 1, "another case is a keyword");
}
