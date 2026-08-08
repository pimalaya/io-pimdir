//! Ordering a collection by its kind's own key (spec §9.3), and renaming a
//! collection without losing it (spec §12).
//!
//! The two share a file because they share a purpose: both exist so that what a
//! reader sees survives what the store does underneath it. An unordered store
//! forces every consumer to scan a whole collection to show fifty rows, and a
//! store that cannot rename forces a full re-download to change one string.

use io_pimdir::PimdirStore;
use io_replica::{
    change::ReplicaWriteOp,
    client::ReplicaStorage,
    collection::ReplicaCollectionId,
    placement::{
        ReplicaFlags, ReplicaHandle, ReplicaLevel, ReplicaLinkId, ReplicaMeta, ReplicaPlacement,
        ReplicaSortKey, ReplicaStatus,
    },
};
use tempfile::tempdir;

fn placement(collection: &str, handle: &str, link_id: &str) -> ReplicaPlacement {
    ReplicaPlacement {
        sort_key: Default::default(),
        collection: ReplicaCollectionId(collection.into()),
        handle: ReplicaHandle(handle.into()),
        link_id: Some(ReplicaLinkId(link_id.into())),
        object: None,
        level: ReplicaLevel::Meta,
        meta: Some(ReplicaMeta(r#"{"v":1}"#.into())),
        flags: ReplicaFlags::default(),
        status: ReplicaStatus::Clean,
        conflict_revision: None,
        base: None,
        origin: None,
    }
}

/// Writes `items` as `(handle, link_id, sort_key)` into `collection`, staging
/// them through the storage seam and then restating their keys the way a
/// consumer does while the engine does not carry one inline.
fn seed(store: &mut PimdirStore, collection: &str, items: &[(&str, &str, &str)]) {
    let ops = items
        .iter()
        .map(|(handle, link, _)| {
            ReplicaWriteOp::UpsertPlacement(placement(collection, handle, link))
        })
        .collect();
    store.write(ops).expect("stage the placements");

    for (_, link, key) in items {
        store
            .set_sort_key(collection, link, key)
            .expect("restate the sort key");
    }
}

/// The `(sort_key, seq)` cursor of a page's last item.
fn cursor(page: &[io_pimdir::PimdirItem]) -> (String, i64) {
    let last = page.last().expect("a non-empty page");
    (last.sort_key.clone(), last.seq)
}

#[test]
fn a_page_is_total_in_both_directions_across_a_tie() {
    // The point of the `(sort_key, seq)` cursor: two items sharing a key must
    // not make a page boundary skip one or serve it twice. A cursor on the key
    // alone does exactly that, and the bug only appears when a limit happens to
    // split a tie, which is why it is worth pinning.
    let dir = tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path(), "remote").unwrap();
    store.ensure_collection("INBOX", "message/rfc822").unwrap();

    seed(
        &mut store,
        "INBOX",
        &[
            ("1", "a", "2026-08-01T10:00:00Z"),
            ("2", "b", "2026-08-01T12:00:00Z"),
            ("3", "c", "2026-08-01T12:00:00Z"),
            ("4", "d", "2026-08-01T12:00:00Z"),
            ("5", "e", "2025-01-01T00:00:00Z"),
        ],
    );

    for descending in [false, true] {
        let page = |after: Option<(&str, i64)>| {
            if descending {
                store.list_items_page_desc("INBOX", after, 2).unwrap()
            } else {
                store.list_items_page_asc("INBOX", after, 2).unwrap()
            }
        };

        let mut seen = Vec::new();
        let mut after: Option<(String, i64)> = None;
        loop {
            let batch = page(after.as_ref().map(|(k, s)| (k.as_str(), *s)));
            if batch.is_empty() {
                break;
            }
            after = Some(cursor(&batch));
            seen.extend(batch.into_iter().map(|item| item.link_id.0));
        }

        let mut unique = seen.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            seen.len(),
            5,
            "descending={descending}: every item appears exactly once, got {seen:?}"
        );
        assert_eq!(unique.len(), 5, "descending={descending}: no repeats");
    }
}

#[test]
fn an_unknown_key_sorts_last_descending_and_first_ascending() {
    // An item that has not been summarised yet keeps `''`. That is what puts it
    // at the end of a newest-first mail listing, where an unresolved row belongs,
    // and at the head of an A-to-Z contact listing, where it is visible.
    let dir = tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path(), "remote").unwrap();
    store.ensure_collection("INBOX", "message/rfc822").unwrap();

    seed(
        &mut store,
        "INBOX",
        &[
            ("1", "old", "2025-01-01T00:00:00Z"),
            ("2", "new", "2026-08-01T00:00:00Z"),
            ("3", "unknown", ""),
        ],
    );

    let desc: Vec<String> = store
        .list_items_page_desc("INBOX", None, 10)
        .unwrap()
        .into_iter()
        .map(|item| item.link_id.0)
        .collect();
    assert_eq!(desc, ["new", "old", "unknown"]);

    let asc: Vec<String> = store
        .list_items_page_asc("INBOX", None, 10)
        .unwrap()
        .into_iter()
        .map(|item| item.link_id.0)
        .collect();
    assert_eq!(asc, ["unknown", "old", "new"]);
}

#[test]
fn an_ordinary_write_does_not_reset_a_sort_key() {
    // The §9.3 invariant. A save that carries no key must leave the stored one
    // alone; if it did not, ordering would be wiped on every sync and a consumer
    // restating keys afterwards would race its own sync forever.
    let dir = tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path(), "remote").unwrap();
    store.ensure_collection("INBOX", "message/rfc822").unwrap();

    seed(&mut store, "INBOX", &[("1", "a", "2026-08-01T10:00:00Z")]);

    // A second write of the same placement, as a re-sync would produce.
    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(placement(
            "INBOX", "1", "a",
        ))])
        .expect("re-write the placement");

    let items = store.list_items_page_desc("INBOX", None, 10).unwrap();
    assert_eq!(items[0].sort_key, "2026-08-01T10:00:00Z");
}

#[test]
fn renaming_a_collection_carries_its_contents() {
    // The alternative an owner reaches for, deleting and recreating, cascades
    // the whole cache away. This is the operation that makes an account rename
    // or a server-side folder rename survivable.
    let dir = tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path(), "remote")
        .unwrap()
        .for_account("alice");
    store
        .ensure_collection("alice/INBOX", "message/rfc822")
        .unwrap();

    seed(
        &mut store,
        "alice/INBOX",
        &[("1", "a", "2026-08-01T10:00:00Z")],
    );

    store
        .rename_collection("alice/INBOX", "personal/INBOX")
        .expect("rename the collection");

    let moved = store
        .list_items_page_desc("personal/INBOX", None, 10)
        .unwrap();
    assert_eq!(moved.len(), 1, "the item followed the rename");
    assert_eq!(moved[0].sort_key, "2026-08-01T10:00:00Z");

    let left = store.list_items_page_desc("alice/INBOX", None, 10).unwrap();
    assert!(left.is_empty(), "nothing is left behind under the old id");

    // The binding followed too, which is what a sync needs: without it the next
    // pass would treat every item as new and re-download the collection.
    let loaded = store
        .load(&ReplicaCollectionId("personal/INBOX".into()))
        .expect("load the renamed collection");
    assert_eq!(loaded.placements.len(), 1);
    assert_eq!(loaded.placements[0].handle.0, "1");
}

#[test]
fn a_key_written_by_a_placement_survives_a_later_write() {
    // The end of the chain the sort key travels: a connector derives it,
    // io-replica carries it on the placement, and the store binds it. If
    // any link drops it the item lands unsorted, and the failure is
    // invisible until a list comes back in the wrong order.
    let dir = tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path(), "remote").unwrap();
    store.ensure_collection("INBOX", "message/rfc822").unwrap();

    let mut placed = placement("INBOX", "1", "a");
    placed.sort_key = ReplicaSortKey::from("2026-08-01T10:00:00Z");
    store
        .write(vec![ReplicaWriteOp::UpsertPlacement(placed.clone())])
        .expect("stage the placement");

    let stored = store.list_items_page_desc("INBOX", None, 10).unwrap();
    assert_eq!(stored[0].sort_key, "2026-08-01T10:00:00Z");

    // A second write of the same placement, as a re-sync produces: the
    // key round-trips through `load`, so the update rewrites what was
    // already there rather than blanking it.
    let loaded = store
        .load(&ReplicaCollectionId("INBOX".into()))
        .expect("load the collection");
    store
        .write(
            loaded
                .placements
                .into_iter()
                .map(ReplicaWriteOp::UpsertPlacement)
                .collect(),
        )
        .expect("re-write what was loaded");

    let stored = store.list_items_page_desc("INBOX", None, 10).unwrap();
    assert_eq!(
        stored[0].sort_key, "2026-08-01T10:00:00Z",
        "a load-then-write cycle must not blank the key"
    );
}
