//! Rules the store schema holds itself (STORAGE §4.1, §5, §13), ported
//! from pimdir's retired checks/invariants.sh: each write breaking one is
//! refused by SQLite, whatever statement an implementation runs.

#![cfg(feature = "client")]

mod invariants_common;

use std::path::Path;

use io_pimdir::{
    change::{PimdirChangeKey, PimdirWriteOp},
    client::PimdirSourceStore,
    collection::PimdirCollectionId,
    object::{PimdirHash, PimdirObject},
    placement::{PimdirHandle, PimdirLinkId},
    reference::{PimdirEndpoint, PimdirReferenceOrigin, PimdirReferenceRole},
};
use rusqlite::{Connection, types::ValueRef};

use invariants_common::*;

const AUG: &str = "Sat, 1 Aug 2026 10:00:00 +0000";
const TABLES: [&str; 10] = [
    "store_meta",
    "collections",
    "sources",
    "objects",
    "items",
    "bindings",
    "queue",
    "receipts",
    "item_reference",
    "item_property",
];

/// INBOX holding `a@x` live with its body, `b@x` live and `r@x`
/// retained, a queue row, a receipt and a person's reference.
fn seeded(dir: &Path) -> PimdirSourceStore {
    let mut store = mail_store(dir, "imap");
    let a = message("a@x", "A", AUG, Some("alice@x"));
    let b = message("b@x", "B", AUG, Some("alice@x"));
    let r = message("r@x", "R", AUG, Some("alice@x"));
    store
        .write(vec![
            body(&store, &a),
            body(&store, &b),
            body(&store, &r),
            PimdirWriteOp::UpsertPlacement(held(&store, "INBOX", "1", &a)),
            PimdirWriteOp::UpsertPlacement(held(&store, "INBOX", "2", &b)),
            PimdirWriteOp::UpsertPlacement(held(&store, "INBOX", "3", &r)),
        ])
        .unwrap();
    store.write(vec![deleted("INBOX", "3")]).unwrap();
    assert_eq!(store.count_retained("INBOX").unwrap(), 1);
    let end = |link: &str| PimdirEndpoint {
        kind: MAIL.into(),
        link_id: PimdirLinkId(link.into()),
    };
    store
        .add_reference(
            &end("a@x"),
            &end("b@x"),
            &PimdirReferenceRole::Related,
            PimdirReferenceOrigin::User,
        )
        .unwrap()
        .unwrap();
    peek(dir)
        .execute_batch(
            "INSERT INTO queue(created_at, producer, collection, action, payload)
                 VALUES('2026-10-01T00:00:00.000Z', 'p', 'INBOX', 'remove', '{\"v\":1}');
             INSERT INTO receipts(id, applied_at, collection, seq)
                 VALUES(9, '2026-10-01T00:00:00.000Z', 'INBOX', 1);",
        )
        .unwrap();
    store
}

fn id_of(conn: &Connection, link: &str) -> i64 {
    int(
        conn,
        &format!("SELECT id FROM items WHERE collection = 'INBOX' AND link_id = '{link}'"),
    )
}

fn object_id(conn: &Connection, hash: &PimdirHash) -> i64 {
    int(
        conn,
        &format!("SELECT id FROM objects WHERE hash = '{}'", hash.0),
    )
}

/// Every row of the tables the rules name, in a stable order.
fn fingerprint(conn: &Connection) -> Vec<String> {
    let mut rows = Vec::new();
    for table in TABLES {
        let mut stmt = conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1, 2"))
            .unwrap();
        let columns = stmt.column_count();
        let mut query = stmt.query([]).unwrap();
        while let Some(row) = query.next().unwrap() {
            let mut line = format!("{table}:");
            for n in 0..columns {
                match row.get_ref(n).unwrap() {
                    ValueRef::Null => line.push_str("|null"),
                    ValueRef::Integer(v) => line.push_str(&format!("|{v}")),
                    ValueRef::Real(v) => line.push_str(&format!("|{v}")),
                    ValueRef::Text(v) => line.push_str(&format!("|{}", String::from_utf8_lossy(v))),
                    ValueRef::Blob(v) => line.push_str(&format!("|{v:?}")),
                }
            }
            rows.push(line);
        }
    }
    rows
}

/// L391: a CHECK refuses clearing `deleted` on a retained row.
#[test]
fn l391_retention_implies_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let _store = seeded(dir.path());
    let conn = peek(dir.path());
    let write = conn.execute("UPDATE items SET deleted = 0 WHERE link_id = 'r@x'", []);
    assert!(refused(write, "retained_at IS NULL OR deleted = 1"));
}

/// L393: a CHECK refuses a `conflict_object` on an item not conflicted.
#[test]
fn l393_a_diverging_body_needs_the_flag() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let conn = peek(dir.path());
    let b = object_id(
        &conn,
        &store.hash(&message("b@x", "B", AUG, Some("alice@x"))),
    );
    let write = conn.execute(
        "UPDATE items SET conflict_object = ?1 WHERE link_id = 'a@x'",
        [b],
    );
    assert!(refused(write, "conflicted = 1 OR conflict_object IS NULL"));
}

/// L486: a body's name is 52 characters of lowercase base32, never a path.
#[test]
fn l486_a_body_name_is_never_a_path() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seeded(dir.path());
    let conn = peek(dir.path());
    let good = "a".repeat(52);
    let names = [
        "../x".to_string(),
        "A".repeat(52),
        "a".repeat(26),
        "a".repeat(53),
        format!("{}1", "a".repeat(51)),
        format!("{}=", "a".repeat(51)),
        format!("aa/{}", "a".repeat(49)),
    ];
    for name in &names {
        let write = conn.execute("INSERT INTO objects(hash, size) VALUES(?1, 1)", [name]);
        assert!(refused(write, "hash"), "{name} refused");
    }
    conn.execute("INSERT INTO objects(hash, size) VALUES(?1, 1)", [&good])
        .unwrap();

    let written = store.write(vec![PimdirWriteOp::StoreObject {
        object: PimdirObject {
            hash: PimdirHash("../x".into()),
            size: 1,
        },
        body: None,
    }]);
    assert!(written.is_err(), "the owner refuses a path as a name");
    assert_eq!(
        int(&conn, "SELECT count(*) FROM objects WHERE hash = '../x'"),
        0
    );
}

/// L522: the `shared_object` foreign key refuses deleting the body it names.
#[test]
fn l522_an_agreed_body_cannot_be_deleted_under_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seeded(dir.path());
    let pinned = b"pinned".to_vec();
    let loose = b"loose".to_vec();
    store
        .write(vec![body(&store, &pinned), body(&store, &loose)])
        .unwrap();
    let conn = peek(dir.path());
    let pinned_id = object_id(&conn, &store.hash(&pinned));
    conn.execute(
        "UPDATE bindings SET shared_object = ?1 WHERE item = ?2",
        [pinned_id, id_of(&conn, "a@x")],
    )
    .unwrap();

    let write = conn.execute("DELETE FROM objects WHERE id = ?1", [pinned_id]);
    assert!(refused(write, "FOREIGN KEY"), "the agreed body stays");
    let loose_id = object_id(&conn, &store.hash(&loose));
    assert_eq!(
        conn.execute("DELETE FROM objects WHERE id = ?1", [loose_id])
            .unwrap(),
        1,
        "a body nothing names goes"
    );
}

/// L702: the detail ladder is 1 and 2, a 0 refused.
#[test]
fn l702_a_level_off_the_ladder_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let _store = seeded(dir.path());
    let conn = peek(dir.path());
    let write = conn.execute(
        "INSERT INTO items(collection, link_id, seq, level) VALUES('INBOX', 'z', 90, 0)",
        [],
    );
    assert!(refused(write, "level IN (1, 2)"));
}

/// L704: a collection conflict policy outside its set is refused.
#[test]
fn l704_a_conflict_policy_outside_its_set_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let _store = seeded(dir.path());
    let conn = peek(dir.path());
    let write = conn.execute(
        "UPDATE collections SET conflict = 'newest' WHERE id = 'INBOX'",
        [],
    );
    assert!(refused(write, "conflict IN"));
}

/// L631: the unique index refuses two holders of one role in an account
/// when the trigger moving it is bypassed.
#[test]
fn l631_a_direct_second_role_holder_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let _store = seeded(dir.path());
    let conn = peek(dir.path());
    conn.execute(
        "INSERT INTO collections(id, kind, name, role) VALUES('S1', 'message/rfc822', 'Sent', 'sent')",
        [],
    )
    .unwrap();
    let write = conn.execute(
        "INSERT INTO collections(id, kind, name, role) VALUES('S2', 'message/rfc822', 'Sent 2', 'sent')",
        [],
    );
    assert!(refused(write, "UNIQUE"));
}

/// L672, L675: a renamed parent is followed by its key, a deleted one
/// leaves its children at the top.
#[test]
fn l672_l675_a_parent_is_followed_by_its_key_and_its_delete_lifts_children() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seeded(dir.path());
    store.ensure_collection("Parent", MAIL).unwrap();
    store
        .set_collection_parent("Archive", Some("Parent"))
        .unwrap();
    store.rename_collection("Parent", "Renamed").unwrap();
    let conn = peek(dir.path());
    let parent = |conn: &Connection| -> Option<String> {
        conn.query_row(
            "SELECT parent FROM collections WHERE id = 'Archive'",
            [],
            |row| row.get(0),
        )
        .unwrap()
    };
    assert_eq!(parent(&conn).as_deref(), Some("Renamed"), "L672");
    assert!(store.delete_collection("Renamed").unwrap());
    assert_eq!(parent(&conn), None, "L675");
}

/// L843: a receipt follows its collection through a rename.
#[test]
fn l843_a_receipt_follows_a_rename() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seeded(dir.path());
    peek(dir.path()).execute("DELETE FROM queue", []).unwrap();
    store.rename_collection("INBOX", "Inbox").unwrap();
    let collection: Option<String> = peek(dir.path())
        .query_row("SELECT collection FROM receipts WHERE id = 9", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(collection.as_deref(), Some("Inbox"));
}

/// The seeded store with imap syncing INBOX: a `sources` row, no round.
fn syncing(dir: &Path) -> PimdirSourceStore {
    let mut store = seeded(dir);
    store
        .write(vec![PimdirWriteOp::SetCheckpoint {
            collection: PimdirCollectionId("INBOX".into()),
            checkpoint: io_pimdir::collection::PimdirCheckpoint(b"c1".to_vec()),
        }])
        .unwrap();
    assert_eq!(int(&peek(dir), "SELECT count(*) FROM sources"), 1);
    store
}

/// L1060: a CHECK refuses a covered bound with no closing.
#[test]
fn l1060_a_bound_without_a_closing_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let _store = syncing(dir.path());
    let write = peek(dir.path()).execute(
        "UPDATE sources SET covered_since = '2026-09-01T00:00:00Z', covered_at = NULL",
        [],
    );
    assert!(refused(write, "covered_at IS NOT NULL OR"));
}

/// L1062: a CHECK refuses a round cursor with no round open.
#[test]
fn l1062_no_round_no_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let _store = syncing(dir.path());
    let write = peek(dir.path()).execute(
        "UPDATE sources SET round_cursor = x'00', round_started_at = NULL",
        [],
    );
    assert!(refused(write, "round_started_at IS NOT NULL OR"));
}

/// L1064: a CHECK refuses a band with no round open.
#[test]
fn l1064_no_round_no_band() {
    let dir = tempfile::tempdir().unwrap();
    let _store = syncing(dir.path());
    let write = peek(dir.path()).execute(
        "UPDATE sources SET round_band = 1, round_started_at = NULL",
        [],
    );
    assert!(refused(write, "round_band = 0 OR"));
}

/// L1213: a base body, revision or known flag set without `base_present`
/// is refused.
#[test]
fn l1213_a_base_column_without_base_present_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let _store = seeded(dir.path());
    let conn = peek(dir.path());
    let a = id_of(&conn, "a@x");
    for mixed in [
        "base_flags_known = 1, base_object = NULL, base_revision = NULL",
        "base_flags_known = 0, base_object = NULL, base_revision = 'r1'",
        "base_flags_known = 0, base_revision = NULL",
    ] {
        let write = conn.execute(
            &format!("UPDATE bindings SET base_present = 0, {mixed} WHERE item = ?1"),
            [a],
        );
        assert!(refused(write, "base_present = 1 OR"), "{mixed}");
    }
}

/// L1966: a CHECK refuses a non-text, non-NULL `sync_version`.
#[test]
fn l1966_the_sync_version_is_text_or_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let _store = seeded(dir.path());
    let conn = peek(dir.path());
    let write = conn.execute("UPDATE store_meta SET sync_version = x'07'", []);
    assert!(refused(write, "typeof(sync_version)"));
    conn.execute("UPDATE store_meta SET sync_version = NULL", [])
        .unwrap();
}

/// L2010: the store id never changes and the incarnation never goes down.
#[test]
fn l2010_the_store_identity_is_fixed() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let before = store.change_cursor().unwrap();
    let raised = store.raise_incarnation(Some(1)).unwrap();
    assert!(raised > before.incarnation, "a raise only raises");

    let conn = peek(dir.path());
    let fixed = |write| match write {
        Err(rusqlite::Error::SqliteFailure(_, Some(message))) => {
            message.contains("store identity is fixed")
        }
        _ => false,
    };
    assert!(fixed(conn.execute(
        "UPDATE store_meta SET store_id = '00000000000000000000000000000000'",
        [],
    )));
    assert!(fixed(conn.execute(
        "UPDATE store_meta SET incarnation = ?1",
        [before.incarnation],
    )));
    let after = store.change_cursor().unwrap();
    assert_eq!(after.incarnation, raised);
}

/// L2023: a value of another type is refused where STRICT refused it, a
/// NULL key and a text checkpoint included.
#[test]
fn l2023_another_type_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let _store = seeded(dir.path());
    let conn = peek(dir.path());
    let hash = "b".repeat(52);
    for values in [
        format!("'{hash}', 'big'"),
        format!("'{hash}', 1.5"),
        format!("CAST('{hash}' AS BLOB), 1"),
        "NULL, 1".to_string(),
    ] {
        let write = conn.execute(
            &format!("INSERT INTO objects(hash, size) VALUES({values})"),
            [],
        );
        assert!(write.is_err(), "{values} refused");
    }
    let write = conn.execute(
        "INSERT INTO sources(collection, source, checkpoint) VALUES('INBOX', 's', 'text')",
        [],
    );
    assert!(refused(write, "checkpoint"));
}

/// L2026: a value SQLite converts losslessly is kept, converted.
#[test]
fn l2026_a_lossless_value_is_converted() {
    let dir = tempfile::tempdir().unwrap();
    let _store = seeded(dir.path());
    let conn = peek(dir.path());
    let hash = "c".repeat(52);
    conn.execute("INSERT INTO objects(hash, size) VALUES(?1, '7')", [&hash])
        .unwrap();
    conn.execute(
        "INSERT INTO items(collection, link_id, seq, level) VALUES('INBOX', 5, 91, 2.0)",
        [],
    )
    .unwrap();
    let size: String = conn
        .query_row(
            "SELECT typeof(size) FROM objects WHERE hash = ?1",
            [&hash],
            |row| row.get(0),
        )
        .unwrap();
    let types: String = conn
        .query_row(
            "SELECT typeof(link_id) || typeof(level) FROM items WHERE seq = 91",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!((size.as_str(), types.as_str()), ("integer", "textinteger"));
}

/// L2063, L2106: each write breaking a column's shape is refused by the
/// CHECK on that column, no row moving, and the other values each column
/// may hold are taken.
#[test]
fn l2063_l2106_a_shape_is_held_by_its_column() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seeded(dir.path());
    store
        .write(vec![PimdirWriteOp::OpenRound {
            collection: PimdirCollectionId("INBOX".into()),
            scope: io_pimdir::collection::PimdirScope::since("2026-09-01T00:00:00Z"),
            band: false,
        }])
        .unwrap();
    let conn = peek(dir.path());
    let before = fingerprint(&conn);
    let shapes = [
        (
            "retained_at GLOB",
            "UPDATE items SET retained_at = 'yesterday' WHERE link_id = 'r@x'",
        ),
        (
            "retained_at GLOB",
            "UPDATE items SET retained_at = '2026-10-01T00:00:00Z' WHERE link_id = 'r@x'",
        ),
        (
            "retained_at GLOB",
            "UPDATE items SET retained_at = '2026-10-01T00:00:00.000+00:00' WHERE link_id = 'r@x'",
        ),
        (
            "retained_at GLOB",
            "UPDATE items SET retained_at = '2026-10-01 00:00:00.000Z' WHERE link_id = 'r@x'",
        ),
        (
            "created_at GLOB",
            "UPDATE store_meta SET created_at = '2026-10-01T00:00:00Z'",
        ),
        (
            "created_at GLOB",
            "UPDATE queue SET created_at = '1790000000'",
        ),
        (
            "created_at GLOB",
            "UPDATE item_reference SET created_at = '2026-10-01'",
        ),
        (
            "applied_at GLOB",
            "UPDATE receipts SET applied_at = '2026-10-01T00:00:00Z'",
        ),
        (
            "covered_at GLOB",
            "UPDATE sources SET covered_at = '2026-10-01T00:00:00Z'",
        ),
        (
            "round_started_at GLOB",
            "UPDATE sources SET round_started_at = '2026-10-01T00:00:00Z'",
        ),
        (
            "covered_since GLOB",
            "UPDATE sources SET covered_since = '2026-09-01', covered_at = '2026-10-01T00:00:00.000Z'",
        ),
        (
            "covered_until GLOB",
            "UPDATE sources SET covered_until = '2026-10-01T00:00:00.000Z', covered_at = '2026-10-01T00:00:00.000Z'",
        ),
        (
            "round_since GLOB",
            "UPDATE sources SET round_since = '2026-09-01T00:00:00'",
        ),
        ("round_until GLOB", "UPDATE sources SET round_until = ''"),
        ("seq >= 1", "UPDATE items SET seq = 0 WHERE link_id = 'a@x'"),
        (
            "seq >= 1",
            "INSERT INTO items(collection, link_id, seq, level) VALUES('INBOX', 'n', -3, 2)",
        ),
        ("seq >= 1", "UPDATE receipts SET seq = 0"),
        (
            "link_id != ''",
            "INSERT INTO items(collection, link_id, seq, level) VALUES('INBOX', '', 3, 2)",
        ),
        (
            "id != ''",
            "INSERT INTO collections(id, kind, name) VALUES('', 'message/rfc822', '')",
        ),
        ("account != ''", "UPDATE collections SET account = ''"),
        ("source != ''", "UPDATE sources SET source = ''"),
        ("source != ''", "UPDATE bindings SET source = ''"),
        ("handle != ''", "UPDATE bindings SET handle = ''"),
        ("size >= 0", "UPDATE objects SET size = -5"),
        ("attempts >= 0", "UPDATE queue SET attempts = -1"),
        (
            "retained_by IS NULL OR retained_at IS NOT NULL",
            "UPDATE items SET retained_by = 'imap' WHERE link_id = 'a@x'",
        ),
        (
            "retained_by IS NULL OR retained_at IS NOT NULL",
            "UPDATE items SET retained_at = NULL WHERE link_id = 'r@x'",
        ),
    ];
    for (column, write) in shapes {
        let result = conn.execute(write, []);
        assert!(
            refused(result, &format!("CHECK constraint failed: {column}")),
            "[{write}] refused by the CHECK on {column}"
        );
    }
    assert_eq!(fingerprint(&conn), before, "no row moved");

    conn.execute_batch(
        "UPDATE sources SET covered_since = NULL, round_until = NULL;
         UPDATE receipts SET seq = NULL;
         UPDATE items SET retained_by = NULL WHERE link_id = 'r@x';
         UPDATE objects SET size = 0 WHERE refcount = 0;",
    )
    .unwrap();
}

/// L2370, L2374, L2378: a refusal is stamped in the shape of §13, states
/// a key of sixteen lowercase hex digits, and survives an upsert.
#[test]
fn l2370_l2374_l2378_a_refusal_is_stamped_keyed_and_kept() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seeded(dir.path());
    store
        .write(vec![PimdirWriteOp::RefusePlacement {
            collection: PimdirCollectionId("INBOX".into()),
            handle: PimdirHandle("1".into()),
            key: PimdirChangeKey("0123456789abcdef".into()),
            reason: "append to Drafts only".into(),
        }])
        .unwrap();
    let conn = peek(dir.path());
    let stamped = "SELECT count(*) FROM bindings WHERE refused_at GLOB \
         '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9].[0-9][0-9][0-9]Z' \
         AND refused_key = '0123456789abcdef'";
    assert_eq!(int(&conn, stamped), 1, "L2370");

    let a = id_of(&conn, "a@x");
    let upper = conn.execute(
        "UPDATE bindings SET refused_key = '0123456789ABCDEF' WHERE item = ?1",
        [a],
    );
    assert!(refused(upper, "refused_key GLOB"), "L2374: uppercase");
    let keyless = conn.execute(
        "UPDATE bindings SET refused_key = NULL WHERE item = ?1",
        [a],
    );
    assert!(refused(keyless, "refusal IS NULL"), "L2374: no key");

    let mut again = held(
        &store,
        "INBOX",
        "1",
        &message("a@x", "A", AUG, Some("alice@x")),
    );
    again.flags = io_pimdir::placement::PimdirFlags::from_iter(["\\Seen"]);
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(again)])
        .unwrap();
    assert_eq!(int(&conn, stamped), 1, "L2378: the upsert keeps it");
}

/// L2486: every pointer at a body is an object id, a hash text refused.
#[test]
fn l2486_a_body_pointer_takes_no_hash_text() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let conn = peek(dir.path());
    let hash = store.hash(&message("a@x", "A", AUG, Some("alice@x")));
    let a = id_of(&conn, "a@x");
    for write in [
        "UPDATE items SET object = ?1 WHERE id = ?2",
        "UPDATE items SET conflicted = 1, conflict_object = ?1 WHERE id = ?2",
        "UPDATE bindings SET base_object = ?1 WHERE item = ?2",
        "UPDATE bindings SET conflicted = 1, conflict_object = ?1 WHERE item = ?2",
        "UPDATE bindings SET shared_object = ?1 WHERE item = ?2",
        "UPDATE queue SET object = ?1 WHERE ?2 IS NOT NULL",
    ] {
        let result = conn.execute(write, rusqlite::params![hash.0, a]);
        assert!(refused(result, "typeof("), "{write}");
    }
}

/// L2521: a write naming a body no object row holds is refused.
#[test]
fn l2521_a_pointer_at_no_body_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let _store = seeded(dir.path());
    let conn = peek(dir.path());
    let a = id_of(&conn, "a@x");
    let missing = 999_999_i64;
    for write in [
        "UPDATE items SET object = ?1 WHERE id = ?2",
        "UPDATE items SET conflicted = 1, conflict_object = ?1 WHERE id = ?2",
        "UPDATE bindings SET base_object = ?1 WHERE item = ?2",
        "UPDATE bindings SET shared_object = ?1 WHERE item = ?2",
    ] {
        let result = conn.execute(write, [missing, a]);
        assert!(refused(result, "FOREIGN KEY"), "{write}");
    }
    let insert = conn.execute(
        "INSERT INTO items(collection, link_id, seq, level, object) VALUES('INBOX', 'q', 92, 2, ?1)",
        [missing],
    );
    assert!(refused(insert, "FOREIGN KEY"));
    assert_eq!(
        int(&conn, "SELECT count(*) FROM items WHERE link_id = 'q'"),
        0
    );
}

/// L2664: a source binds a handle once in a collection; another source,
/// or another collection, takes it.
#[test]
fn l2664_a_source_binds_a_handle_once_in_a_collection() {
    let dir = tempfile::tempdir().unwrap();
    let _store = seeded(dir.path());
    let conn = peek(dir.path());
    let b = id_of(&conn, "b@x");
    conn.execute("DELETE FROM bindings WHERE item = ?1", [b])
        .unwrap();
    let insert = |source: &str| {
        conn.execute(
            "INSERT INTO bindings(item, collection, source, handle) VALUES(?1, 'INBOX', ?2, '1')",
            rusqlite::params![b, source],
        )
    };
    assert!(refused(
        insert("imap"),
        "UNIQUE constraint failed: bindings.collection, bindings.source, bindings.handle"
    ));
    assert_eq!(insert("graph").unwrap(), 1, "another source takes it");
}

fn kind_of(conn: &Connection, collection: &str) -> Option<String> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT kind FROM collections WHERE id = ?1",
        [collection],
        |row| row.get(0),
    )
    .optional()
    .unwrap()
}

/// L607: naming an absent collection creates it with an undeclared kind.
#[test]
fn l607_naming_an_absent_collection_creates_it_kindless() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    store.set_collection_name("Later", "Later").unwrap();
    assert_eq!(kind_of(&peek(dir.path()), "Later").as_deref(), Some(""));
}

/// L695, L700: a kind outside the five is refused, creating nothing, and
/// over a declared kind, which stays.
#[test]
fn l695_l700_a_kind_outside_the_set_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    assert!(store.ensure_collection("Lists", "text/x-vlist").is_err());
    let conn = peek(dir.path());
    assert_eq!(kind_of(&conn, "Lists"), None, "L695: never declared");
    assert!(store.ensure_collection("INBOX", "text/x-vlist").is_err());
    assert_eq!(
        kind_of(&conn, "INBOX").as_deref(),
        Some(MAIL),
        "L700: the declared kind stays"
    );
}

/// L312: a stamp request draws a fresh stamp, so no two items share one.
#[test]
fn l312_stamps_stay_unique_after_a_stamp_request() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seeded(dir.path());
    store
        .write(vec![PimdirWriteOp::Stamp {
            collection: PimdirCollectionId("INBOX".into()),
            handles: vec![PimdirHandle("1".into()), PimdirHandle("2".into())],
        }])
        .unwrap();
    let conn = peek(dir.path());
    assert_eq!(
        int(
            &conn,
            "SELECT count(*) - count(DISTINCT changed) FROM items"
        ),
        0
    );
    assert_eq!(
        int(
            &conn,
            "SELECT count(*) FROM items WHERE changed < 1 \
             OR changed >= (SELECT next_change FROM store_meta)"
        ),
        0,
        "each stamp drawn from the counter"
    );
}

/// L474: a body the collector deletes counts as a purge.
#[test]
fn l474_a_collected_body_counts_as_a_purge() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seeded(dir.path());
    store.write(vec![body(&store, b"loose")]).unwrap();
    let before = store.change_cursor().unwrap().purges;
    let collected = store.collect_garbage().unwrap();
    assert_eq!(collected.objects, 1);
    assert_eq!(store.change_cursor().unwrap().purges, before + 1);
}

/// L850: a queue id is never drawn again, so an old receipt names no new
/// row.
#[test]
fn l850_a_queue_id_is_never_reused() {
    let dir = tempfile::tempdir().unwrap();
    let _store = seeded(dir.path());
    let conn = peek(dir.path());
    let highest = int(&conn, "SELECT max(id) FROM queue");
    conn.execute("DELETE FROM queue", []).unwrap();
    conn.execute(
        "INSERT INTO queue(created_at, producer, collection, action, payload)
         VALUES('2026-10-02T00:00:00.000Z', 'p', 'INBOX', 'remove', '{\"v\":1}')",
        [],
    )
    .unwrap();
    assert!(int(&conn, "SELECT max(id) FROM queue") > highest);
}

/// L2506: an object id is drawn above every id drawn before, the
/// collected highest included, and a body stored again keeps its row.
#[test]
fn l2506_an_object_id_is_never_reused() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = seeded(dir.path());
    let a = message("a@x", "A", AUG, Some("alice@x"));
    let conn = peek(dir.path());
    let a_id = object_id(&conn, &store.hash(&a));
    store.write(vec![body(&store, &a)]).unwrap();
    assert_eq!(object_id(&conn, &store.hash(&a)), a_id, "one row, one id");

    store.write(vec![body(&store, b"loose")]).unwrap();
    let highest = int(&conn, "SELECT max(id) FROM objects");
    assert_eq!(object_id(&conn, &store.hash(b"loose")), highest);
    assert_eq!(store.collect_garbage().unwrap().objects, 1);
    store.write(vec![body(&store, b"later")]).unwrap();
    assert!(object_id(&conn, &store.hash(b"later")) > highest);
}

/// L927, L2225: the schema holds no probes table and no store-wide sort
/// index.
#[test]
fn l927_l2225_the_schema_holds_no_dropped_structure() {
    let dir = tempfile::tempdir().unwrap();
    let _store = seeded(dir.path());
    let conn = peek(dir.path());
    assert_eq!(
        int(
            &conn,
            "SELECT count(*) FROM sqlite_schema \
             WHERE name IN ('probes', 'items_by_sort_global')"
        ),
        0
    );
}
