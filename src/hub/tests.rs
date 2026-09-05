use alloc::vec::Vec;

use crate::{
    change::{PimdirDropReason, PimdirWriteOp},
    hub::*,
    object::PimdirHash,
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
};

fn base(flags: &[&str]) -> PimdirBase {
    PimdirBase {
        flags: PimdirFlags::from_iter(flags.iter().copied()),
        revision: None,
        object: None,
    }
}

/// A hub with one item on `left`, in sync at `Meta` with no body.
fn hub_with_left(flags: &[&str]) -> PimdirHub {
    let mut sources = BTreeMap::new();
    sources.insert(
        PimdirSourceId::from("left"),
        PimdirBinding {
            handle: PimdirHandle::from("l1"),
            base: Some(base(flags)),
            conflicted: false,
            conflict_revision: None,
            conflict_object: None,
            shared_object: None,
        },
    );
    let item = PimdirHubItem {
        sort_key: Default::default(),
        flags: PimdirFlags::from_iter(flags.iter().copied()),
        object: None,
        summary: None,
        level: PimdirLevel::Meta,
        deleted: false,
        conflicted: false,
        conflict_object: None,
        sources,
    };
    PimdirHub {
        items: [(PimdirLinkId::from("m1"), item)].into_iter().collect(),
        ..Default::default()
    }
}

fn placements(hub: &PimdirHub, source: &str) -> Vec<PimdirPlacement> {
    hub.project(&"inbox".into(), &PimdirSourceId::from(source))
}

/// Binds `right` to the single item at the given base flags.
fn bind_right(hub: &mut PimdirHub, flags: &[&str]) {
    hub.items
        .get_mut(&PimdirLinkId::from("m1"))
        .unwrap()
        .sources
        .insert(
            PimdirSourceId::from("right"),
            PimdirBinding {
                handle: PimdirHandle::from("r1"),
                base: Some(base(flags)),
                conflicted: false,
                conflict_revision: None,
                conflict_object: None,
                shared_object: None,
            },
        );
}

/// Gives `link` a body on the item and on left's base: `Full` and clean.
fn hydrate_left(hub: &mut PimdirHub, link: &str) {
    let item = hub.items.get_mut(&PimdirLinkId::from(link)).unwrap();
    item.object = Some(PimdirHash::from("body"));
    item.level = PimdirLevel::Full;
    for binding in item.sources.values_mut() {
        if let Some(base) = &mut binding.base {
            base.object = Some(PimdirHash::from("body"));
        }
        binding.shared_object = Some(PimdirHash::from("body"));
    }
}

/// The key the upgrade mints for left's second copy of `m1`.
fn minted_link() -> PimdirLinkId {
    PimdirLinkId::from("dup:m1#l2")
}

/// Adds left's second copy of `m1` as the item the upgrade mints for it.
fn mint_on_left(hub: &mut PimdirHub) {
    let mut copy = hub.items.get(&PimdirLinkId::from("m1")).unwrap().clone();
    copy.sources = [(
        PimdirSourceId::from("left"),
        PimdirBinding {
            handle: PimdirHandle::from("l2"),
            base: Some(base(&["seen"])),
            conflicted: false,
            conflict_revision: None,
            conflict_object: None,
            shared_object: None,
        },
    )]
    .into_iter()
    .collect();
    hub.items.insert(minted_link(), copy);
    hydrate_left(hub, minted_link().as_str());
}

/// The zero-bodies guardrail: agreeing sources project clean, no body.
#[test]
fn in_agreement_items_project_clean_without_a_body() {
    let mut hub = hub_with_left(&["seen"]);
    hub.items
        .get_mut(&PimdirLinkId::from("m1"))
        .unwrap()
        .sources
        .insert(
            PimdirSourceId::from("right"),
            PimdirBinding {
                handle: PimdirHandle::from("r1"),
                base: Some(base(&["seen"])),
                conflicted: false,
                conflict_revision: None,
                conflict_object: None,
                shared_object: None,
            },
        );

    for source in ["left", "right"] {
        let projected = placements(&hub, source);
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].status, PimdirStatus::Clean);
        assert_ne!(projected[0].level, PimdirLevel::Full);
        assert_eq!(projected[0].object, None, "no body demanded");
    }
}

/// A flag left pulled reads dirty against right's base, so right pushes it.
#[test]
fn a_flag_change_absorbed_from_one_source_projects_dirty_on_the_other() {
    let mut hub = hub_with_left(&[]);
    hub.items
        .get_mut(&PimdirLinkId::from("m1"))
        .unwrap()
        .sources
        .insert(
            PimdirSourceId::from("right"),
            PimdirBinding {
                handle: PimdirHandle::from("r1"),
                base: Some(base(&[])),
                conflicted: false,
                conflict_revision: None,
                conflict_object: None,
                shared_object: None,
            },
        );

    let mut pulled = placements(&hub, "left").pop().unwrap();
    pulled.flags = PimdirFlags::from_iter(["seen"]);
    pulled.status = PimdirStatus::Clean;
    pulled.base = Some(base(&["seen"]));
    hub.absorb(
        &PimdirSourceId::from("left"),
        &[PimdirWriteOp::UpsertPlacement(pulled)],
    );

    let right = placements(&hub, "right").pop().unwrap();
    assert!(right.flags.contains("seen"), "hub adopted left's change");
    assert_eq!(right.status, PimdirStatus::Dirty, "right must push it");
    assert_eq!(
        right.base.unwrap().flags,
        PimdirFlags::from_iter([] as [&str; 0]),
        "right's base is untouched, so the merge pushes to right",
    );
}

#[test]
fn an_item_missing_on_a_source_projects_a_created_append_once_hydrated() {
    let mut hub = hub_with_left(&["seen"]);
    hub.items.get_mut(&PimdirLinkId::from("m1")).unwrap().object = Some(PimdirHash::from("h1"));
    hub.items.get_mut(&PimdirLinkId::from("m1")).unwrap().level = PimdirLevel::Full;

    let right = placements(&hub, "right");
    assert_eq!(right.len(), 1);
    assert_eq!(right[0].status, PimdirStatus::Created);
    assert_eq!(right[0].object, Some(PimdirHash::from("h1")));
    assert!(right[0].base.is_none());
}

/// No body yet, so the append is not staged and nothing forces a fetch.
#[test]
fn a_missing_item_without_a_body_is_not_projected_no_fetch() {
    let hub = hub_with_left(&["seen"]);
    assert!(placements(&hub, "right").is_empty());
}

#[test]
fn absorbing_a_drop_removes_only_that_sources_binding() {
    let mut hub = hub_with_left(&["seen"]);
    hub.items
        .get_mut(&PimdirLinkId::from("m1"))
        .unwrap()
        .sources
        .insert(
            PimdirSourceId::from("right"),
            PimdirBinding {
                handle: PimdirHandle::from("r1"),
                base: Some(base(&["seen"])),
                conflicted: false,
                conflict_revision: None,
                conflict_object: None,
                shared_object: None,
            },
        );

    hub.absorb(
        &PimdirSourceId::from("left"),
        &[PimdirWriteOp::DropPlacement {
            collection: "inbox".into(),
            handle: PimdirHandle::from("l1"),
            reason: PimdirDropReason::Deleted,
        }],
    );

    let item = hub.items.get(&PimdirLinkId::from("m1")).expect("kept");
    assert!(!item.sources.contains_key(&PimdirSourceId::from("left")));
    assert!(item.sources.contains_key(&PimdirSourceId::from("right")));
}

/// A minted copy is a member like the first and travels like one.
#[test]
fn a_minted_copy_is_offered_to_a_source_that_holds_neither() {
    let mut hub = hub_with_left(&["seen"]);
    hydrate_left(&mut hub, "m1");
    mint_on_left(&mut hub);

    let phone = placements(&hub, "phone");
    assert_eq!(phone.len(), 2, "both copies are offered: {phone:?}");
    for placement in &phone {
        assert_eq!(placement.status, PimdirStatus::Created);
        assert_eq!(placement.object, Some(PimdirHash::from("body")));
    }
    assert_eq!(
        placements(&hub, "left").len(),
        2,
        "and the source holding both projects both",
    );
}

/// Two copies are two items: the delete propagates for the one that went.
#[test]
fn a_drop_of_a_minted_copy_deletes_only_that_copy() {
    let mut hub = hub_with_left(&["seen"]);
    hydrate_left(&mut hub, "m1");
    mint_on_left(&mut hub);
    bind_right(&mut hub, &["seen"]);
    hub.items.get_mut(&minted_link()).unwrap().sources.insert(
        PimdirSourceId::from("right"),
        PimdirBinding {
            handle: PimdirHandle::from("r2"),
            base: Some(base(&["seen"])),
            conflicted: false,
            conflict_revision: None,
            conflict_object: None,
            shared_object: None,
        },
    );

    hub.absorb(
        &PimdirSourceId::from("left"),
        &[PimdirWriteOp::DropPlacement {
            collection: "inbox".into(),
            handle: PimdirHandle::from("l2"),
            reason: PimdirDropReason::Deleted,
        }],
    );

    let minted = hub.items.get(&minted_link()).expect("kept");
    assert!(minted.deleted, "the copy that vanished is gone everywhere");
    let bare = hub.items.get(&PimdirLinkId::from("m1")).expect("kept");
    assert!(!bare.deleted, "the copy nobody touched is untouched");

    let right = placements(&hub, "right");
    let status = |handle: &str| {
        right
            .iter()
            .find(|p| p.handle.as_str() == handle)
            .expect("right projects both copies")
            .status
    };
    assert_eq!(
        status("r2"),
        PimdirStatus::Tombstone,
        "right removes the copy that went",
    );
    assert_ne!(
        status("r1"),
        PimdirStatus::Tombstone,
        "and keeps the one that did not",
    );
}

/// The resolver is asked for a tombstone too, answering its destination.
#[test]
fn a_tombstone_is_projected_with_the_destination_the_store_derives() {
    let mut hub = hub_with_left(&["seen"]);
    hub.items
        .get_mut(&PimdirLinkId::from("m1"))
        .unwrap()
        .deleted = true;

    let projected = hub.project_with(&"inbox".into(), &PimdirSourceId::from("left"), |p| {
        assert_eq!(p.status, PimdirStatus::Tombstone);
        Some(PimdirOrigin {
            collection: "archive".into(),
            handle: p.handle.clone(),
        })
    });

    assert_eq!(projected.len(), 1);
    assert_eq!(
        projected[0].origin.as_ref().map(|o| o.collection.as_str()),
        Some("archive"),
    );
}

/// A rekeyed placeholder or a rebuilt spine drops a row, not an item.
#[test]
fn a_superseded_row_does_not_delete_the_shared_item() {
    let mut hub = hub_with_left(&["seen"]);
    bind_right(&mut hub, &["seen"]);

    hub.absorb(
        &PimdirSourceId::from("left"),
        &[PimdirWriteOp::DropPlacement {
            collection: "inbox".into(),
            handle: PimdirHandle::from("l1"),
            reason: PimdirDropReason::Superseded,
        }],
    );

    let item = hub.items.get(&PimdirLinkId::from("m1")).expect("kept");
    assert!(!item.deleted, "no delete propagates");
    let right = placements(&hub, "right");
    assert_eq!(right[0].status, PimdirStatus::Clean, "right is untouched");
}

#[test]
fn a_delete_on_one_source_projects_a_tombstone_on_the_other() {
    let mut hub = hub_with_left(&["seen"]);
    bind_right(&mut hub, &["seen"]);

    hub.absorb(
        &PimdirSourceId::from("left"),
        &[PimdirWriteOp::DropPlacement {
            collection: "inbox".into(),
            handle: PimdirHandle::from("l1"),
            reason: PimdirDropReason::Deleted,
        }],
    );

    let item = hub.items.get(&PimdirLinkId::from("m1")).expect("kept");
    assert!(item.deleted, "the delete is recorded");
    assert!(!item.sources.contains_key(&PimdirSourceId::from("left")));

    let right = placements(&hub, "right");
    assert_eq!(right.len(), 1);
    assert_eq!(right[0].status, PimdirStatus::Tombstone);
    assert_eq!(right[0].handle.as_str(), "r1");
    assert!(
        right[0].base.is_some(),
        "based, so the engine pushes a remove"
    );
    assert!(
        placements(&hub, "left").is_empty(),
        "left no longer holds it, so nothing is re-copied"
    );
}

/// A staged delete keeps its binding, so the projection pushes the remove.
#[test]
fn a_client_staged_tombstone_upsert_marks_deleted_and_keeps_the_binding() {
    let mut hub = hub_with_left(&["seen"]);

    let mut tombstone = placements(&hub, "left").pop().unwrap();
    tombstone.status = PimdirStatus::Tombstone;
    hub.absorb(
        &PimdirSourceId::from("left"),
        &[PimdirWriteOp::UpsertPlacement(tombstone)],
    );

    let item = hub.items.get(&PimdirLinkId::from("m1")).expect("kept");
    assert!(item.deleted, "the staged delete is recorded");
    assert!(
        item.sources.contains_key(&PimdirSourceId::from("left")),
        "the binding is kept so the projection knows the remote handle",
    );

    let left = placements(&hub, "left");
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].status, PimdirStatus::Tombstone);
    assert_eq!(left[0].handle.as_str(), "l1");
    assert!(
        left[0].base.is_some(),
        "based, so the engine pushes a remove"
    );
}

#[test]
fn a_deleted_item_stays_unbound_once_every_source_propagates() {
    let mut hub = hub_with_left(&["seen"]);
    bind_right(&mut hub, &["seen"]);

    for (source, handle) in [("left", "l1"), ("right", "r1")] {
        hub.absorb(
            &PimdirSourceId::from(source),
            &[PimdirWriteOp::DropPlacement {
                collection: "inbox".into(),
                handle: PimdirHandle::from(handle),
                reason: PimdirDropReason::Deleted,
            }],
        );
    }

    let item = hub
        .items
        .get(&PimdirLinkId::from("m1"))
        .expect("kept for the store to retain");
    assert!(item.deleted && item.sources.is_empty());
    assert!(placements(&hub, "left").is_empty() && placements(&hub, "right").is_empty());
}

#[test]
fn a_live_upsert_resurrects_a_delete_in_flight() {
    let mut hub = hub_with_left(&["seen"]);
    bind_right(&mut hub, &["seen"]);

    hub.absorb(
        &PimdirSourceId::from("right"),
        &[PimdirWriteOp::DropPlacement {
            collection: "inbox".into(),
            handle: PimdirHandle::from("r1"),
            reason: PimdirDropReason::Deleted,
        }],
    );
    assert!(hub.items.get(&PimdirLinkId::from("m1")).unwrap().deleted);

    let mut pulled = PimdirPlacement {
        sort_key: Default::default(),
        collection: "inbox".into(),
        handle: PimdirHandle::from("l1"),
        link_id: Some(PimdirLinkId::from("m1")),
        object: None,
        level: PimdirLevel::Meta,
        summary: None,
        flags: PimdirFlags::from_iter(["seen", "flagged"]),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(base(&["seen", "flagged"])),
        origin: None,
    };
    hub.absorb(
        &PimdirSourceId::from("left"),
        &[PimdirWriteOp::UpsertPlacement(pulled.clone())],
    );

    let item = hub
        .items
        .get(&PimdirLinkId::from("m1"))
        .expect("resurrected");
    assert!(!item.deleted, "a live upsert clears the delete");
    assert!(item.flags.contains("flagged"));
    pulled.object = Some(PimdirHash::from("h1"));
    hub.items.get_mut(&PimdirLinkId::from("m1")).unwrap().object = Some(PimdirHash::from("h1"));
    assert_eq!(
        placements(&hub, "right")[0].status,
        PimdirStatus::Created,
        "the resurrected item copies back to right",
    );
}

/// A hub with one mutable item, body `o0`, synced on left and right.
fn content_hub(policy: PimdirHubConflict) -> PimdirHub {
    let based = |handle: &str| PimdirBinding {
        handle: PimdirHandle::from(handle),
        base: Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: Some("r0".into()),
            object: Some(PimdirHash::from("o0")),
        }),
        conflicted: false,
        conflict_revision: None,
        conflict_object: None,
        shared_object: Some(PimdirHash::from("o0")),
    };
    let mut sources = BTreeMap::new();
    sources.insert(PimdirSourceId::from("left"), based("l1"));
    sources.insert(PimdirSourceId::from("right"), based("r1"));
    let item = PimdirHubItem {
        sort_key: Default::default(),
        flags: PimdirFlags::default(),
        object: Some(PimdirHash::from("o0")),
        summary: None,
        level: PimdirLevel::Full,
        deleted: false,
        conflicted: false,
        conflict_object: None,
        sources,
    };
    PimdirHub {
        items: [(PimdirLinkId::from("m1"), item)].into_iter().collect(),
        conflict: policy,
    }
}

/// An upsert write for the shared item from `handle`, carrying `object`.
fn content_upsert(handle: &str, object: &str) -> PimdirWriteOp {
    PimdirWriteOp::UpsertPlacement(PimdirPlacement {
        sort_key: Default::default(),
        collection: "inbox".into(),
        handle: PimdirHandle::from(handle),
        link_id: Some(PimdirLinkId::from("m1")),
        object: Some(PimdirHash::from(object)),
        level: PimdirLevel::Full,
        summary: None,
        flags: PimdirFlags::default(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: Some("r1".into()),
            object: Some(PimdirHash::from(object)),
        }),
        origin: None,
    })
}

/// An offline edit of the shared item from `handle`.
///
/// The shape a local mutation leaves: the new body against the base
/// last synced with the source's own remote, which an edit never moves.
fn edited_upsert(handle: &str, object: &str) -> PimdirWriteOp {
    let PimdirWriteOp::UpsertPlacement(mut placement) = content_upsert(handle, object) else {
        unreachable!()
    };

    placement.status = PimdirStatus::Dirty;
    placement.base = Some(PimdirBase {
        flags: PimdirFlags::default(),
        revision: Some("r0".into()),
        object: Some(PimdirHash::from("o0")),
    });
    PimdirWriteOp::UpsertPlacement(placement)
}

fn item_object(hub: &PimdirHub) -> Option<PimdirHash> {
    hub.items
        .get(&PimdirLinkId::from("m1"))
        .unwrap()
        .object
        .clone()
}

/// Only left edited since both agreed, so the body is adopted.
#[test]
fn a_clean_fast_forward_adopts_the_new_body() {
    let mut hub = content_hub(PimdirHubConflict::Manual);
    hub.absorb(&PimdirSourceId::from("left"), &[content_upsert("l1", "oa")]);
    assert_eq!(item_object(&hub), Some(PimdirHash::from("oa")));
    assert!(!hub.items.get(&PimdirLinkId::from("m1")).unwrap().conflicted);
}

/// A source cannot disagree with itself.
///
/// The first edit moves the shared body ahead of the sync base, the
/// gap another source folding in leaves, and the second edit arriving
/// over it must not read as two sources disagreeing.
#[test]
fn a_second_offline_edit_is_not_a_divergence() {
    let mut hub = content_hub(PimdirHubConflict::Manual);
    let left = PimdirSourceId::from("left");
    hub.items
        .get_mut(&PimdirLinkId::from("m1"))
        .unwrap()
        .sources
        .remove(&PimdirSourceId::from("right"));

    hub.absorb(&left, &[edited_upsert("l1", "o1")]);
    hub.absorb(&left, &[edited_upsert("l1", "o2")]);

    let item = hub.items.get(&PimdirLinkId::from("m1")).unwrap();
    assert_eq!(
        item.object,
        Some(PimdirHash::from("o2")),
        "the second edit is the shared body"
    );
    assert!(!item.conflicted, "a source cannot disagree with itself");
    assert_eq!(item.conflict_object, None, "so nothing diverges from it");
}

/// Two sources' unpushed edits look like one source's second edit.
///
/// Telling them apart is the point: this one is two sources disagreeing.
#[test]
fn a_divergence_between_unpushed_edits_still_conflicts() {
    let mut hub = content_hub(PimdirHubConflict::Manual);
    hub.absorb(&PimdirSourceId::from("left"), &[edited_upsert("l1", "oa")]);
    hub.absorb(&PimdirSourceId::from("right"), &[edited_upsert("r1", "ob")]);

    let item = hub.items.get(&PimdirLinkId::from("m1")).unwrap();
    assert!(item.conflicted, "the divergence is detected");
    assert_eq!(
        item.object,
        Some(PimdirHash::from("oa")),
        "the shared body is kept"
    );
    assert_eq!(
        item.conflict_object,
        Some(PimdirHash::from("ob")),
        "and the diverging one preserved"
    );
}

/// A resolution restating the sync base is still a decision.
///
/// Read as "this source changed nothing", the hub would keep the body
/// the resolution discarded and hand every source a decision the
/// user did not take.
#[test]
fn a_resolution_keeping_the_ancestor_moves_the_shared_body() {
    let mut hub = content_hub(PimdirHubConflict::Manual);
    let left = PimdirSourceId::from("left");
    hub.absorb(&left, &[edited_upsert("l1", "oa")]);

    let PimdirWriteOp::UpsertPlacement(mut conflicted) = edited_upsert("l1", "oa") else {
        unreachable!()
    };
    conflicted.status = PimdirStatus::Conflict;
    conflicted.conflict_revision = Some("r2".into());
    conflicted.conflict_object = Some(PimdirHash::from("ox"));
    hub.absorb(&left, &[PimdirWriteOp::UpsertPlacement(conflicted)]);

    let PimdirWriteOp::UpsertPlacement(mut resolved) = content_upsert("l1", "o0") else {
        unreachable!()
    };
    resolved.status = PimdirStatus::Dirty;
    resolved.base = Some(PimdirBase {
        flags: PimdirFlags::default(),
        revision: Some("r2".into()),
        object: Some(PimdirHash::from("ox")),
    });
    hub.absorb(&left, &[PimdirWriteOp::UpsertPlacement(resolved)]);

    let item = hub.items.get(&PimdirLinkId::from("m1")).unwrap();
    assert_eq!(
        item.object,
        Some(PimdirHash::from("o0")),
        "the shared body is the one the resolution kept",
    );
    assert!(!item.conflicted);
}

/// Both moved from `o0`, so `Manual` flags it and keeps both bodies.
#[test]
fn divergent_content_conflicts_and_preserves_both_manual() {
    let mut hub = content_hub(PimdirHubConflict::Manual);
    hub.absorb(&PimdirSourceId::from("left"), &[content_upsert("l1", "oa")]);
    hub.absorb(
        &PimdirSourceId::from("right"),
        &[content_upsert("r1", "ob")],
    );

    let item = hub.items.get(&PimdirLinkId::from("m1")).unwrap();
    assert!(item.conflicted, "the divergence is detected");
    assert_eq!(
        item.object,
        Some(PimdirHash::from("oa")),
        "the shared body is kept"
    );
    assert_eq!(
        item.conflict_object,
        Some(PimdirHash::from("ob")),
        "the diverging body is preserved for resolution",
    );
}

#[test]
fn prefer_incoming_takes_the_last_writer() {
    let mut hub = content_hub(PimdirHubConflict::PreferIncoming);
    hub.absorb(&PimdirSourceId::from("left"), &[content_upsert("l1", "oa")]);
    hub.absorb(
        &PimdirSourceId::from("right"),
        &[content_upsert("r1", "ob")],
    );

    let item = hub.items.get(&PimdirLinkId::from("m1")).unwrap();
    assert!(!item.conflicted);
    assert_eq!(
        item.object,
        Some(PimdirHash::from("ob")),
        "last writer wins"
    );
}

#[test]
fn prefer_existing_keeps_the_shared_body() {
    let mut hub = content_hub(PimdirHubConflict::PreferExisting);
    hub.absorb(&PimdirSourceId::from("left"), &[content_upsert("l1", "oa")]);
    hub.absorb(
        &PimdirSourceId::from("right"),
        &[content_upsert("r1", "ob")],
    );

    let item = hub.items.get(&PimdirLinkId::from("m1")).unwrap();
    assert!(!item.conflicted);
    assert_eq!(
        item.object,
        Some(PimdirHash::from("oa")),
        "the shared body is kept"
    );
}

/// A conflicted upsert from `handle`, diverging body included.
fn conflicted_upsert(handle: &str, object: &str, revision: &str) -> PimdirWriteOp {
    PimdirWriteOp::UpsertPlacement(PimdirPlacement {
        sort_key: Default::default(),
        collection: "inbox".into(),
        handle: PimdirHandle::from(handle),
        link_id: Some(PimdirLinkId::from("m1")),
        object: Some(PimdirHash::from(object)),
        level: PimdirLevel::Full,
        summary: None,
        flags: PimdirFlags::default(),
        status: PimdirStatus::Conflict,
        conflict_revision: Some(revision.into()),
        conflict_object: Some(PimdirHash::from("o-remote")),
        base: Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: Some("r0".into()),
            object: Some(PimdirHash::from("o0")),
        }),
        origin: None,
    })
}

/// Status, remote revision and diverging body all come back out.
///
/// Read back as `Dirty`, the engine would re-derive the rejected push
/// every run, and a lost body would send the resolver to the network.
#[test]
fn a_conflicted_placement_round_trips_with_its_diverging_body() {
    let mut hub = content_hub(PimdirHubConflict::Manual);
    hub.absorb(
        &PimdirSourceId::from("left"),
        &[conflicted_upsert("l1", "o-local", "r-remote")],
    );

    let projected = placements(&hub, "left");
    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].status, PimdirStatus::Conflict);
    assert_eq!(
        projected[0].object,
        Some(PimdirHash::from("o-local")),
        "the local side of the divergence is the shared body"
    );
    assert_eq!(
        projected[0].conflict_revision.as_deref(),
        Some("r-remote"),
        "the observed remote revision is what a resolver merges against"
    );
    assert_eq!(
        projected[0].conflict_object,
        Some(PimdirHash::from("o-remote")),
        "the diverging body comes back with the revision it describes"
    );
}

/// A matching base would project `Clean`, silently losing the conflict.
#[test]
fn a_conflict_outranks_the_base_comparison() {
    let mut hub = content_hub(PimdirHubConflict::Manual);
    hub.absorb(
        &PimdirSourceId::from("left"),
        &[conflicted_upsert("l1", "o0", "r-remote")],
    );

    let binding =
        hub.items[&PimdirLinkId::from("m1")].sources[&PimdirSourceId::from("left")].clone();
    assert!(binding.conflicted);
    assert_eq!(
        binding.base.as_ref().and_then(|b| b.object.clone()),
        hub.items[&PimdirLinkId::from("m1")].object,
        "the base equals the shared content, so only the conflict decides"
    );
    assert_eq!(placements(&hub, "left")[0].status, PimdirStatus::Conflict);
}

/// Any status but `Conflict` clears the binding, so an edit resolves it.
#[test]
fn resolving_the_conflict_with_an_edit_clears_it() {
    let mut hub = content_hub(PimdirHubConflict::Manual);
    let left = PimdirSourceId::from("left");
    hub.absorb(&left, &[conflicted_upsert("l1", "o-local", "r-remote")]);
    assert_eq!(placements(&hub, "left")[0].status, PimdirStatus::Conflict);

    hub.absorb(&left, &[edited_upsert("l1", "o-merged")]);

    let item = hub.items[&PimdirLinkId::from("m1")].clone();
    assert_eq!(
        item.object,
        Some(PimdirHash::from("o-merged")),
        "the merged body is the shared body, or the next run pushes the unmerged one"
    );

    let binding = item.sources[&left].clone();
    assert!(!binding.conflicted);
    assert_eq!(
        binding.conflict_revision, None,
        "a resolved binding must not carry a stale revision forward"
    );
    assert_eq!(
        binding.conflict_object, None,
        "nor the body that revision named"
    );
    assert_ne!(placements(&hub, "left")[0].status, PimdirStatus::Conflict);
}

/// A per-source conflict and a cross-source one never share a flag.
#[test]
fn the_two_conflict_axes_stay_independent() {
    let mut hub = content_hub(PimdirHubConflict::Manual);
    let link = PimdirLinkId::from("m1");

    hub.absorb(
        &PimdirSourceId::from("left"),
        &[conflicted_upsert("l1", "o-local", "r-remote")],
    );
    assert!(hub.items[&link].sources[&PimdirSourceId::from("left")].conflicted);
    assert!(
        !hub.items[&link].conflicted,
        "a per-source conflict is not a cross-source one"
    );

    assert!(!hub.items[&link].sources[&PimdirSourceId::from("right")].conflicted);
    assert_eq!(placements(&hub, "right")[0].status, PimdirStatus::Dirty);

    let mut hub = content_hub(PimdirHubConflict::Manual);
    hub.absorb(
        &PimdirSourceId::from("left"),
        &[content_upsert("l1", "o-l")],
    );
    hub.absorb(
        &PimdirSourceId::from("right"),
        &[content_upsert("r1", "o-r")],
    );
    assert!(hub.items[&link].conflicted);
    assert_eq!(
        hub.items[&link].conflict_object,
        Some(PimdirHash::from("o-r"))
    );
    assert!(
        !hub.items[&link].sources[&PimdirSourceId::from("right")].conflicted,
        "a cross-source conflict is not a per-source one"
    );
}

/// A staged delete must not inherit a stale conflict.
#[test]
fn a_tombstone_is_never_conflicted() {
    let mut hub = content_hub(PimdirHubConflict::Manual);
    let left = PimdirSourceId::from("left");
    hub.absorb(&left, &[conflicted_upsert("l1", "o-local", "r-remote")]);

    let mut tombstone = match content_upsert("l1", "o0") {
        PimdirWriteOp::UpsertPlacement(p) => p,
        _ => unreachable!(),
    };
    tombstone.status = PimdirStatus::Tombstone;
    hub.absorb(&left, &[PimdirWriteOp::UpsertPlacement(tombstone)]);

    let item = hub.items[&PimdirLinkId::from("m1")].clone();
    assert_eq!(
        item.object,
        Some(PimdirHash::from("o-local")),
        "a staged delete adopts no content"
    );

    let binding = item.sources[&left].clone();
    assert!(!binding.conflicted);
    assert_eq!(binding.conflict_revision, None);
    assert_eq!(binding.conflict_object, None);
}

/// A tombstone adopts no content, and its known flags ride along (SYNC §9).
#[test]
fn a_tombstones_known_flags_ride_along() {
    let mut hub = hub_with_left(&["seen"]);
    bind_right(&mut hub, &["seen"]);

    let mut tombstone = placements(&hub, "left").pop().unwrap();
    tombstone.status = PimdirStatus::Tombstone;
    tombstone.flags = PimdirFlags::from_iter(["seen", "flagged"]);
    hub.absorb(
        &PimdirSourceId::from("left"),
        &[PimdirWriteOp::UpsertPlacement(tombstone)],
    );

    let item = &hub.items[&PimdirLinkId::from("m1")];
    assert!(item.deleted);
    assert!(
        item.flags.contains("flagged"),
        "the marker reached the item"
    );
    assert!(
        placements(&hub, "right")[0].flags.contains("flagged"),
        "and every other source's tombstone",
    );

    let mut blind = placements(&hub, "left").pop().unwrap();
    blind.flags = PimdirFlags::Unknown;
    hub.absorb(
        &PimdirSourceId::from("left"),
        &[PimdirWriteOp::UpsertPlacement(blind)],
    );
    assert!(
        hub.items[&PimdirLinkId::from("m1")]
            .flags
            .contains("flagged"),
        "an unknown set erases nothing",
    );
}

/// A body one source's content pull dropped is no divergence for another.
///
/// Read as one, the other source projects dirty on every load and the
/// flag axis rewrites its row on every run while it owes nothing.
#[test]
fn a_body_less_item_projects_clean_for_a_source_whose_base_holds_a_body() {
    let mut hub = content_hub(PimdirHubConflict::Manual);
    let left = PimdirSourceId::from("left");

    let mut pulled = placements(&hub, "left").pop().unwrap();
    pulled.object = None;
    pulled.level = PimdirLevel::Probed;
    pulled.base = Some(PimdirBase {
        flags: PimdirFlags::default(),
        revision: Some("r1".into()),
        object: None,
    });
    hub.absorb(&left, &[PimdirWriteOp::UpsertPlacement(pulled)]);
    assert_eq!(hub.items[&PimdirLinkId::from("m1")].object, None);

    let right = placements(&hub, "right").pop().unwrap();
    assert_eq!(
        right.status,
        PimdirStatus::Clean,
        "right owes nothing: {right:?}"
    );
    assert_eq!(right.object, None);
    assert_eq!(
        right.base.as_ref().and_then(|b| b.object.clone()),
        Some(PimdirHash::from("o0")),
        "its base still says what its server holds",
    );
}

/// A staged delete adopts no body, so it says nothing about agreement.
///
/// Read as agreement, right's later edit would fast-forward over
/// left's body.
#[test]
fn a_tombstone_does_not_move_the_agreement_point() {
    let mut hub = content_hub(PimdirHubConflict::Manual);
    let right = PimdirSourceId::from("right");
    hub.absorb(&PimdirSourceId::from("left"), &[edited_upsert("l1", "oa")]);

    let PimdirWriteOp::UpsertPlacement(mut tombstone) = edited_upsert("r1", "o0") else {
        unreachable!()
    };
    tombstone.status = PimdirStatus::Tombstone;
    hub.absorb(&right, &[PimdirWriteOp::UpsertPlacement(tombstone)]);
    hub.absorb(&right, &[edited_upsert("r1", "ob")]);

    let item = hub.items.get(&PimdirLinkId::from("m1")).unwrap();
    assert!(item.conflicted, "right never saw left's body");
    assert_eq!(item.object, Some(PimdirHash::from("oa")));
    assert_eq!(item.conflict_object, Some(PimdirHash::from("ob")));
}
mod sort_key_tests {

    use crate::{
        change::PimdirWriteOp,
        hub::*,
        placement::{PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement},
    };

    fn upsert(source: &str, key: &str) -> PimdirWriteOp {
        let _ = source;
        PimdirWriteOp::UpsertPlacement(PimdirPlacement {
            collection: PimdirCollectionId::from("inbox"),
            handle: PimdirHandle::from("1"),
            link_id: Some(PimdirLinkId::from("mid:a@host")),
            object: None,
            level: PimdirLevel::Meta,
            summary: None,
            sort_key: PimdirSortKey::from(key),
            flags: PimdirFlags::default(),
            status: PimdirStatus::Clean,
            conflict_revision: None,
            conflict_object: None,
            base: None,
            origin: None,
        })
    }

    /// An absorbed key comes back out, or the storage reads it as unknown.
    #[test]
    fn a_sort_key_round_trips_through_the_hub() {
        let mut hub = PimdirHub::default();
        let left = PimdirSourceId::from("left");

        hub.absorb(&left, &[upsert("left", "2026-08-01T10:00:00Z")]);

        let projected = hub.project(&PimdirCollectionId::from("inbox"), &left);
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].sort_key.0, "2026-08-01T10:00:00Z");
    }

    /// A source that only probed the item must not un-sort it.
    #[test]
    fn an_unknown_key_does_not_erase_a_known_one() {
        let mut hub = PimdirHub::default();
        let left = PimdirSourceId::from("left");
        let right = PimdirSourceId::from("right");

        hub.absorb(&left, &[upsert("left", "2026-08-01T10:00:00Z")]);
        hub.absorb(&right, &[upsert("right", "")]);

        let projected = hub.project(&PimdirCollectionId::from("inbox"), &left);
        assert_eq!(projected[0].sort_key.0, "2026-08-01T10:00:00Z");
    }

    /// A real key corrects a real key; only unknown is inert.
    #[test]
    fn a_later_derivation_replaces_an_earlier_one() {
        let mut hub = PimdirHub::default();
        let left = PimdirSourceId::from("left");

        hub.absorb(&left, &[upsert("left", "2026-08-01T10:00:00Z")]);
        hub.absorb(&left, &[upsert("left", "2026-08-02T09:00:00Z")]);

        let projected = hub.project(&PimdirCollectionId::from("inbox"), &left);
        assert_eq!(projected[0].sort_key.0, "2026-08-02T09:00:00Z");
    }
}

/// Flags carry the same unknown state a sort key does, and the same rule.
///
/// Per pimdir STORAGE §13 the store's `flags` column is `NULL` until
/// something reads them, distinct from a known-empty `'[]'`.
mod flags_tests {

    use crate::{
        change::PimdirWriteOp,
        hub::*,
        placement::{PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement},
    };

    fn upsert(flags: PimdirFlags) -> PimdirWriteOp {
        PimdirWriteOp::UpsertPlacement(PimdirPlacement {
            collection: PimdirCollectionId::from("inbox"),
            handle: PimdirHandle::from("1"),
            link_id: Some(PimdirLinkId::from("mid:a@host")),
            object: None,
            level: PimdirLevel::Meta,
            summary: None,
            sort_key: PimdirSortKey::default(),
            flags,
            status: PimdirStatus::Clean,
            conflict_revision: None,
            conflict_object: None,
            base: None,
            origin: None,
        })
    }

    /// A source that only probed the item must not clear its markers.
    #[test]
    fn an_unknown_set_does_not_erase_a_known_one() {
        let mut hub = PimdirHub::default();
        let left = PimdirSourceId::from("left");
        let right = PimdirSourceId::from("right");

        hub.absorb(&left, &[upsert(PimdirFlags::from_iter(["seen"]))]);
        hub.absorb(&right, &[upsert(PimdirFlags::Unknown)]);

        let projected = hub.project(&PimdirCollectionId::from("inbox"), &left);
        assert!(projected[0].flags.contains("seen"));
    }

    /// Only unknown is inert: a deliberate clearing is a real set.
    #[test]
    fn a_known_set_replaces_an_unknown_one() {
        let mut hub = PimdirHub::default();
        let left = PimdirSourceId::from("left");

        hub.absorb(&left, &[upsert(PimdirFlags::Unknown)]);
        hub.absorb(&left, &[upsert(PimdirFlags::from_iter(["seen"]))]);
        let projected = hub.project(&PimdirCollectionId::from("inbox"), &left);
        assert!(projected[0].flags.contains("seen"));

        hub.absorb(&left, &[upsert(PimdirFlags::default())]);
        let projected = hub.project(&PimdirCollectionId::from("inbox"), &left);
        assert_eq!(projected[0].flags, PimdirFlags::default());
    }
}

mod stored_level_tests {

    use crate::{
        change::PimdirWriteOp,
        hub::*,
        object::PimdirHash,
        placement::{PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement},
    };

    /// One source's placement of the same item, at the stated level and body.
    fn upsert(level: PimdirLevel, object: Option<&str>, base: Option<&str>) -> PimdirWriteOp {
        PimdirWriteOp::UpsertPlacement(PimdirPlacement {
            collection: PimdirCollectionId::from("contacts"),
            handle: PimdirHandle::from("card-1.vcf"),
            link_id: Some(PimdirLinkId::from("uid:card-1")),
            object: object.map(PimdirHash::from),
            level,
            summary: Some(crate::summary::stub("hi")),
            sort_key: PimdirSortKey::default(),
            flags: PimdirFlags::default(),
            status: PimdirStatus::Clean,
            conflict_revision: None,
            conflict_object: None,
            base: Some(PimdirBase {
                flags: PimdirFlags::default(),
                revision: None,
                object: base.map(PimdirHash::from),
            }),
            origin: None,
        })
    }

    /// The merge dropped the stale body, so the level falls with it: the
    /// pull lowers it to `Probed` (SYNC §5, vectors/sync/11) for the
    /// upgrade to refetch, the summary kept as that of the body it dropped.
    #[test]
    fn a_refreshed_item_stops_claiming_the_body_it_lost() {
        let mut hub = PimdirHub::default();
        let left = PimdirSourceId::from("left");

        hub.absorb(
            &left,
            &[upsert(PimdirLevel::Full, Some("body1"), Some("body1"))],
        );
        hub.absorb(&left, &[upsert(PimdirLevel::Probed, None, None)]);

        let item = &hub.items[&PimdirLinkId::from("uid:card-1")];
        assert_eq!(item.object, None, "the stale body is gone");
        assert_eq!(item.level, PimdirLevel::Probed, "and the level with it");
        assert!(item.summary.is_some(), "the stale summary stays");

        let projected = hub.project(&PimdirCollectionId::from("contacts"), &left);
        assert_eq!(projected[0].level, PimdirLevel::Probed);
    }

    /// An upgrade reads the projection, which heals a store predating the rule.
    #[test]
    fn a_body_less_item_stored_as_full_projects_below_it() {
        let mut hub = PimdirHub::default();
        let left = PimdirSourceId::from("left");

        hub.absorb(&left, &[upsert(PimdirLevel::Full, Some("body1"), None)]);
        let item = hub
            .items
            .get_mut(&PimdirLinkId::from("uid:card-1"))
            .expect("the absorbed item");
        item.object = None;
        item.level = PimdirLevel::Full;

        let projected = hub.project(&PimdirCollectionId::from("contacts"), &left);
        assert_eq!(projected[0].level, PimdirLevel::Meta);
    }

    /// The rule is the body's absence and nothing else.
    #[test]
    fn a_stored_body_keeps_the_level_it_reached() {
        let mut hub = PimdirHub::default();
        let left = PimdirSourceId::from("left");

        hub.absorb(&left, &[upsert(PimdirLevel::Full, Some("body1"), None)]);

        let item = &hub.items[&PimdirLinkId::from("uid:card-1")];
        assert_eq!(item.level, PimdirLevel::Full);
        assert_eq!(item.stored_level(), PimdirLevel::Full);
    }
}
