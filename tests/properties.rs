//! An item's properties (STORAGE §4.3): local state keyed by the item's
//! public id, written by the owner on its own account or applying a
//! `set-property` (§15.3), read by the four reads of §14.1, folded by a
//! reader's overlay (§15.4), kept by a retention and an eviction (§11.3),
//! gone with the last row holding the seq.

#![cfg(feature = "client")]

use std::path::Path;

use io_pimdir::{
    change::{PimdirDropReason, PimdirWriteOp},
    client::{
        PimdirError, PimdirSourceStore, PimdirStore,
        producer::{PimdirActionStatus, PimdirProducer},
        reader::PimdirReader,
    },
    codec::PimdirAction,
    collection::PimdirCollectionId,
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirSortKey, PimdirStatus,
    },
    property::{LABEL, PimdirProperty, PimdirPropertyHolder},
    summary::{PimdirSummary, mail::PimdirMailSummary},
};

const BODY: &str = "cafebabecafebabecafebabecafebabecafebabecafebabecafe";

/// A clean, based placement of `link` under `handle`, dated 1 August 2026.
fn placement(collection: &str, handle: &str, link: &str) -> PimdirPlacement {
    let date = "2026-08-01T10:00:00Z";
    PimdirPlacement {
        sort_key: PimdirSortKey(date.into()),
        collection: PimdirCollectionId(collection.into()),
        handle: PimdirHandle(handle.into()),
        link_id: Some(PimdirLinkId(link.into())),
        object: Some(PimdirHash(BODY.into())),
        level: PimdirLevel::Full,
        summary: Some(PimdirSummary::Mail(PimdirMailSummary {
            message_id: Some(link.into()),
            subject: link.into(),
            date: Some(date.into()),
            ..Default::default()
        })),
        flags: PimdirFlags::default(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: None,
            object: Some(PimdirHash(BODY.into())),
        }),
        origin: None,
        server_object_id: None,
    }
}

/// A store holding `m` in INBOX and Archive, one seq in both, and `n` in
/// INBOX, each bound by `imap`; answers the seqs of `m` and `n`.
fn seeded(dir: &Path) -> (PimdirSourceStore, i64, i64) {
    let mut store = PimdirStore::open(dir).unwrap().for_source("imap").unwrap();
    for collection in ["INBOX", "Archive"] {
        store
            .ensure_collection(collection, "message/rfc822")
            .unwrap();
    }
    store
        .write(vec![
            PimdirWriteOp::StoreObject {
                object: PimdirObject {
                    hash: PimdirHash(BODY.into()),
                    size: 4,
                },
                body: Some(b"body".to_vec()),
            },
            PimdirWriteOp::UpsertPlacement(placement("INBOX", "1", "m")),
            PimdirWriteOp::UpsertPlacement(placement("INBOX", "2", "n")),
            PimdirWriteOp::UpsertPlacement(placement("Archive", "1", "m")),
        ])
        .unwrap();
    let m = store.seq_for_link("INBOX", "m").unwrap().unwrap();
    let n = store.seq_for_link("INBOX", "n").unwrap().unwrap();
    assert_eq!(store.seq_for_link("Archive", "m").unwrap(), Some(m));
    (store, m, n)
}

fn property(key: &str, value: &str) -> PimdirProperty {
    PimdirProperty {
        key: key.into(),
        value: value.into(),
    }
}

fn label(seq: i64, add: &[&str], remove: &[&str]) -> PimdirAction {
    PimdirAction::SetProperty {
        seq,
        key: LABEL.into(),
        add: add.iter().map(|value| value.to_string()).collect(),
        remove: remove.iter().map(|value| value.to_string()).collect(),
    }
}

fn enqueue(dir: &Path, collection: &str, action: &PimdirAction) -> i64 {
    PimdirProducer::open(dir, "test")
        .unwrap()
        .enqueue(collection, action, None)
        .unwrap()
        .handle
        .id
}

fn status(dir: &Path, id: i64) -> PimdirActionStatus {
    PimdirReader::open(dir).unwrap().action_status(id).unwrap()
}

#[test]
fn a_set_property_applies_as_no_source_and_names_its_item() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, m, _) = seeded(dir.path());

    let id = enqueue(dir.path(), "Archive", &label(m, &["Receipts"], &[]));
    let report = store.drain().unwrap();
    assert_eq!(report.applied, 1);

    let PimdirActionStatus::Applied {
        collection, seq, ..
    } = status(dir.path(), id)
    else {
        panic!("the set-property is applied");
    };
    assert_eq!(collection.as_deref(), Some("Archive"));
    assert_eq!(seq, Some(m));
    assert_eq!(
        store.list_item_properties(m).unwrap(),
        vec![property(LABEL, "Receipts")]
    );
    let holders = |reader: &PimdirReader| reader.list_value_holders(LABEL, "Receipts").unwrap();
    assert_eq!(
        holders(&store),
        vec![
            PimdirPropertyHolder {
                collection: "Archive".into(),
                seq: m,
            },
            PimdirPropertyHolder {
                collection: "INBOX".into(),
                seq: m,
            },
        ],
        "the property holds in every collection filing the seq"
    );
    assert_eq!(store.list_property_holders(LABEL).unwrap(), holders(&store));
}

#[test]
fn a_label_compares_folded_and_is_respelled_by_a_remove_and_an_add() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, m, n) = seeded(dir.path());
    enqueue(dir.path(), "INBOX", &label(m, &["Receipts", "Tax"], &[]));
    enqueue(dir.path(), "INBOX", &label(n, &["receipts"], &[]));
    enqueue(dir.path(), "INBOX", &label(m, &[" RECEIPTS "], &[]));
    store.drain().unwrap();
    assert_eq!(
        store.list_item_properties(m).unwrap(),
        vec![property(LABEL, "Receipts"), property(LABEL, "Tax")],
        "a folded duplicate adds nothing"
    );

    let cursor = store.change_cursor().unwrap();
    enqueue(dir.path(), "INBOX", &label(m, &["TAX"], &["tax"]));
    enqueue(dir.path(), "INBOX", &label(m, &[], &["Absent"]));
    assert_eq!(store.drain().unwrap().applied, 2);
    assert_eq!(
        store.list_item_properties(m).unwrap(),
        vec![property(LABEL, "Receipts"), property(LABEL, "TAX")]
    );
    assert_eq!(
        store.list_property_values(LABEL).unwrap(),
        vec!["Receipts", "TAX", "receipts"],
        "each spelling once, in byte order"
    );
    let changed = store.items_changed_since(cursor.changed, 10).unwrap();
    assert_eq!(changed.len(), 2, "the respelling stamps both rows of m");
    assert!(changed.iter().all(|item| item.seq == m));
}

#[test]
fn a_seq_no_live_row_of_its_collection_holds_parks_and_so_does_a_65th_value() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, m, n) = seeded(dir.path());
    let elsewhere = enqueue(dir.path(), "Archive", &label(n, &["x"], &[]));

    let values: Vec<String> = (0..64).map(|value| value.to_string()).collect();
    let full = PimdirAction::SetProperty {
        seq: m,
        key: "himalaya/tag".into(),
        add: values,
        remove: Vec::new(),
    };
    enqueue(dir.path(), "INBOX", &full);
    let past = enqueue(
        dir.path(),
        "INBOX",
        &PimdirAction::SetProperty {
            seq: m,
            key: "himalaya/tag".into(),
            add: vec!["64".into()],
            remove: Vec::new(),
        },
    );
    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (1, 2));

    let parked = |id| match status(dir.path(), id) {
        PimdirActionStatus::Parked { error, .. } => error,
        other => panic!("parked, not {other:?}"),
    };
    assert!(parked(elsewhere).contains("no live item of Archive"));
    assert!(parked(past).contains("65 values"));
    assert_eq!(store.list_item_properties(n).unwrap(), Vec::new());
    assert_eq!(store.list_item_properties(m).unwrap().len(), 64);
}

#[test]
fn the_overlay_folds_a_pending_set_property_on_its_own_collection_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, m, n) = seeded(dir.path());
    assert!(store.add_property(m, LABEL, "Receipts").unwrap());

    enqueue(
        dir.path(),
        "INBOX",
        &label(m, &["receipts", "Tax"], &["RECEIPTS"]),
    );
    enqueue(dir.path(), "INBOX", &label(n, &["parked"], &[]));
    enqueue(dir.path(), "Archive", &label(n, &["parked"], &[]));
    let reader = PimdirReader::open(dir.path()).unwrap().with_pending();
    assert_eq!(
        reader.item_properties("INBOX", m).unwrap(),
        vec![property(LABEL, "Tax"), property(LABEL, "receipts")]
    );
    assert_eq!(
        reader.item_properties("Archive", m).unwrap(),
        vec![property(LABEL, "Receipts")],
        "a row pending on another collection shows once applied"
    );
    assert_eq!(
        reader.item_properties("INBOX", n).unwrap(),
        vec![property(LABEL, "parked")]
    );
    assert_eq!(
        reader.item_properties("Archive", n).unwrap(),
        Vec::new(),
        "a row the owner parks folds into nothing"
    );
    assert_eq!(
        PimdirReader::open(dir.path())
            .unwrap()
            .item_properties("INBOX", m)
            .unwrap(),
        vec![property(LABEL, "Receipts")],
        "a reader without the overlay reads the committed rows"
    );

    store.drain().unwrap();
    assert_eq!(
        store.list_item_properties(m).unwrap(),
        reader.item_properties("INBOX", m).unwrap(),
        "the overlay predicted the drain"
    );
}

#[test]
fn a_property_outlives_a_retention_and_an_eviction_and_goes_with_the_last_row() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, m, n) = seeded(dir.path());
    assert!(store.add_property(m, "himalaya/muted", "1").unwrap());
    assert!(!store.add_property(m, "himalaya/muted", "1").unwrap());

    let report = store.evict_before("INBOX", "2026-10-01T00:00:00Z").unwrap();
    assert_eq!(report.seqs, vec![n], "an item holding a property stays");

    store
        .write(vec![PimdirWriteOp::DropPlacement {
            collection: PimdirCollectionId("Archive".into()),
            handle: PimdirHandle("1".into()),
            reason: PimdirDropReason::Deleted,
        }])
        .unwrap();
    assert_eq!(store.count_items("Archive").unwrap(), 0);
    assert_eq!(
        store.list_item_properties(m).unwrap(),
        vec![property("himalaya/muted", "1")],
        "a delete leaving a row of the seq keeps them"
    );

    store
        .write(vec![PimdirWriteOp::DropPlacement {
            collection: PimdirCollectionId("INBOX".into()),
            handle: PimdirHandle("1".into()),
            reason: PimdirDropReason::Deleted,
        }])
        .unwrap();
    assert_eq!(
        store.list_item_properties(m).unwrap().len(),
        1,
        "a retained row keeps them"
    );
    assert!(
        store
            .list_property_holders("himalaya/muted")
            .unwrap()
            .is_empty()
    );
    assert!(store.purge("INBOX", m).unwrap());
    assert!(store.list_item_properties(m).unwrap().is_empty());
}

#[test]
fn the_owner_writes_no_property_the_store_cannot_hold() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, m, _) = seeded(dir.path());
    let refused = |result: Result<bool, PimdirError>| match result {
        Err(err) => err.code(),
        Ok(changed) => panic!("refused, not {changed}"),
    };
    assert_eq!(
        refused(store.add_property(m, "pimdir/session-key", "k")),
        "property-invalid"
    );
    assert_eq!(
        refused(store.add_property(m, "Himalaya/x", "1")),
        "property-invalid"
    );
    assert_eq!(
        refused(store.add_property(m, LABEL, "a\0b")),
        "property-invalid"
    );
    assert_eq!(
        refused(store.add_property(m, LABEL, &"x".repeat(1025))),
        "property-invalid"
    );
    for value in 0..64 {
        store.add_property(m, "x/y", &value.to_string()).unwrap();
    }
    assert_eq!(
        refused(store.add_property(m, "x/y", "64")),
        "property-invalid"
    );
    assert_eq!(refused(store.add_property(9999, "x/y", "1")), "sql");

    assert!(store.add_property(m, LABEL, "Tax").unwrap());
    assert!(!store.add_property(m, LABEL, "TAX").unwrap());
    assert!(store.remove_property(m, LABEL, "tax").unwrap());
    assert!(!store.remove_property(m, LABEL, "tax").unwrap());
}

#[test]
fn a_producer_refuses_a_key_pimdir_keeps() {
    let dir = tempfile::tempdir().unwrap();
    let (_, m, _) = seeded(dir.path());
    let refused = PimdirProducer::open(dir.path(), "test")
        .unwrap()
        .enqueue(
            "INBOX",
            &PimdirAction::SetProperty {
                seq: m,
                key: "pimdir/session-key".into(),
                add: vec!["k".into()],
                remove: Vec::new(),
            },
            None,
        )
        .unwrap_err();
    assert_eq!(refused.code(), "action-malformed");
    assert!(
        PimdirReader::open(dir.path())
            .unwrap()
            .list_pending_actions()
            .unwrap()
            .is_empty()
    );
}
