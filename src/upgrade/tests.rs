use alloc::{collections::BTreeMap, string::String, vec};

use crate::{
    load::PimdirLoaded,
    object::PimdirHash,
    placement::{PimdirBase, PimdirFlags, PimdirLinkId, PimdirOrigin, PimdirStatus},
    remote::{PimdirFetchedBody, PimdirFetchedItem},
    upgrade::*,
};

/// The placement an `UpsertPlacement` op writes for `handle`, if any.
fn upserted<'a>(ops: &'a [PimdirWriteOp], handle: &str) -> Option<&'a PimdirPlacement> {
    ops.iter().rev().find_map(|op| match op {
        PimdirWriteOp::UpsertPlacement(p) if p.handle.as_str() == handle => Some(p),
        _ => None,
    })
}

fn probed(handle: &str, link: Option<&str>, level: PimdirLevel) -> PimdirPlacement {
    PimdirPlacement {
        sort_key: Default::default(),
        collection: "inbox".into(),
        handle: PimdirHandle::from(handle),
        link_id: link.map(PimdirLinkId::from),
        object: None,
        level,
        summary: None,
        flags: PimdirFlags::default(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: None,
        origin: None,
    }
}

#[test]
fn full_dedup_links_without_fetch() {
    crate::testlog::init();
    let loaded = PimdirLoaded {
        placements: vec![probed("2", Some("msg-a"), PimdirLevel::Meta)],
        checkpoint: None,
    };
    let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("2")], PimdirTier::Full);
    let _ = up.resume(None);

    let links = match up.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsLookupObject(links)) => links,
        state => panic!("expected WantsLookupObject, got {state:?}"),
    };
    assert_eq!(links, vec![PimdirLinkId::from("msg-a")]);

    let mut known = BTreeMap::new();
    known.insert(PimdirLinkId::from("msg-a"), PimdirHash::from("h-a"));

    let ops = match up.resume(Some(PimdirArg::LookupObject(known))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite (no fetch), got {state:?}"),
    };
    let PimdirWriteOp::UpsertPlacement(p) = &ops[0] else {
        panic!("expected UpsertPlacement, got {:?}", ops[0]);
    };
    assert_eq!(p.level, PimdirLevel::Full);
    assert_eq!(p.object, Some(PimdirHash::from("h-a")));

    let report = match up.resume(Some(PimdirArg::Write)) {
        PimdirCoroutineState::Complete(Ok(report)) => report,
        state => panic!("expected Complete(Ok), got {state:?}"),
    };
    assert_eq!(report.deduped, 1);
    assert_eq!(report.fetched, 0);
}

#[test]
fn full_miss_fetches_and_stores() {
    let loaded = PimdirLoaded {
        placements: vec![probed("1", Some("msg-b"), PimdirLevel::Meta)],
        checkpoint: None,
    };
    let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Full);
    let _ = up.resume(None);
    let _ = up.resume(Some(PimdirArg::Load(loaded)));

    let handles = match up.resume(Some(PimdirArg::LookupObject(BTreeMap::new()))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsFetch { handles, tier, .. }) => {
            assert_eq!(tier, PimdirTier::Full);
            handles
        }
        state => panic!("expected WantsFetch, got {state:?}"),
    };
    assert_eq!(handles, vec![PimdirHandle::from("1")]);

    let items = vec![PimdirFetchedItem {
        sort_key: Default::default(),
        handle: PimdirHandle::from("1"),
        link_id: PimdirLinkId::from("msg-b"),
        summary: Some(crate::summary::stub("hdr")),
        body: Some(PimdirFetchedBody::Inline {
            hash: PimdirHash::from("h-b"),
            bytes: b"body".to_vec(),
        }),
        revision: None,
    }];
    let ops = match up.resume(Some(PimdirArg::Fetch(items))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };
    assert!(matches!(ops[0], PimdirWriteOp::StoreObject { .. }));

    let report = match up.resume(Some(PimdirArg::Write)) {
        PimdirCoroutineState::Complete(Ok(report)) => report,
        state => panic!("expected Complete(Ok), got {state:?}"),
    };
    assert_eq!(report.fetched, 1);
    assert_eq!(report.deduped, 0);
}

#[test]
fn fetch_results_are_matched_by_handle_not_order() {
    let loaded = PimdirLoaded {
        placements: vec![
            probed("1", Some("msg-a"), PimdirLevel::Meta),
            probed("2", Some("msg-b"), PimdirLevel::Meta),
        ],
        checkpoint: None,
    };
    let mut up = PimdirUpgrade::new(
        "inbox",
        vec![PimdirHandle::from("1"), PimdirHandle::from("2")],
        PimdirTier::Full,
    );
    let _ = up.resume(None);
    let _ = up.resume(Some(PimdirArg::Load(loaded)));
    let _ = up.resume(Some(PimdirArg::LookupObject(BTreeMap::new())));

    let items = vec![
        PimdirFetchedItem {
            sort_key: Default::default(),
            handle: PimdirHandle::from("2"),
            link_id: PimdirLinkId::from("msg-b"),
            summary: Some(crate::summary::stub("h")),
            body: Some(PimdirFetchedBody::Inline {
                hash: PimdirHash::from("h-b"),
                bytes: b"bbb".to_vec(),
            }),
            revision: None,
        },
        PimdirFetchedItem {
            sort_key: Default::default(),
            handle: PimdirHandle::from("1"),
            link_id: PimdirLinkId::from("msg-a"),
            summary: Some(crate::summary::stub("h")),
            body: Some(PimdirFetchedBody::Inline {
                hash: PimdirHash::from("h-a"),
                bytes: b"aaaaa".to_vec(),
            }),
            revision: None,
        },
    ];
    let ops = match up.resume(Some(PimdirArg::Fetch(items))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };

    let object_for = |handle: &str| {
        ops.iter().find_map(|op| match op {
            PimdirWriteOp::UpsertPlacement(p) if p.handle.as_str() == handle => p.object.clone(),
            _ => None,
        })
    };
    assert_eq!(object_for("1"), Some(PimdirHash::from("h-a")));
    assert_eq!(object_for("2"), Some(PimdirHash::from("h-b")));
}

#[test]
fn a_full_fetch_keeps_an_already_resolved_link_id() {
    let loaded = PimdirLoaded {
        placements: vec![probed("1", Some("mid:real"), PimdirLevel::Meta)],
        checkpoint: None,
    };
    let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Full);
    let _ = up.resume(None);
    let _ = up.resume(Some(PimdirArg::Load(loaded)));
    let _ = up.resume(Some(PimdirArg::LookupObject(BTreeMap::new())));

    let items = vec![PimdirFetchedItem {
        sort_key: Default::default(),
        handle: PimdirHandle::from("1"),
        link_id: PimdirLinkId::from("alt:divergent"),
        summary: Some(crate::summary::stub("hdr")),
        body: Some(PimdirFetchedBody::Persisted {
            hash: PimdirHash::from("h"),
            size: 10,
        }),
        revision: None,
    }];
    let ops = match up.resume(Some(PimdirArg::Fetch(items))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };
    let placement = ops
        .iter()
        .find_map(|op| match op {
            PimdirWriteOp::UpsertPlacement(p) => Some(p),
            _ => None,
        })
        .expect("a placement upsert");
    assert_eq!(
        placement.link_id,
        Some(PimdirLinkId::from("mid:real")),
        "the Full fetch keeps the Meta-resolved link, not the body's"
    );
    assert_eq!(placement.level, PimdirLevel::Full);
}

#[test]
fn a_meta_fetch_still_sets_the_link_of_an_unlinked_item() {
    let loaded = PimdirLoaded {
        placements: vec![probed("1", None, PimdirLevel::Probed)],
        checkpoint: None,
    };
    let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Meta);
    let _ = up.resume(None);
    let _ = up.resume(Some(PimdirArg::Load(loaded)));
    let _ = up.resume(Some(PimdirArg::LookupObject(BTreeMap::new())));

    let items = vec![PimdirFetchedItem {
        sort_key: Default::default(),
        handle: PimdirHandle::from("1"),
        link_id: PimdirLinkId::from("mid:resolved"),
        summary: Some(crate::summary::stub("hdr")),
        body: None,
        revision: None,
    }];
    match up.resume(Some(PimdirArg::Fetch(items))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsLoad {
            scope: PimdirLoadScope::Links(links),
            ..
        }) => assert_eq!(
            links,
            vec![
                PimdirLinkId::from("mid:resolved"),
                PimdirLinkId::from("dup:mid:resolved#1"),
            ],
        ),
        state => panic!("expected WantsLoad, got {state:?}"),
    }
    let ops = match up.resume(Some(PimdirArg::Load(PimdirLoaded::default()))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };
    let placement = ops
        .iter()
        .find_map(|op| match op {
            PimdirWriteOp::UpsertPlacement(p) => Some(p),
            _ => None,
        })
        .expect("a placement upsert");
    assert_eq!(
        placement.link_id,
        Some(PimdirLinkId::from("mid:resolved")),
        "a probed item takes the fetched link"
    );
}

/// The link ids the upgrade wrote, by handle.
fn links(ops: &[PimdirWriteOp]) -> BTreeMap<&str, Option<&str>> {
    ops.iter()
        .filter_map(|op| match op {
            PimdirWriteOp::UpsertPlacement(p) => {
                Some((p.handle.as_str(), p.link_id.as_ref().map(|l| l.as_str())))
            }
            _ => None,
        })
        .collect()
}

/// The write batch of a meta upgrade of `handles` resolving to `link`.
///
/// The fresh identity is checked against the `stored` placements.
fn upgrade_twins(
    handles: &[&str],
    link: &str,
    loaded: Vec<PimdirPlacement>,
    stored: Vec<PimdirPlacement>,
) -> Vec<PimdirWriteOp> {
    crate::testlog::init();
    let requested = handles.iter().copied().map(PimdirHandle::from).collect();
    let mut up = PimdirUpgrade::new("inbox", requested, PimdirTier::Meta);
    let _ = up.resume(None);
    let _ = up.resume(Some(PimdirArg::Load(PimdirLoaded {
        placements: loaded,
        checkpoint: None,
    })));

    let items = handles
        .iter()
        .map(|handle| PimdirFetchedItem {
            sort_key: Default::default(),
            handle: PimdirHandle::from(*handle),
            link_id: PimdirLinkId::from(link),
            summary: Some(crate::summary::stub("hdr")),
            body: None,
            revision: None,
        })
        .collect();

    let ops = match up.resume(Some(PimdirArg::Fetch(items))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => Some(ops),
        PimdirCoroutineState::Yielded(PimdirYield::WantsLoad {
            scope: PimdirLoadScope::Links(_),
            ..
        }) => None,
        state => panic!("expected WantsWrite or a link check, got {state:?}"),
    };

    match ops {
        Some(ops) => ops,
        None => match up.resume(Some(PimdirArg::Load(PimdirLoaded {
            placements: stored,
            checkpoint: None,
        }))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        },
    }
}

/// Both copies are hydrated by one batch, neither linked yet.
#[test]
fn a_second_copy_of_one_identity_is_minted() {
    let ops = upgrade_twins(
        &["u1", "u2"],
        "m1",
        vec![
            probed("u1", None, PimdirLevel::Probed),
            probed("u2", None, PimdirLevel::Probed),
        ],
        Vec::new(),
    );

    assert_eq!(
        links(&ops),
        BTreeMap::from([("u1", Some("m1")), ("u2", Some("dup:m1#u2"))]),
        "the first copy keeps the hint, the second is minted from it \
         and its own handle",
    );
}

/// Only the second copy is hydrated: the holder comes from the check.
#[test]
fn the_mint_is_decided_against_the_collection_not_the_batch() {
    let ops = upgrade_twins(
        &["u2"],
        "m1",
        vec![probed("u2", None, PimdirLevel::Probed)],
        vec![probed("u1", Some("m1"), PimdirLevel::Meta)],
    );

    assert_eq!(
        links(&ops),
        BTreeMap::from([("u2", Some("dup:m1#u2"))]),
        "a batch that never names the holder still mints",
    );
}

#[test]
fn a_minted_copy_is_not_minted_again() {
    let ops = upgrade_twins(
        &["u2"],
        "m1",
        vec![probed("u2", Some("dup:m1#u2"), PimdirLevel::Probed)],
        vec![probed("u1", Some("m1"), PimdirLevel::Meta)],
    );

    assert_eq!(
        links(&ops),
        BTreeMap::from([("u2", Some("dup:m1#u2"))]),
        "no dup:dup:m1#u2#u2",
    );
}

/// A pending create of `link` under a provisional handle, body `h-c`.
fn pending_create(handle: &str, link: &str, flags: &[&str]) -> PimdirPlacement {
    let mut create = probed(handle, Some(link), PimdirLevel::Full);
    create.object = Some(PimdirHash::from("h-c"));
    create.flags = PimdirFlags::from_iter(flags.iter().copied());
    create.summary = Some(crate::summary::stub("staged"));
    create.status = PimdirStatus::Created;
    create.origin = Some(PimdirOrigin {
        collection: "sent".into(),
        handle: PimdirHandle::from("9"),
    });
    create
}

/// A hint a pending create holds is that create arriving (SYNC §6).
///
/// The provisional handle is superseded in the same batch, the binding
/// moves onto the fetched handle, and the base is what the fetch reported
/// while the staged flags and body stay.
#[test]
fn a_fetched_hint_held_by_a_pending_create_lands_it() {
    let mut probe = probed("7", None, PimdirLevel::Probed);
    probe.flags = PimdirFlags::from_iter(["seen"]);
    let ops = upgrade_twins(
        &["7"],
        "m1",
        vec![probe],
        vec![pending_create("tmp-1", "m1", &["seen"])],
    );

    assert_eq!(
        ops[0],
        PimdirWriteOp::DropPlacement {
            collection: "inbox".into(),
            handle: PimdirHandle::from("tmp-1"),
            reason: PimdirDropReason::Superseded,
        },
        "the provisional handle goes first: {ops:?}",
    );
    let landed = upserted(&ops, "7").expect("the create under the fetched handle");
    assert_eq!(landed.link_id, Some(PimdirLinkId::from("m1")), "not minted");
    assert_eq!(landed.status, PimdirStatus::Clean);
    assert_eq!(
        landed.object,
        Some(PimdirHash::from("h-c")),
        "the body stays"
    );
    assert_eq!(landed.level, PimdirLevel::Full);
    assert_eq!(landed.summary, Some(crate::summary::stub("staged")));
    assert_eq!(landed.origin, None, "landed, so nothing left to copy from");
    let base = landed.base.as_ref().expect("landing bases the create");
    assert_eq!(base.flags, PimdirFlags::from_iter(["seen"]));
    assert_eq!(base.object, Some(PimdirHash::from("h-c")));
    assert!(
        upserted(&ops, "tmp-1").is_none(),
        "the provisional row is not rewritten: {ops:?}",
    );
}

/// The flags and body staged on the create still push after landing.
#[test]
fn a_landed_create_keeps_its_staged_edit_pending() {
    let mut probe = probed("7", None, PimdirLevel::Probed);
    probe.flags = PimdirFlags::from_iter(["seen"]);
    let ops = upgrade_twins(
        &["7"],
        "m1",
        vec![probe],
        vec![pending_create("tmp-1", "m1", &["seen", "flagged"])],
    );

    let landed = upserted(&ops, "7").expect("the landed create");
    assert_eq!(landed.status, PimdirStatus::Dirty);
    assert!(landed.flags.contains("flagged"), "the staged flag stays");
    let base = landed.base.as_ref().expect("a base");
    assert!(
        !base.flags.contains("flagged"),
        "the base is what was fetched"
    );
}

/// A hint a based binding holds is a second copy, minted as before.
#[test]
fn a_hint_held_by_a_based_binding_is_still_minted() {
    let ops = upgrade_twins(
        &["7"],
        "m1",
        vec![probed("7", None, PimdirLevel::Probed)],
        vec![based("u1", "m1", None)],
    );

    assert_eq!(links(&ops), BTreeMap::from([("7", Some("dup:m1#7"))]));
    assert!(
        !ops.iter()
            .any(|op| matches!(op, PimdirWriteOp::DropPlacement { .. })),
        "nothing is superseded: {ops:?}",
    );
}

/// A source is free to spell its own identity like a minted key.
#[test]
fn a_mint_never_takes_a_key_the_collection_holds() {
    let ops = upgrade_twins(
        &["u2"],
        "m1",
        vec![probed("u2", None, PimdirLevel::Probed)],
        vec![
            probed("u1", Some("m1"), PimdirLevel::Meta),
            probed("u3", Some("dup:m1#u2"), PimdirLevel::Meta),
        ],
    );

    assert_eq!(
        links(&ops),
        BTreeMap::from([("u2", Some("dup:dup:m1#u2#u2"))]),
        "the copy takes a key of its own rather than u3's",
    );
}

#[test]
fn a_meta_fetch_keeps_a_body_holding_row_full() {
    let mut stored = probed("1", Some("msg-a"), PimdirLevel::Full);
    stored.object = Some(PimdirHash::from("h1"));
    let loaded = PimdirLoaded {
        placements: vec![stored],
        checkpoint: None,
    };
    let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Meta);
    let _ = up.resume(None);
    let _ = up.resume(Some(PimdirArg::Load(loaded)));

    let items = vec![PimdirFetchedItem {
        sort_key: Default::default(),
        handle: PimdirHandle::from("1"),
        link_id: PimdirLinkId::from("msg-a"),
        summary: Some(crate::summary::stub("hdr")),
        body: None,
        revision: None,
    }];
    let ops = match up.resume(Some(PimdirArg::Fetch(items))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };

    let upserted = ops
        .iter()
        .find_map(|op| match op {
            PimdirWriteOp::UpsertPlacement(p) => Some(p),
            _ => None,
        })
        .expect("the summarised placement");
    assert_eq!(upserted.level, PimdirLevel::Full);
    assert_eq!(upserted.summary, Some(crate::summary::stub("hdr")));
}

#[test]
fn a_persisted_body_stores_the_object_without_bytes() {
    let loaded = PimdirLoaded {
        placements: vec![probed("1", Some("msg-b"), PimdirLevel::Meta)],
        checkpoint: None,
    };
    let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Full);
    let _ = up.resume(None);
    let _ = up.resume(Some(PimdirArg::Load(loaded)));
    let _ = up.resume(Some(PimdirArg::LookupObject(BTreeMap::new())));

    let items = vec![PimdirFetchedItem {
        sort_key: Default::default(),
        handle: PimdirHandle::from("1"),
        link_id: PimdirLinkId::from("msg-b"),
        summary: Some(crate::summary::stub("hdr")),
        body: Some(PimdirFetchedBody::Persisted {
            hash: PimdirHash::from("h-b"),
            size: 4096,
        }),
        revision: None,
    }];
    let ops = match up.resume(Some(PimdirArg::Fetch(items))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };
    match &ops[0] {
        PimdirWriteOp::StoreObject { object, body } => {
            assert_eq!(object.hash, PimdirHash::from("h-b"));
            assert_eq!(object.size, 4096, "size comes from the report, not bytes");
            assert!(body.is_none(), "no bytes: the fetch already persisted them");
        }
        other => panic!("expected StoreObject, got {other:?}"),
    }
    assert!(matches!(
        &ops[1],
        PimdirWriteOp::UpsertPlacement(p)
            if p.object == Some(PimdirHash::from("h-b")) && p.level == PimdirLevel::Full
    ));
}

#[test]
fn full_fetch_stamps_the_base_revision_and_object() {
    let mut placement = probed("1", Some("msg-b"), PimdirLevel::Meta);
    placement.base = Some(PimdirBase {
        flags: PimdirFlags::default(),
        revision: None,
        object: None,
    });
    let loaded = PimdirLoaded {
        placements: vec![placement],
        checkpoint: None,
    };

    let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Full);
    let _ = up.resume(None);
    let _ = up.resume(Some(PimdirArg::Load(loaded)));
    let _ = up.resume(Some(PimdirArg::LookupObject(BTreeMap::new())));

    let items = vec![PimdirFetchedItem {
        sort_key: Default::default(),
        handle: PimdirHandle::from("1"),
        link_id: PimdirLinkId::from("msg-b"),
        summary: Some(crate::summary::stub("hdr")),
        body: Some(PimdirFetchedBody::Inline {
            hash: PimdirHash::from("h-b"),
            bytes: b"body".to_vec(),
        }),
        revision: Some("r7".into()),
    }];
    let ops = match up.resume(Some(PimdirArg::Fetch(items))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };

    let patched = ops
        .iter()
        .find_map(|op| match op {
            PimdirWriteOp::UpsertPlacement(p) => Some(p),
            _ => None,
        })
        .expect("an upserted placement");
    let base = patched.base.as_ref().expect("a base");
    assert_eq!(base.revision.as_deref(), Some("r7"));
    assert_eq!(base.object, Some(PimdirHash::from("h-b")));
    assert_eq!(patched.object, Some(PimdirHash::from("h-b")));
}

#[test]
fn meta_upgrade_fetches_headers() {
    let loaded = PimdirLoaded {
        placements: vec![probed("1", None, PimdirLevel::Probed)],
        checkpoint: None,
    };
    let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Meta);
    let _ = up.resume(None);

    match up.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsFetch { tier, .. }) => {
            assert_eq!(tier, PimdirTier::Meta);
        }
        state => panic!("expected WantsFetch Meta, got {state:?}"),
    }
}

#[test]
fn already_full_completes_without_work() {
    let mut placement = probed("1", Some("x"), PimdirLevel::Full);
    placement.object = Some(PimdirHash::from("h1"));
    let loaded = PimdirLoaded {
        placements: vec![placement],
        checkpoint: None,
    };
    let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Full);
    let _ = up.resume(None);

    match up.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Complete(Ok(report)) => assert_eq!(report.upgraded, 0),
        state => panic!("expected Complete(Ok), got {state:?}"),
    }
}

/// Else a row recorded full with no body would be skipped forever.
#[test]
fn a_full_row_holding_no_body_is_upgraded_again() {
    let loaded = PimdirLoaded {
        placements: vec![probed("1", None, PimdirLevel::Full)],
        checkpoint: None,
    };
    let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Full);
    let _ = up.resume(None);

    match up.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsFetch { handles, .. }) => {
            assert_eq!(handles, vec![PimdirHandle::from("1")]);
        }
        state => panic!("expected WantsFetch, got {state:?}"),
    }
}

#[test]
fn a_meta_row_holding_no_summary_is_upgraded_again() {
    let loaded = PimdirLoaded {
        placements: vec![probed("1", None, PimdirLevel::Meta)],
        checkpoint: None,
    };
    let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Meta);
    let _ = up.resume(None);

    match up.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsFetch { handles, .. }) => {
            assert_eq!(handles, vec![PimdirHandle::from("1")]);
        }
        state => panic!("expected WantsFetch, got {state:?}"),
    }
}

#[test]
fn missing_arg_errors() {
    let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Meta);
    let _ = up.resume(None);
    match up.resume(None) {
        PimdirCoroutineState::Complete(Err(PimdirArgError::MissingArg)) => {}
        state => panic!("expected MissingArg, got {state:?}"),
    }
}

/// An empty report would pass for a run that did nothing.
#[test]
fn a_completed_upgrade_does_not_resume() {
    let mut placement = probed("1", Some("x"), PimdirLevel::Full);
    placement.object = Some(PimdirHash::from("h1"));
    let loaded = PimdirLoaded {
        placements: vec![placement],
        checkpoint: None,
    };
    let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Full);
    let _ = up.resume(None);
    let _ = up.resume(Some(PimdirArg::Load(loaded)));

    match up.resume(Some(PimdirArg::Write)) {
        PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => {}
        state => panic!("expected UnexpectedArg, got {state:?}"),
    }
    match up.resume(None) {
        PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => {}
        state => panic!("expected UnexpectedArg, got {state:?}"),
    }
}

#[test]
fn unexpected_arg_errors() {
    let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Meta);
    let _ = up.resume(None);
    match up.resume(Some(PimdirArg::Write)) {
        PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => {}
        state => panic!("expected UnexpectedArg, got {state:?}"),
    }
}

#[test]
fn unknown_handle_completes_without_work() {
    let loaded = PimdirLoaded {
        placements: vec![probed("1", None, PimdirLevel::Probed)],
        checkpoint: None,
    };
    let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("nope")], PimdirTier::Meta);
    let _ = up.resume(None);

    match up.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Complete(Ok(report)) => assert_eq!(report.upgraded, 0),
        state => panic!("expected Complete(Ok), got {state:?}"),
    }
}

#[test]
fn full_without_link_ids_fetches_directly() {
    let loaded = PimdirLoaded {
        placements: vec![probed("1", None, PimdirLevel::Probed)],
        checkpoint: None,
    };
    let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Full);
    let _ = up.resume(None);

    match up.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsFetch { tier, handles, .. }) => {
            assert_eq!(tier, PimdirTier::Full);
            assert_eq!(handles, vec![PimdirHandle::from("1")]);
        }
        state => panic!("expected WantsFetch Full, got {state:?}"),
    }
}

#[test]
fn fetched_unknown_handle_is_skipped() {
    let loaded = PimdirLoaded {
        placements: vec![probed("1", None, PimdirLevel::Probed)],
        checkpoint: None,
    };
    let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Meta);
    let _ = up.resume(None);
    let _ = up.resume(Some(PimdirArg::Load(loaded)));

    let items = vec![PimdirFetchedItem {
        sort_key: Default::default(),
        handle: PimdirHandle::from("ghost"),
        link_id: PimdirLinkId::from("msg-x"),
        summary: Some(crate::summary::stub("hdr")),
        body: None,
        revision: None,
    }];
    let ops = match up.resume(Some(PimdirArg::Fetch(items))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };
    assert!(ops.is_empty(), "nothing to write: {ops:?}");

    match up.resume(Some(PimdirArg::Write)) {
        PimdirCoroutineState::Complete(Ok(report)) => assert_eq!(report.upgraded, 0),
        state => panic!("expected Complete(Ok), got {state:?}"),
    }
}

#[test]
fn full_mixes_dedup_hits_and_fetch_misses() {
    crate::testlog::init();
    let loaded = PimdirLoaded {
        placements: vec![
            probed("1", Some("msg-a"), PimdirLevel::Meta),
            probed("2", Some("msg-b"), PimdirLevel::Meta),
        ],
        checkpoint: None,
    };
    let mut up = PimdirUpgrade::new(
        "inbox",
        vec![PimdirHandle::from("1"), PimdirHandle::from("2")],
        PimdirTier::Full,
    );
    let _ = up.resume(None);
    let _ = up.resume(Some(PimdirArg::Load(loaded)));

    let mut known = BTreeMap::new();
    known.insert(PimdirLinkId::from("msg-a"), PimdirHash::from("h-a"));

    let handles = match up.resume(Some(PimdirArg::LookupObject(known))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsFetch { handles, .. }) => handles,
        state => panic!("expected WantsFetch for the miss, got {state:?}"),
    };
    assert_eq!(
        handles,
        vec![PimdirHandle::from("2")],
        "only the miss fetches"
    );

    let items = vec![PimdirFetchedItem {
        sort_key: Default::default(),
        handle: PimdirHandle::from("2"),
        link_id: PimdirLinkId::from("msg-b"),
        summary: Some(crate::summary::stub("hdr")),
        body: Some(PimdirFetchedBody::Inline {
            hash: PimdirHash::from("h-b"),
            bytes: b"body".to_vec(),
        }),
        revision: None,
    }];
    let _ = up.resume(Some(PimdirArg::Fetch(items)));

    let report = match up.resume(Some(PimdirArg::Write)) {
        PimdirCoroutineState::Complete(Ok(report)) => report,
        state => panic!("expected Complete(Ok), got {state:?}"),
    };
    assert_eq!(report.upgraded, 2);
    assert_eq!(report.deduped, 1);
    assert_eq!(report.fetched, 1);
}

/// A placement reconciled once: based, summarised, at `revision`.
fn based(handle: &str, link: &str, revision: Option<&str>) -> PimdirPlacement {
    let mut placement = probed(handle, Some(link), PimdirLevel::Meta);
    placement.base = Some(PimdirBase {
        flags: PimdirFlags::default(),
        revision: revision.map(String::from),
        object: None,
    });
    placement
}

/// Runs a full upgrade of one placement past the link lookup.
///
/// The lookup is answered with `known`.
fn upgrade_with_lookup(
    placement: PimdirPlacement,
    known: BTreeMap<PimdirLinkId, PimdirHash>,
) -> PimdirCoroutineState<PimdirYield, Result<PimdirUpgradeReport, PimdirArgError>> {
    let handle = placement.handle.clone();
    let loaded = PimdirLoaded {
        placements: vec![placement],
        checkpoint: None,
    };
    let mut up = PimdirUpgrade::new("inbox", vec![handle], PimdirTier::Full);
    let _ = up.resume(None);

    match up.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsLookupObject(_)) => {
            up.resume(Some(PimdirArg::LookupObject(known)))
        }
        state => state,
    }
}

/// A base left behind would read as a local edit on every sync.
#[test]
fn a_deduped_body_rebases_so_the_placement_reads_clean() {
    let known = BTreeMap::from([(PimdirLinkId::from("msg-a"), PimdirHash::from("h-a"))]);

    let ops = match upgrade_with_lookup(based("2", "msg-a", None), known) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite (no fetch), got {state:?}"),
    };
    let PimdirWriteOp::UpsertPlacement(placement) = &ops[0] else {
        panic!("expected UpsertPlacement, got {:?}", ops[0]);
    };

    assert_eq!(placement.object, Some(PimdirHash::from("h-a")));
    assert_eq!(placement.level, PimdirLevel::Full);
    assert_eq!(
        placement.base.as_ref().and_then(|base| base.object.clone()),
        Some(PimdirHash::from("h-a")),
        "the base holds the linked body, so nothing reads as edited"
    );
}

#[test]
fn a_mutable_placement_is_fetched_rather_than_linked() {
    let known = BTreeMap::from([(PimdirLinkId::from("uid:card-1"), PimdirHash::from("h-a"))]);

    let state = upgrade_with_lookup(based("card-1.vcf", "uid:card-1", Some("etag-1")), known);

    match state {
        PimdirCoroutineState::Yielded(PimdirYield::WantsFetch { handles, tier, .. }) => {
            assert_eq!(tier, PimdirTier::Full);
            assert_eq!(handles, vec![PimdirHandle::from("card-1.vcf")]);
        }
        state => panic!("expected WantsFetch, got {state:?}"),
    }
}

/// A based, mutable card holding the local side of a divergence.
fn conflicted(conflict_object: Option<&str>) -> PimdirPlacement {
    let mut placement = based("card-1.vcf", "uid:card-1", Some("etag-1"));
    placement.object = Some(PimdirHash::from("h-local"));
    placement.level = PimdirLevel::Full;
    placement.status = PimdirStatus::Conflict;
    placement.conflict_revision = Some(String::from("etag-2"));
    placement.conflict_object = conflict_object.map(PimdirHash::from);
    placement
}

/// Runs a full upgrade of one placement up to its yield after the load.
fn upgrade_full(
    placement: PimdirPlacement,
) -> (
    PimdirUpgrade,
    PimdirCoroutineState<PimdirYield, Result<PimdirUpgradeReport, PimdirArgError>>,
) {
    let handle = placement.handle.clone();
    let loaded = PimdirLoaded {
        placements: vec![placement],
        checkpoint: None,
    };
    let mut up = PimdirUpgrade::new("inbox", vec![handle], PimdirTier::Full);
    let _ = up.resume(None);
    let state = up.resume(Some(PimdirArg::Load(loaded)));

    (up, state)
}

/// It reads full and holds a body, so the level rule would skip it.
#[test]
fn a_conflicted_placement_asks_for_the_diverging_body() {
    let (_up, state) = upgrade_full(conflicted(None));

    match state {
        PimdirCoroutineState::Yielded(PimdirYield::WantsFetch { handles, tier, .. }) => {
            assert_eq!(tier, PimdirTier::Full);
            assert_eq!(handles, vec![PimdirHandle::from("card-1.vcf")]);
        }
        state => panic!("expected WantsFetch, got {state:?}"),
    }

    let (_up, state) = upgrade_full(conflicted(Some("h-remote")));

    match state {
        PimdirCoroutineState::Complete(Ok(report)) => assert_eq!(report.fetched, 0),
        state => panic!("expected Complete(Ok), got {state:?}"),
    }
}

/// Read as the local body, it would drop the edit under conflict.
#[test]
fn a_fetched_body_lands_as_the_conflict_object() {
    let (mut up, _state) = upgrade_full(conflicted(None));

    let items = vec![PimdirFetchedItem {
        handle: PimdirHandle::from("card-1.vcf"),
        link_id: PimdirLinkId::from("uid:card-1"),
        summary: Some(crate::summary::stub("remote")),
        sort_key: Default::default(),
        body: Some(PimdirFetchedBody::Inline {
            hash: PimdirHash::from("h-remote"),
            bytes: b"remote".to_vec(),
        }),
        revision: Some(String::from("etag-2")),
    }];

    let ops = match up.resume(Some(PimdirArg::Fetch(items))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };
    let PimdirWriteOp::UpsertPlacement(placement) = &ops[1] else {
        panic!("expected UpsertPlacement, got {:?}", ops[1]);
    };

    assert_eq!(
        placement.conflict_object,
        Some(PimdirHash::from("h-remote"))
    );
    assert_eq!(
        placement.object,
        Some(PimdirHash::from("h-local")),
        "the local side of the divergence is untouched"
    );
    assert_eq!(
        placement
            .base
            .as_ref()
            .and_then(|base| base.revision.clone()),
        Some(String::from("etag-1")),
        "nor does the fetch rebase what it never merged"
    );
}
