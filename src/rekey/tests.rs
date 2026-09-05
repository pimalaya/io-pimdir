use alloc::{format, vec};

use crate::{
    load::PimdirLoaded,
    object::PimdirHash,
    placement::PimdirOrigin,
    rekey::*,
    remote::{PimdirFetchedItem, PimdirRemoteSnapshot},
};

/// An old-spine placement, synced clean at base `flags`.
fn synced(handle: &str, link: &str, flags: &[&str]) -> PimdirPlacement {
    PimdirPlacement {
        sort_key: Default::default(),
        collection: "inbox".into(),
        handle: PimdirHandle::from(handle),
        link_id: Some(PimdirLinkId::from(link)),
        object: None,
        level: PimdirLevel::Meta,
        summary: Some(crate::summary::stub("row")),
        flags: PimdirFlags::from_iter(flags.iter().copied()),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags: PimdirFlags::from_iter(flags.iter().copied()),
            revision: None,
            object: None,
        }),
        origin: None,
    }
}

fn item(handle: &str, flags: &[&str]) -> PimdirRemoteItem {
    PimdirRemoteItem {
        handle: PimdirHandle::from(handle),
        flags: PimdirFlags::from_iter(flags.iter().copied()),
        revision: None,
    }
}

fn fetched(handle: &str, link: &str) -> PimdirFetchedItem {
    PimdirFetchedItem {
        sort_key: Default::default(),
        handle: PimdirHandle::from(handle),
        link_id: PimdirLinkId::from(link),
        summary: Some(crate::summary::stub("fresh row")),
        body: None,
        revision: None,
    }
}

/// Runs a rekey over an old spine, a new spine and its meta replies.
fn run(
    old: Vec<PimdirPlacement>,
    items: Vec<PimdirRemoteItem>,
    metas: Vec<PimdirFetchedItem>,
) -> (Vec<PimdirWriteOp>, PimdirRekeyReport) {
    crate::testlog::init();
    let mut rekey = PimdirRekey::new("inbox");
    let _ = rekey.resume(None);
    let _ = rekey.resume(Some(PimdirArg::Load(PimdirLoaded {
        placements: old,
        checkpoint: None,
    })));

    let snapshot = PimdirRemoteSnapshot {
        items,
        vanished: Vec::new(),
        complete: true,
        checkpoint: PimdirCheckpoint(b"v2".to_vec()),
    };
    let writes = match rekey.resume(Some(PimdirArg::Enumerate(snapshot))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsFetch { tier, .. }) => {
            assert_eq!(tier, PimdirTier::Meta);
            match rekey.resume(Some(PimdirArg::Fetch(metas))) {
                PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(w)) => w,
                state => panic!("expected WantsWrite, got {state:?}"),
            }
        }
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(w)) => w,
        state => panic!("expected fetch or write, got {state:?}"),
    };

    let report = match rekey.resume(Some(PimdirArg::Write)) {
        PimdirCoroutineState::Complete(Ok(report)) => report,
        state => panic!("expected Complete(Ok), got {state:?}"),
    };
    (writes, report)
}

fn upserted<'a>(writes: &'a [PimdirWriteOp], handle: &str) -> Option<&'a PimdirPlacement> {
    writes.iter().find_map(|w| match w {
        PimdirWriteOp::UpsertPlacement(p) if p.handle.as_str() == handle => Some(p),
        _ => None,
    })
}

/// The handles the batch both drops and writes.
///
/// The one way a rekey can depend on the order a storage applies it.
fn dropped_and_upserted(writes: &[PimdirWriteOp]) -> Vec<&str> {
    writes
        .iter()
        .filter_map(|w| match w {
            PimdirWriteOp::DropPlacement { handle, .. } => Some(handle.as_str()),
            _ => None,
        })
        .filter(|handle| upserted(writes, handle).is_some())
        .collect()
}

/// The common case: a server renumbering into the same handle range.
#[test]
fn a_reused_handle_is_not_dropped_by_the_batch_that_writes_it() {
    let old = synced("1", "a", &[]);
    let (writes, report) = run(vec![old], vec![item("1", &[])], vec![fetched("1", "a")]);

    assert_eq!(report.rekeyed, 1, "the item is carried over");
    assert!(
        upserted(&writes, "1").is_some(),
        "the new spine holds it: {writes:?}",
    );
    assert_eq!(
        dropped_and_upserted(&writes),
        Vec::<&str>::new(),
        "the batch decides by apply order: {writes:?}",
    );
}

/// Same hazard without reuse: the edit resurrects under its old handle.
#[test]
fn a_resurrected_edit_is_not_dropped_by_the_batch_that_writes_it() {
    let mut old = synced("1", "a", &[]);
    old.status = PimdirStatus::Dirty;
    old.object = Some(PimdirHash::from("h2"));
    old.level = PimdirLevel::Full;
    old.base = Some(PimdirBase {
        flags: PimdirFlags::default(),
        revision: None,
        object: Some(PimdirHash::from("h1")),
    });

    let (writes, _report) = run(
        vec![old],
        vec![item("101", &[])],
        vec![fetched("101", "other")],
    );

    let resurrected = upserted(&writes, "1").expect("the edit survives as a create");
    assert_eq!(resurrected.status, PimdirStatus::Created);
    assert_eq!(
        dropped_and_upserted(&writes),
        Vec::<&str>::new(),
        "the local edit survives only if the drop is applied first: {writes:?}",
    );
}

#[test]
fn a_pending_flag_delta_survives_the_bump() {
    let mut old = synced("1", "msg-a", &["seen"]);
    old.flags = PimdirFlags::from_iter(["seen", "flagged"]);
    old.status = PimdirStatus::Dirty;

    let (writes, report) = run(
        vec![old],
        vec![item("101", &["seen"])],
        vec![fetched("101", "msg-a")],
    );

    assert_eq!(report.rekeyed, 1);
    assert_eq!(report.dropped, 0);
    assert!(
        writes.iter().any(
            |w| matches!(w, PimdirWriteOp::DropPlacement { handle, .. } if handle.as_str() == "1")
        ),
        "the old handle is dropped: {writes:?}",
    );
    let carried = upserted(&writes, "101").expect("a carried placement");
    assert_eq!(
        carried.status,
        PimdirStatus::Dirty,
        "the delta stays pending"
    );
    assert!(carried.flags.contains("flagged"));
    let base = carried.base.as_ref().expect("a base");
    assert!(base.flags.contains("seen") && !base.flags.contains("flagged"));
}

#[test]
fn a_tombstone_survives_with_its_destination() {
    let mut old = synced("1", "msg-a", &["seen"]);
    old.status = PimdirStatus::Tombstone;
    old.origin = Some(PimdirOrigin {
        collection: "archive".into(),
        handle: PimdirHandle::from("1"),
    });

    let (writes, report) = run(
        vec![old],
        vec![item("101", &["seen"])],
        vec![fetched("101", "msg-a")],
    );

    assert_eq!(report.rekeyed, 1);
    let carried = upserted(&writes, "101").expect("a carried placement");
    assert_eq!(carried.status, PimdirStatus::Tombstone);
    assert_eq!(
        carried.origin.as_ref().expect("a move target").collection,
        "archive".into(),
    );
}

#[test]
fn a_staged_edit_survives_with_its_body() {
    let mut old = synced("1", "msg-a", &[]);
    old.object = Some(PimdirHash::from("h2"));
    old.level = PimdirLevel::Full;
    old.status = PimdirStatus::Dirty;

    let (writes, report) = run(
        vec![old],
        vec![item("101", &[])],
        vec![fetched("101", "msg-a")],
    );

    assert_eq!(report.rekeyed, 1);
    let carried = upserted(&writes, "101").expect("a carried placement");
    assert_eq!(carried.status, PimdirStatus::Dirty);
    assert_eq!(
        carried.object,
        Some(PimdirHash::from("h2")),
        "the body survives"
    );
    assert_eq!(carried.level, PimdirLevel::Full, "the cache survives");
}

#[test]
fn a_clean_cache_carries_over_without_pending_state() {
    let mut old = synced("1", "msg-a", &["seen"]);
    old.object = Some(PimdirHash::from("h1"));
    old.base.as_mut().expect("a base").object = Some(PimdirHash::from("h1"));
    old.level = PimdirLevel::Full;

    let (writes, report) = run(
        vec![old],
        vec![item("101", &["seen"])],
        vec![fetched("101", "msg-a")],
    );

    assert_eq!(report.rekeyed, 1);
    let carried = upserted(&writes, "101").expect("a carried placement");
    assert_eq!(carried.status, PimdirStatus::Clean);
    assert_eq!(carried.object, Some(PimdirHash::from("h1")));
    assert_eq!(carried.level, PimdirLevel::Full);
    let base = carried.base.as_ref().expect("a base");
    assert_eq!(base.object, Some(PimdirHash::from("h1")));
}

#[test]
fn an_unmatched_staged_edit_resurrects_as_a_pending_create() {
    let mut old = synced("1", "msg-a", &[]);
    old.object = Some(PimdirHash::from("h2"));
    old.level = PimdirLevel::Full;
    old.status = PimdirStatus::Dirty;

    let (writes, report) = run(vec![old], vec![], vec![]);

    assert_eq!(report.rekeyed, 1, "carried as a pending create");
    assert_eq!(report.dropped, 0);
    let resurrected = upserted(&writes, "1").expect("a resurrected placement");
    assert_eq!(resurrected.status, PimdirStatus::Created);
    assert!(resurrected.base.is_none());
    assert_eq!(resurrected.object, Some(PimdirHash::from("h2")));
}

/// A probed-only placement has no link id to match on.
#[test]
fn unmatched_pending_state_is_dropped_and_counted() {
    let mut old = synced("1", "msg-a", &[]);
    old.link_id = None;
    old.flags = PimdirFlags::from_iter(["flagged"]);
    old.status = PimdirStatus::Dirty;

    let (writes, report) = run(vec![old], vec![item("101", &[])], vec![]);

    assert_eq!(report.rekeyed, 0);
    assert_eq!(report.pulled, 1);
    assert_eq!(report.dropped, 1, "the pending edit is lost, and said so");
    let fresh = upserted(&writes, "101").expect("a fresh placement");
    assert_eq!(fresh.status, PimdirStatus::Clean);
}

#[test]
fn no_link_ids_skips_the_meta_fetch() {
    let mut old = synced("1", "msg-a", &[]);
    old.link_id = None;

    let mut rekey = PimdirRekey::new("inbox");
    let _ = rekey.resume(None);
    let _ = rekey.resume(Some(PimdirArg::Load(PimdirLoaded {
        placements: vec![old],
        checkpoint: None,
    })));

    let snapshot = PimdirRemoteSnapshot {
        items: vec![item("101", &[])],
        vanished: Vec::new(),
        complete: true,
        checkpoint: PimdirCheckpoint(b"v2".to_vec()),
    };
    match rekey.resume(Some(PimdirArg::Enumerate(snapshot))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(_)) => {}
        state => panic!("expected WantsWrite without a fetch, got {state:?}"),
    }
}

#[test]
fn pending_creates_are_left_untouched() {
    let mut placeholder = synced("tmp-1", "msg-b", &[]);
    placeholder.status = PimdirStatus::Created;
    placeholder.base = None;

    let (writes, report) = run(vec![placeholder], vec![], vec![]);

    assert_eq!(report.rekeyed + report.pulled + report.dropped, 0);
    assert!(
        !writes.iter().any(|w| matches!(
            w,
            PimdirWriteOp::DropPlacement { handle, .. } if handle.as_str() == "tmp-1"
        )),
        "the placeholder is not spine, it stays: {writes:?}",
    );
}

#[test]
fn missing_arg_errors() {
    let mut rekey = PimdirRekey::new("inbox");
    let _ = rekey.resume(None);
    match rekey.resume(None) {
        PimdirCoroutineState::Complete(Err(PimdirArgError::MissingArg)) => {}
        state => panic!("expected MissingArg, got {state:?}"),
    }
}

/// An empty report would pass for a run that did nothing.
#[test]
fn a_completed_rekey_does_not_resume() {
    let mut rekey = PimdirRekey::new("inbox");
    let _ = rekey.resume(None);
    let _ = rekey.resume(Some(PimdirArg::Load(PimdirLoaded::default())));
    let _ = rekey.resume(Some(PimdirArg::Enumerate(PimdirRemoteSnapshot {
        items: Vec::new(),
        vanished: Vec::new(),
        complete: true,
        checkpoint: PimdirCheckpoint(b"v2".to_vec()),
    })));
    let _ = rekey.resume(Some(PimdirArg::Write));

    match rekey.resume(Some(PimdirArg::Write)) {
        PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => {}
        state => panic!("expected UnexpectedArg, got {state:?}"),
    }
    match rekey.resume(None) {
        PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => {}
        state => panic!("expected UnexpectedArg, got {state:?}"),
    }
}

/// A revision-carrying member of the old spine, synced clean at `r1`.
fn mutable(handle: &str, link: &str) -> PimdirPlacement {
    let mut old = synced(handle, link, &[]);
    old.object = Some(PimdirHash::from("h1"));
    old.level = PimdirLevel::Full;
    old.base = Some(PimdirBase {
        flags: PimdirFlags::default(),
        revision: Some("r1".into()),
        object: Some(PimdirHash::from("h1")),
    });
    old
}

fn item_rev(handle: &str, revision: &str) -> PimdirRemoteItem {
    let mut item = item(handle, &[]);
    item.revision = Some(revision.into());
    item
}

/// A remote edit over a clean placement is carried as the pull a sync
/// would make: body dropped, level lowered, base at the fetched revision
/// with no body (SYNC §8).
#[test]
fn a_remote_edit_over_a_clean_placement_is_carried_as_a_pull() {
    let (writes, report) = run(
        vec![mutable("a.vcf", "uid:1")],
        vec![item_rev("b.vcf", "r2")],
        vec![fetched("b.vcf", "uid:1")],
    );

    assert_eq!(report.rekeyed, 1);
    let carried = upserted(&writes, "b.vcf").expect("a carried placement");
    assert_eq!(carried.status, PimdirStatus::Clean);
    assert_eq!(carried.object, None, "the stale body is dropped");
    assert_eq!(carried.level, PimdirLevel::Probed);
    let base = carried.base.as_ref().expect("a base");
    assert_eq!(base.revision.as_deref(), Some("r2"));
    assert_eq!(base.object, None);
}

/// A remote edit over a local edit is a conflict at the fetched revision,
/// the base untouched: a base at `r2` would push the edit last-writer-wins.
#[test]
fn a_remote_edit_over_a_local_edit_is_carried_as_a_conflict() {
    let mut old = mutable("a.vcf", "uid:1");
    old.object = Some(PimdirHash::from("h2"));
    old.status = PimdirStatus::Dirty;

    let (writes, report) = run(
        vec![old],
        vec![item_rev("b.vcf", "r2")],
        vec![fetched("b.vcf", "uid:1")],
    );

    assert_eq!(report.rekeyed, 1);
    let carried = upserted(&writes, "b.vcf").expect("a carried placement");
    assert_eq!(carried.status, PimdirStatus::Conflict);
    assert_eq!(carried.conflict_revision.as_deref(), Some("r2"));
    assert_eq!(carried.conflict_object, None, "asked for by the upgrade");
    assert_eq!(
        carried.object,
        Some(PimdirHash::from("h2")),
        "the edit stays"
    );
    let base = carried.base.as_ref().expect("a base");
    assert_eq!(base.revision.as_deref(), Some("r1"), "never reconciled r2");
    assert_eq!(base.object, Some(PimdirHash::from("h1")));
}

/// An unchanged revision carries the placement whole, base included.
#[test]
fn an_unchanged_revision_carries_the_base_as_it_was() {
    let (writes, _report) = run(
        vec![mutable("a.vcf", "uid:1")],
        vec![item_rev("b.vcf", "r1")],
        vec![fetched("b.vcf", "uid:1")],
    );

    let carried = upserted(&writes, "b.vcf").expect("a carried placement");
    assert_eq!(carried.status, PimdirStatus::Clean);
    assert_eq!(carried.object, Some(PimdirHash::from("h1")));
    let base = carried.base.as_ref().expect("a base");
    assert_eq!(base.revision.as_deref(), Some("r1"));
    assert_eq!(base.object, Some(PimdirHash::from("h1")));
}

/// A tombstone meeting a remote edit is revived and pulled, as a sync
/// would: a delta never relists the member, so nothing else could.
#[test]
fn a_tombstone_over_a_remote_edit_is_revived() {
    let mut old = mutable("a.vcf", "uid:1");
    old.status = PimdirStatus::Tombstone;
    old.origin = Some(PimdirOrigin {
        collection: "archive".into(),
        handle: PimdirHandle::from("a.vcf"),
    });

    let (writes, _report) = run(
        vec![old],
        vec![item_rev("b.vcf", "r2")],
        vec![fetched("b.vcf", "uid:1")],
    );

    let carried = upserted(&writes, "b.vcf").expect("a carried placement");
    assert_eq!(carried.status, PimdirStatus::Clean);
    assert_eq!(carried.object, None);
    assert_eq!(carried.origin, None, "the move is abandoned");
    let base = carried.base.as_ref().expect("a base");
    assert_eq!(base.revision.as_deref(), Some("r2"));
    assert_eq!(base.object, None);
}

/// A large mailbox is resolved in bounded meta fetches, one write.
#[test]
fn the_meta_fetch_goes_in_chunks() {
    let extra = 5;
    let count = PimdirRekey::FETCH_CHUNK + extra;
    let old: Vec<PimdirPlacement> = (0..count)
        .map(|i| synced(&format!("{i:04}"), &format!("m{i}"), &[]))
        .collect();
    let items: Vec<PimdirRemoteItem> = (0..count)
        .map(|i| item(&format!("v2-{i:04}"), &[]))
        .collect();

    crate::testlog::init();
    let mut rekey = PimdirRekey::new("inbox");
    let _ = rekey.resume(None);
    let _ = rekey.resume(Some(PimdirArg::Load(PimdirLoaded {
        placements: old,
        checkpoint: None,
    })));
    let mut state = rekey.resume(Some(PimdirArg::Enumerate(PimdirRemoteSnapshot {
        items,
        vanished: Vec::new(),
        complete: true,
        checkpoint: PimdirCheckpoint(b"v2".to_vec()),
    })));

    let mut chunks = Vec::new();
    let writes = loop {
        match state {
            PimdirCoroutineState::Yielded(PimdirYield::WantsFetch { handles, .. }) => {
                let metas = handles
                    .iter()
                    .map(|h| {
                        fetched(
                            h.as_str(),
                            &format!("m{}", &h.as_str()[3..].parse::<usize>().unwrap()),
                        )
                    })
                    .collect();
                chunks.push(handles.len());
                state = rekey.resume(Some(PimdirArg::Fetch(metas)));
            }
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(writes)) => break writes,
            state => panic!("expected fetch or write, got {state:?}"),
        }
    };

    assert_eq!(chunks, [PimdirRekey::FETCH_CHUNK, extra]);
    let report = match rekey.resume(Some(PimdirArg::Write)) {
        PimdirCoroutineState::Complete(Ok(report)) => report,
        state => panic!("expected Complete(Ok), got {state:?}"),
    };
    assert_eq!(report.rekeyed, count, "every member found its new handle");
    assert_eq!(report.pulled, 0);
    assert_eq!(
        writes
            .iter()
            .filter(|w| matches!(w, PimdirWriteOp::SetCheckpoint { .. }))
            .count(),
        1,
    );
}

#[test]
fn unexpected_arg_errors() {
    let mut rekey = PimdirRekey::new("inbox");
    let _ = rekey.resume(None);
    match rekey.resume(Some(PimdirArg::Write)) {
        PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => {}
        state => panic!("expected UnexpectedArg, got {state:?}"),
    }
}

/// A minted key is a key: the rebuild reads nothing into its shape.
#[test]
fn a_minted_key_is_carried_over_a_handle_space_change() {
    let mut minted = synced("8", "dup:m1#8", &[]);
    minted.flags = PimdirFlags::from_iter(["seen"]);
    minted.status = PimdirStatus::Dirty;

    let (writes, report) = run(
        vec![synced("7", "m1", &[]), minted],
        vec![item("v2-0", &[]), item("v2-1", &[])],
        vec![fetched("v2-0", "m1"), fetched("v2-1", "dup:m1#8")],
    );

    assert_eq!(report.rekeyed, 2);
    assert_eq!(report.dropped, 0, "no pending state was lost");
    let carried = |handle: &str| {
        writes
            .iter()
            .find_map(|op| match op {
                PimdirWriteOp::UpsertPlacement(p) if p.handle.as_str() == handle => Some(p),
                _ => None,
            })
            .expect("the carried placement")
            .clone()
    };

    assert_eq!(carried("v2-0").link_id, Some(PimdirLinkId::from("m1")));
    let copy = carried("v2-1");
    assert_eq!(copy.link_id, Some(PimdirLinkId::from("dup:m1#8")));
    assert_eq!(
        copy.status,
        PimdirStatus::Dirty,
        "and its pending push survives the rebuild like any other",
    );
}

/// Both copies resolve to the shared hint, never to the minted key.
#[test]
fn two_copies_of_one_hint_are_carried_apart() {
    let mut first = synced("7", "m1", &[]);
    first.object = Some(PimdirHash::from("h1"));
    first.level = PimdirLevel::Full;
    let mut second = synced("8", "dup:m1#8", &[]);
    second.object = Some(PimdirHash::from("h2"));
    second.level = PimdirLevel::Full;

    let (writes, report) = run(
        vec![first, second],
        vec![item("v2-0", &[]), item("v2-1", &[])],
        vec![fetched("v2-0", "m1"), fetched("v2-1", "m1")],
    );

    assert_eq!(report.rekeyed, 2, "both copies are carried, not merged");
    assert_eq!(report.pulled, 0);
    let carried = |handle: &str| upserted(&writes, handle).expect("a carried placement");
    assert_eq!(carried("v2-0").link_id, Some(PimdirLinkId::from("m1")));
    assert_eq!(carried("v2-0").object, Some(PimdirHash::from("h1")));
    assert_eq!(
        carried("v2-1").link_id,
        Some(PimdirLinkId::from("dup:m1#8")),
        "the second copy keeps the key it was minted under",
    );
    assert_eq!(
        carried("v2-1").object,
        Some(PimdirHash::from("h2")),
        "with its own body, which is the copy nobody would see again",
    );
}

/// So a store rebuilt from scratch converges on the same keys.
#[test]
fn a_new_copy_of_one_hint_is_minted_from_its_own_handle() {
    let (writes, _report) = run(
        vec![synced("7", "m1", &[])],
        vec![item("v2-0", &[]), item("v2-1", &[])],
        vec![fetched("v2-0", "m1"), fetched("v2-1", "m1")],
    );

    let minted = upserted(&writes, "v2-1").expect("the second copy");
    assert_eq!(minted.link_id, Some(PimdirLinkId::from("dup:m1#v2-1")));
}

#[test]
fn a_rebuild_leaves_a_pending_create_its_key() {
    let mut placeholder = synced("tmp-1", "m1", &[]);
    placeholder.status = PimdirStatus::Created;
    placeholder.base = None;

    let (writes, _report) = run(
        vec![placeholder],
        vec![item("v2-0", &[])],
        vec![fetched("v2-0", "m1")],
    );

    let member = upserted(&writes, "v2-0").expect("the rebuilt member");
    assert_eq!(member.link_id, Some(PimdirLinkId::from("dup:m1#v2-0")));
}

/// A storage sharing items across sources has to tell the two apart.
#[test]
fn a_rebuild_says_which_rows_are_gone_and_which_moved() {
    let (writes, _report) = run(
        vec![synced("7", "m1", &[]), synced("8", "m2", &[])],
        vec![item("v2-0", &[])],
        vec![fetched("v2-0", "m1")],
    );

    let dropped: Vec<(&str, PimdirDropReason)> = writes
        .iter()
        .filter_map(|op| match op {
            PimdirWriteOp::DropPlacement { handle, reason, .. } => Some((handle.as_str(), *reason)),
            _ => None,
        })
        .collect();
    assert_eq!(
        dropped,
        vec![
            ("7", PimdirDropReason::Rekeyed),
            ("8", PimdirDropReason::Deleted),
        ],
    );
}
