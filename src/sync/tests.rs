use alloc::{format, vec, vec::Vec};

use crate::{
    load::PimdirLoaded,
    object::PimdirHash,
    placement::{PimdirLinkId, PimdirOrigin},
    remote::{PimdirPushOutcome, PimdirPushResult, PimdirRemoteSnapshot},
    sync::*,
};

/// A pending create staged in "inbox", its body sourced from "sent".
fn created(handle: &str) -> PimdirPlacement {
    PimdirPlacement {
        sort_key: Default::default(),
        collection: "inbox".into(),
        handle: PimdirHandle::from(handle),
        link_id: None,
        object: None,
        level: PimdirLevel::Probed,
        summary: None,
        flags: PimdirFlags::default(),
        status: PimdirStatus::Created,
        conflict_revision: None,
        conflict_object: None,
        base: None,
        origin: Some(PimdirOrigin {
            collection: "sent".into(),
            handle: PimdirHandle::from("9"),
        }),
    }
}

fn synced(handle: &str, flags: &[&str]) -> PimdirPlacement {
    PimdirPlacement {
        sort_key: Default::default(),
        collection: "inbox".into(),
        handle: PimdirHandle::from(handle),
        link_id: Some(PimdirLinkId::from(handle)),
        object: None,
        level: PimdirLevel::Probed,
        summary: None,
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

fn remote(handle: &str, flags: &[&str]) -> PimdirRemoteItem {
    PimdirRemoteItem {
        handle: PimdirHandle::from(handle),
        flags: PimdirFlags::from_iter(flags.iter().copied()),
        revision: None,
    }
}

fn full(items: Vec<PimdirRemoteItem>) -> PimdirRemoteSnapshot {
    PimdirRemoteSnapshot {
        items,
        vanished: Vec::new(),
        complete: true,
        checkpoint: PimdirCheckpoint(b"c1".to_vec()),
    }
}

fn delta(items: Vec<PimdirRemoteItem>, vanished: Vec<PimdirHandle>) -> PimdirRemoteSnapshot {
    PimdirRemoteSnapshot {
        items,
        vanished,
        complete: false,
        checkpoint: PimdirCheckpoint(b"c1".to_vec()),
    }
}

fn run(
    sync: &mut PimdirSync,
    local: Vec<PimdirPlacement>,
    items: Vec<PimdirRemoteItem>,
) -> (
    Option<Vec<PimdirChange>>,
    Vec<PimdirWriteOp>,
    PimdirSyncReport,
) {
    run_snapshot(sync, local, full(items))
}

fn run_snapshot(
    sync: &mut PimdirSync,
    local: Vec<PimdirPlacement>,
    snapshot: PimdirRemoteSnapshot,
) -> (
    Option<Vec<PimdirChange>>,
    Vec<PimdirWriteOp>,
    PimdirSyncReport,
) {
    crate::testlog::init();
    let _ = sync.resume(None);
    let _ = sync.resume(Some(PimdirArg::Load(PimdirLoaded {
        placements: local,
        checkpoint: None,
    })));

    let mut pushes = None;
    let writes = match sync.resume(Some(PimdirArg::Enumerate(snapshot))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsPush { changes, .. }) => {
            let results = changes
                .iter()
                .map(|change| PimdirPushResult {
                    handle: change.handle().clone(),
                    outcome: PimdirPushOutcome::Accepted,
                    assigned: None,
                    revision: None,
                })
                .collect();
            pushes = Some(changes);
            match sync.resume(Some(PimdirArg::Push(results))) {
                PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(w)) => w,
                state => panic!("expected WantsWrite, got {state:?}"),
            }
        }
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(w)) => w,
        state => panic!("expected push or write, got {state:?}"),
    };

    let report = match sync.resume(Some(PimdirArg::Write)) {
        PimdirCoroutineState::Complete(Ok(report)) => report,
        state => panic!("expected Complete(Ok), got {state:?}"),
    };

    (pushes, writes, report)
}

#[test]
fn remote_add_pulls_probed() {
    crate::testlog::init();
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, report) = run(&mut sync, vec![], vec![remote("1", &["seen"])]);

    assert!(pushes.is_none());
    assert_eq!(report.pulled, 1);
    let PimdirWriteOp::UpsertPlacement(p) = &writes[0] else {
        panic!("expected UpsertPlacement, got {:?}", writes[0]);
    };
    assert_eq!(p.level, PimdirLevel::Probed);
    assert!(p.flags.contains("seen"));
}

/// Unknown markers hold no opinion (spec §13): the remote set is pulled.
#[test]
fn an_unknown_local_set_adopts_the_remote_one_and_pushes_nothing() {
    let mut local = synced("1", &[]);
    local.flags = PimdirFlags::Unknown;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

    assert!(pushes.is_none(), "nothing to push from an unknown set");
    assert_eq!(report.pulled, 1);
    let PimdirWriteOp::UpsertPlacement(p) = &writes[0] else {
        panic!("expected UpsertPlacement, got {:?}", writes[0]);
    };
    assert!(p.flags.contains("seen"));
}

#[test]
fn local_flag_change_pushes() {
    let mut local = synced("1", &[]);
    local.flags = PimdirFlags::from_iter(["seen"]);
    local.status = PimdirStatus::Dirty;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, _writes, report) = run(&mut sync, vec![local], vec![remote("1", &[])]);

    let pushes = pushes.expect("a push");
    assert!(matches!(pushes[0].kind, PimdirChangeKind::SetFlags { .. }));
    assert_eq!(report.pushed, 1);
}

/// Runs a sync through its push with the given results.
///
/// Returns the writes the engine then stages, and the report.
fn run_push(
    sync: &mut PimdirSync,
    local: Vec<PimdirPlacement>,
    items: Vec<PimdirRemoteItem>,
    results: Vec<PimdirPushResult>,
) -> (Vec<PimdirWriteOp>, PimdirSyncReport) {
    crate::testlog::init();
    let _ = sync.resume(None);
    let _ = sync.resume(Some(PimdirArg::Load(PimdirLoaded {
        placements: local,
        checkpoint: None,
    })));
    match sync.resume(Some(PimdirArg::Enumerate(full(items)))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsPush { .. }) => {}
        state => panic!("expected WantsPush, got {state:?}"),
    }
    let writes = match sync.resume(Some(PimdirArg::Push(results))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(writes)) => writes,
        state => panic!("expected WantsWrite, got {state:?}"),
    };
    let report = match sync.resume(Some(PimdirArg::Write)) {
        PimdirCoroutineState::Complete(Ok(report)) => report,
        state => panic!("expected Complete(Ok), got {state:?}"),
    };
    (writes, report)
}

/// What a run asked for, in the order it asked.
struct Run {
    /// Each push chunk, in order.
    chunks: Vec<Vec<PimdirChange>>,
    /// Each write batch, in order.
    batches: Vec<Vec<PimdirWriteOp>>,
    /// The yields as they came, so a test can pin the interleaving.
    order: Vec<&'static str>,
    report: PimdirSyncReport,
}

impl Run {
    /// Every write of the run, batch boundaries flattened away.
    fn writes(&self) -> Vec<PimdirWriteOp> {
        self.batches.iter().flatten().cloned().collect()
    }

    /// The index of the batch holding a write, if any.
    fn batch_of(&self, mut held: impl FnMut(&PimdirWriteOp) -> bool) -> Option<usize> {
        self.batches
            .iter()
            .position(|batch| batch.iter().any(&mut held))
    }
}

/// Runs a sync to completion against an accepting remote, keeping yields.
///
/// Unlike [`run`] it assumes nothing about how many pushes and
/// writes a run takes, which is what the chunked paths are about.
fn run_batches(
    sync: &mut PimdirSync,
    local: Vec<PimdirPlacement>,
    snapshot: PimdirRemoteSnapshot,
) -> Run {
    crate::testlog::init();
    let _ = sync.resume(None);
    let _ = sync.resume(Some(PimdirArg::Load(PimdirLoaded {
        placements: local,
        checkpoint: None,
    })));

    let mut run = Run {
        chunks: Vec::new(),
        batches: Vec::new(),
        order: Vec::new(),
        report: PimdirSyncReport::default(),
    };
    let mut arg = Some(PimdirArg::Enumerate(snapshot));

    loop {
        match sync.resume(arg.take()) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsPush { changes, .. }) => {
                run.order.push("push");
                let results = changes
                    .iter()
                    .map(|change| PimdirPushResult {
                        handle: change.handle().clone(),
                        outcome: PimdirPushOutcome::Accepted,
                        assigned: None,
                        revision: None,
                    })
                    .collect();
                run.chunks.push(changes);
                arg = Some(PimdirArg::Push(results));
            }
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(writes)) => {
                run.order.push("write");
                run.batches.push(writes);
                arg = Some(PimdirArg::Write);
            }
            PimdirCoroutineState::Complete(Ok(report)) => {
                run.report = report;
                return run;
            }
            state => panic!("expected push or write, got {state:?}"),
        }
    }
}

/// The last placement an UpsertPlacement op writes for `handle`, if any.
///
/// The last, since a batch applies in order and the row a store ends up
/// holding is the last write naming it.
fn upserted<'a>(writes: &'a [PimdirWriteOp], handle: &str) -> Option<&'a PimdirPlacement> {
    writes.iter().rev().find_map(|w| match w {
        PimdirWriteOp::UpsertPlacement(p) if p.handle.as_str() == handle => Some(p),
        _ => None,
    })
}

#[test]
fn rejected_flag_push_keeps_dirty() {
    let mut local = synced("1", &[]);
    local.flags = PimdirFlags::from_iter(["flagged"]);
    local.status = PimdirStatus::Dirty;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let results = vec![PimdirPushResult {
        handle: PimdirHandle::from("1"),
        outcome: PimdirPushOutcome::Rejected,
        assigned: None,
        revision: None,
    }];
    let (writes, report) = run_push(&mut sync, vec![local], vec![remote("1", &[])], results);

    assert!(
        upserted(&writes, "1").is_none(),
        "a rejected flag push must not rebase the placement: {writes:?}",
    );
    assert_eq!(report.rejected, 1);
}

#[test]
fn accepted_flag_push_rebases_clean() {
    let mut local = synced("1", &[]);
    local.flags = PimdirFlags::from_iter(["flagged"]);
    local.status = PimdirStatus::Dirty;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let results = vec![PimdirPushResult {
        handle: PimdirHandle::from("1"),
        outcome: PimdirPushOutcome::Accepted,
        assigned: None,
        revision: None,
    }];
    let (writes, _report) = run_push(&mut sync, vec![local], vec![remote("1", &[])], results);

    let rebased = upserted(&writes, "1").expect("an accepted flag push rebases the placement");
    assert_eq!(rebased.status, PimdirStatus::Clean);
    assert!(
        rebased
            .base
            .as_ref()
            .expect("a base")
            .flags
            .contains("flagged")
    );
}

/// A dirty flag placement, ready to derive one push.
fn pending(handle: &str) -> PimdirPlacement {
    let mut placement = synced(handle, &[]);
    placement.flags = PimdirFlags::from_iter(["flagged"]);
    placement.status = PimdirStatus::Dirty;
    placement
}

fn accepted(handle: &str) -> PimdirPushResult {
    PimdirPushResult {
        handle: PimdirHandle::from(handle),
        outcome: PimdirPushOutcome::Accepted,
        assigned: None,
        revision: None,
    }
}

fn rejected(handle: &str) -> PimdirPushResult {
    PimdirPushResult {
        outcome: PimdirPushOutcome::Rejected,
        ..accepted(handle)
    }
}

/// A handle nobody reported on is retried, never assumed accepted.
#[test]
fn an_unreported_push_stays_pending() {
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (writes, report) = run_push(
        &mut sync,
        vec![pending("1"), pending("2")],
        vec![remote("1", &[]), remote("2", &[])],
        vec![accepted("1")],
    );

    assert_eq!(
        upserted(&writes, "1")
            .expect("the reported push rebases")
            .status,
        PimdirStatus::Clean,
    );
    assert!(
        upserted(&writes, "2").is_none(),
        "an unreported push must stay dirty for the next run: {writes:?}",
    );
    assert_eq!(report.pushed, 1);
    assert_eq!(report.rejected, 0, "silence is not a rejection");
}

/// Results match by handle: order, strangers and duplicates do not matter.
#[test]
fn a_result_set_is_matched_by_handle_not_by_shape() {
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let results = vec![
        accepted("2"),
        accepted("nobody-pushed-this"),
        rejected("nobody-pushed-this-either"),
        accepted("1"),
        accepted("2"),
        rejected("2"),
    ];
    let (writes, report) = run_push(
        &mut sync,
        vec![pending("1"), pending("2")],
        vec![remote("1", &[]), remote("2", &[])],
        results,
    );

    for handle in ["1", "2"] {
        let rebased = upserted(&writes, handle);
        assert!(rebased.is_some_and(|p| p.status == PimdirStatus::Clean));
    }
    assert!(upserted(&writes, "nobody-pushed-this").is_none());
    assert_eq!(
        report.pushed, 2,
        "a duplicate result and an unknown handle cannot inflate the count",
    );
    assert_eq!(
        report.rejected, 0,
        "a rejection counts for a pushed handle only, and once",
    );
    assert_eq!(
        report.events.len(),
        0,
        "a pushed change reports no event: {:?}",
        report.events,
    );
}

#[test]
fn partial_push_accepts_one_rejects_other() {
    let mut one = synced("1", &[]);
    one.flags = PimdirFlags::from_iter(["flagged"]);
    one.status = PimdirStatus::Dirty;
    let mut two = synced("2", &[]);
    two.flags = PimdirFlags::from_iter(["flagged"]);
    two.status = PimdirStatus::Dirty;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let results = vec![
        PimdirPushResult {
            handle: PimdirHandle::from("1"),
            outcome: PimdirPushOutcome::Accepted,
            assigned: None,
            revision: None,
        },
        PimdirPushResult {
            handle: PimdirHandle::from("2"),
            outcome: PimdirPushOutcome::Rejected,
            assigned: None,
            revision: None,
        },
    ];
    let (writes, report) = run_push(
        &mut sync,
        vec![one, two],
        vec![remote("1", &[]), remote("2", &[])],
        results,
    );

    assert_eq!(
        upserted(&writes, "1").expect("accepted rebases").status,
        PimdirStatus::Clean,
    );
    assert!(
        upserted(&writes, "2").is_none(),
        "rejected handle must stay dirty: {writes:?}",
    );
    assert_eq!(report.pushed, 1, "only the accepted change counts");
    assert_eq!(report.rejected, 1);
}

#[test]
fn rejected_push_retries_on_next_sync() {
    let mut local = synced("1", &[]);
    local.flags = PimdirFlags::from_iter(["flagged"]);
    local.status = PimdirStatus::Dirty;

    let mut first = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let results = vec![PimdirPushResult {
        handle: PimdirHandle::from("1"),
        outcome: PimdirPushOutcome::Rejected,
        assigned: None,
        revision: None,
    }];
    let (writes, _report) = run_push(
        &mut first,
        vec![local.clone()],
        vec![remote("1", &[])],
        results,
    );
    assert!(upserted(&writes, "1").is_none(), "rejected push left dirty");

    let mut second = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, _writes, report) = run(&mut second, vec![local], vec![remote("1", &[])]);
    let pushes = pushes.expect("the dirty change is pushed again");
    assert!(matches!(pushes[0].kind, PimdirChangeKind::SetFlags { .. }));
    assert_eq!(report.pushed, 1);
}

/// Each chunk is recorded before the next is pushed, bounding the replay.
#[test]
fn a_chunk_is_recorded_before_the_next_one_is_pushed() {
    let extra = 3;
    let count = PimdirSync::PUSH_CHUNK + extra;

    let mut local = Vec::new();
    let mut items = Vec::new();
    for index in 0..count {
        let handle = format!("{index:03}");
        let mut placement = synced(&handle, &[]);
        placement.flags = PimdirFlags::from_iter(["seen"]);
        placement.status = PimdirStatus::Dirty;
        local.push(placement);
        items.push(remote(&handle, &[]));
    }

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let run = run_batches(&mut sync, local, full(items));

    assert_eq!(run.order, ["push", "write", "push", "write"]);
    assert_eq!(run.chunks[0].len(), PimdirSync::PUSH_CHUNK);
    assert_eq!(run.chunks[1].len(), extra);

    for change in &run.chunks[0] {
        let rebased = upserted(&run.batches[0], change.handle().as_str());
        assert!(rebased.is_some_and(|p| p.status == PimdirStatus::Clean));
    }
    for change in &run.chunks[1] {
        let handle = change.handle().as_str();
        assert!(upserted(&run.batches[0], handle).is_none());
        let rebased = upserted(&run.batches[1], handle);
        assert!(rebased.is_some_and(|p| p.status == PimdirStatus::Clean));
    }

    assert!(
        run.batch_of(|op| matches!(op, PimdirWriteOp::SetCheckpoint { .. })) == Some(1),
        "the checkpoint must land in the closing batch",
    );

    assert_eq!(run.report.pushed, count);
}

/// An unordered enumeration derives exactly what the sorted one derives.
#[test]
fn an_unordered_enumeration_merges_like_an_ordered_one() {
    let local = || {
        let mut dirty = synced("5", &[]);
        dirty.flags = PimdirFlags::from_iter(["flagged"]);
        dirty.status = PimdirStatus::Dirty;
        vec![synced("1", &[]), synced("2", &[]), synced("3", &[]), dirty]
    };
    let items = || {
        vec![
            remote("1", &["seen"]),
            remote("3", &[]),
            remote("4", &[]),
            remote("5", &[]),
        ]
    };

    let mut ordered = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let ordered = run_batches(&mut ordered, local(), full(items()));

    let mut shuffled = items();
    shuffled.reverse();
    let mut unordered = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let unordered = run_batches(&mut unordered, local(), full(shuffled));

    assert_eq!(unordered.chunks, ordered.chunks, "different pushes");
    assert_eq!(unordered.writes(), ordered.writes(), "different writes");
    assert_eq!(unordered.report, ordered.report, "different report");
    assert_eq!(
        ordered.report.pulled, 3,
        "one pull each of add, flags, drop"
    );
    assert_eq!(ordered.report.pushed, 1);
}

/// A handle listed twice pairs with its one placement, pulling no phantom.
#[test]
fn a_handle_listed_twice_is_merged_once() {
    let snapshot = full(vec![remote("1", &["seen"]), remote("1", &["seen"])]);
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let run = run_batches(&mut sync, vec![synced("1", &[])], snapshot);

    assert_eq!(
        run.report.events,
        [PimdirSyncEvent::FlagsChanged("1".into())]
    );
    assert_eq!(
        run.writes().len(),
        2,
        "one upsert and the checkpoint: {:?}",
        run.writes(),
    );
}

/// The merge hands a full write batch over rather than holding every write.
#[test]
fn a_full_write_batch_is_handed_over_mid_merge() {
    let extra = 76;
    let count = PimdirSync::WRITE_CHUNK + extra;
    let items = (0..count)
        .map(|index| remote(&format!("{index:05}"), &[]))
        .collect();

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let run = run_batches(&mut sync, vec![], full(items));

    assert_eq!(run.order, ["write", "write"], "one batch, or three");
    assert_eq!(run.batches[0].len(), PimdirSync::WRITE_CHUNK);
    assert_eq!(
        run.batches[1].len(),
        extra + 1,
        "the rest, plus the checkpoint",
    );
    assert!(
        run.batch_of(|op| matches!(op, PimdirWriteOp::SetCheckpoint { .. })) == Some(1),
        "a mid-merge batch must not checkpoint what it has not merged",
    );
    assert_eq!(run.report.pulled, count);
}

/// A batch boundary falls between candidates, never inside one.
///
/// A keep-both resolution writes the pulled placement and the staged
/// body together, and losing either would lose a version.
#[test]
fn a_batch_never_cuts_through_one_candidate() {
    let fillers = PimdirSync::WRITE_CHUNK - 1;
    let items = (0..fillers)
        .map(|index| remote(&format!("{index:05}"), &[]))
        .chain([remote_rev("zz", "r2")])
        .collect();

    let mut sync = PimdirSync::new("inbox", with_conflict(PimdirConflictPolicy::KeepBoth));
    let run = run_batches(&mut sync, vec![edited("zz")], full(items));

    let staged = run
        .batch_of(|op| matches!(op, PimdirWriteOp::UpsertPlacement(p) if p.status == PimdirStatus::Created))
        .expect("a keep-both duplicate");
    let pulled = run
        .batch_of(|op| matches!(op, PimdirWriteOp::UpsertPlacement(p) if p.handle.as_str() == "zz"))
        .expect("the pulled placement");

    assert!(
        run.batches[0].len() > PimdirSync::WRITE_CHUNK,
        "the boundary must fall on the resolution, not before it",
    );
    assert_eq!(
        staged, pulled,
        "both versions of one candidate must land together: {:?}",
        run.order,
    );
}

#[test]
fn remote_flag_change_pulls() {
    let local = synced("1", &[]);
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

    assert!(pushes.is_none());
    assert_eq!(report.pulled, 1);
    let PimdirWriteOp::UpsertPlacement(p) = &writes[0] else {
        panic!("expected UpsertPlacement, got {:?}", writes[0]);
    };
    assert!(p.flags.contains("seen"));
    assert_eq!(p.status, PimdirStatus::Clean);
}

/// Each side wins its own flag: the union is pushed, no conflict.
#[test]
fn divergent_flags_merge_element_wise() {
    let mut local = synced("1", &[]);
    local.flags = PimdirFlags::from_iter(["flagged"]);
    local.status = PimdirStatus::Dirty;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

    match &pushes.expect("a push")[0].kind {
        PimdirChangeKind::SetFlags { flags, .. } => {
            assert!(flags.contains("flagged") && flags.contains("seen"));
        }
        other => panic!("expected a SetFlags push, got {other:?}"),
    }
    assert_eq!(report.conflicts, 0);
    assert_eq!(report.pulled, 1, "the remote-won flag is folded in");

    let rebased = writes
        .iter()
        .rev()
        .find_map(|w| match w {
            PimdirWriteOp::UpsertPlacement(p) if p.handle.as_str() == "1" => Some(p),
            _ => None,
        })
        .expect("a rebased placement");
    assert_eq!(rebased.status, PimdirStatus::Clean);
    assert!(rebased.flags.contains("flagged") && rebased.flags.contains("seen"));
    let base = rebased.base.as_ref().expect("a base");
    assert!(base.flags.contains("flagged") && base.flags.contains("seen"));
}

/// The local removal and the remote addition both win.
#[test]
fn flag_removal_merges_against_concurrent_addition() {
    let mut local = synced("1", &["seen"]);
    local.flags = PimdirFlags::default();
    local.status = PimdirStatus::Dirty;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, _writes, report) = run(
        &mut sync,
        vec![local],
        vec![remote("1", &["seen", "important"])],
    );

    match &pushes.expect("a push")[0].kind {
        PimdirChangeKind::SetFlags { flags, .. } => {
            assert!(flags.contains("important"), "the remote addition wins");
            assert!(!flags.contains("seen"), "the local removal wins");
        }
        other => panic!("expected a SetFlags push, got {other:?}"),
    }
    assert_eq!(report.conflicts, 0);
}

#[test]
fn read_only_keeps_local_dirty() {
    let mut local = synced("1", &[]);
    local.flags = PimdirFlags::from_iter(["seen"]);
    local.status = PimdirStatus::Dirty;

    let opts = PimdirSyncOptions {
        push: false,
        rights: PimdirPushRights::all(),
        delete: PimdirDeletePolicy::Revert,
        conflict: PimdirConflictPolicy::Manual,
        full: false,
    };
    let mut sync = PimdirSync::new("inbox", opts);
    let (pushes, _writes, report) = run(&mut sync, vec![local], vec![remote("1", &[])]);

    assert!(pushes.is_none(), "read-only source must not push");
    assert_eq!(report.pushed, 0);
}

#[test]
fn delta_vanished_drops() {
    let local = synced("1", &["seen"]);
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let snapshot = delta(vec![], vec![PimdirHandle::from("1")]);
    let (pushes, writes, report) = run_snapshot(&mut sync, vec![local], snapshot);

    assert!(pushes.is_none());
    assert_eq!(report.pulled, 1);
    assert!(
        matches!(&writes[0], PimdirWriteOp::DropPlacement { handle, .. } if handle.as_str() == "1"),
        "vanished placement dropped, got {:?}",
        writes[0],
    );
}

#[test]
fn delta_pull_add() {
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let snapshot = delta(vec![remote("9", &["seen"])], vec![]);
    let (pushes, writes, report) = run_snapshot(&mut sync, vec![], snapshot);

    assert!(pushes.is_none());
    assert_eq!(report.pulled, 1);
    let PimdirWriteOp::UpsertPlacement(p) = &writes[0] else {
        panic!("expected UpsertPlacement, got {:?}", writes[0]);
    };
    assert_eq!(p.handle.as_str(), "9");
    assert_eq!(p.level, PimdirLevel::Probed);
}

#[test]
fn delta_leaves_unlisted_untouched() {
    let one = synced("1", &[]);
    let two = synced("2", &[]);
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let snapshot = delta(vec![remote("2", &["seen"])], vec![]);
    let (pushes, writes, report) = run_snapshot(&mut sync, vec![one, two], snapshot);

    assert!(pushes.is_none());
    assert_eq!(report.pulled, 1);
    assert_eq!(writes.len(), 2, "only the changed placement and checkpoint");
    let PimdirWriteOp::UpsertPlacement(p) = &writes[0] else {
        panic!("expected UpsertPlacement, got {:?}", writes[0]);
    };
    assert_eq!(p.handle.as_str(), "2");
    assert!(p.flags.contains("seen"));
}

/// An unlisted dirty handle derives its pending push against its own base.
#[test]
fn delta_pushes_unlisted_local_dirty() {
    let mut local = synced("1", &[]);
    local.flags = PimdirFlags::from_iter(["seen"]);
    local.status = PimdirStatus::Dirty;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let snapshot = delta(vec![], vec![]);
    let (pushes, _writes, report) = run_snapshot(&mut sync, vec![local], snapshot);

    let pushes = pushes.expect("a push");
    assert!(matches!(pushes[0].kind, PimdirChangeKind::SetFlags { .. }));
    assert_eq!(report.pushed, 1);
}

#[test]
fn unchanged_flags_is_noop() {
    let local = synced("1", &["seen"]);
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

    assert!(pushes.is_none());
    assert_eq!(report, PimdirSyncReport::default(), "a no-op sync");
    assert!(
        upserted(&writes, "1").is_none(),
        "an unchanged placement is not rewritten: {writes:?}",
    );
}

#[test]
fn concurrent_same_flags_rebases_without_push() {
    let mut local = synced("1", &[]);
    local.flags = PimdirFlags::from_iter(["flagged"]);
    local.status = PimdirStatus::Dirty;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, report) = run(&mut sync, vec![local], vec![remote("1", &["flagged"])]);

    assert!(pushes.is_none(), "no push when both reached the same flags");
    assert_eq!(report.conflicts, 0);
    let rebased = upserted(&writes, "1").expect("a converging rebase");
    assert_eq!(rebased.status, PimdirStatus::Clean);
    assert!(
        rebased
            .base
            .as_ref()
            .expect("a base")
            .flags
            .contains("flagged")
    );
}

#[test]
fn no_base_present_converges_on_remote() {
    let mut local = synced("1", &["flagged"]);
    local.base = None;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

    assert!(pushes.is_none());
    assert_eq!(report.pulled, 1);
    let pulled = upserted(&writes, "1").expect("a converged placement");
    assert_eq!(pulled.status, PimdirStatus::Clean);
    assert!(pulled.flags.contains("seen"));
    assert!(!pulled.flags.contains("flagged"), "remote flags win");
}

/// Pulled flags never launder a conflict away, or the staged edit is lost.
#[test]
fn flag_pull_on_a_conflicted_placement_keeps_the_conflict() {
    let mut placement = edited("1");
    placement.status = PimdirStatus::Conflict;
    placement.conflict_revision = Some("r2".into());

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let mut item = remote_rev("1", "r2");
    item.flags = PimdirFlags::from_iter(["seen"]);
    let (pushes, writes, report) = run(&mut sync, vec![placement], vec![item]);

    assert!(pushes.is_none());
    assert_eq!(report.pulled, 1);
    let pulled = upserted(&writes, "1").expect("a flag pull");
    assert_eq!(
        pulled.status,
        PimdirStatus::Conflict,
        "the conflict survives"
    );
    assert!(pulled.flags.contains("seen"));
    assert_eq!(
        pulled.object,
        Some(PimdirHash::from("h2")),
        "the edit survives"
    );
}

#[test]
fn read_only_still_pulls_remote_changes() {
    let local = synced("1", &[]);
    let opts = PimdirSyncOptions {
        push: false,
        rights: PimdirPushRights::all(),
        delete: PimdirDeletePolicy::Revert,
        conflict: PimdirConflictPolicy::Manual,
        full: false,
    };
    let mut sync = PimdirSync::new("inbox", opts);
    let (pushes, writes, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

    assert!(pushes.is_none());
    assert_eq!(report.pulled, 1);
    assert!(
        upserted(&writes, "1")
            .expect("a pull")
            .flags
            .contains("seen")
    );
}

#[test]
fn accepted_delete_drops_tombstone() {
    let mut local = synced("1", &["seen"]);
    local.status = PimdirStatus::Tombstone;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let results = vec![PimdirPushResult {
        handle: PimdirHandle::from("1"),
        outcome: PimdirPushOutcome::Accepted,
        assigned: None,
        revision: None,
    }];
    let (writes, report) = run_push(
        &mut sync,
        vec![local],
        vec![remote("1", &["seen"])],
        results,
    );

    assert_eq!(report.pushed, 1);
    assert!(
        writes.iter().any(
            |w| matches!(w, PimdirWriteOp::DropPlacement { handle, .. } if handle.as_str() == "1")
        ),
        "an accepted delete drops the tombstone: {writes:?}",
    );
}

#[test]
fn rejected_delete_keeps_tombstone() {
    let mut local = synced("1", &["seen"]);
    local.status = PimdirStatus::Tombstone;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let results = vec![PimdirPushResult {
        handle: PimdirHandle::from("1"),
        outcome: PimdirPushOutcome::Rejected,
        assigned: None,
        revision: None,
    }];
    let (writes, report) = run_push(
        &mut sync,
        vec![local],
        vec![remote("1", &["seen"])],
        results,
    );

    assert_eq!(report.rejected, 1);
    assert!(
        !writes.iter().any(
            |w| matches!(w, PimdirWriteOp::DropPlacement { handle, .. } if handle.as_str() == "1")
        ),
        "a rejected delete must not drop the tombstone: {writes:?}",
    );
}

#[test]
fn local_delete_gone_remote_just_drops() {
    let mut local = synced("1", &["seen"]);
    local.status = PimdirStatus::Tombstone;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, report) = run(&mut sync, vec![local], vec![]);

    assert!(pushes.is_none());
    assert_eq!(report.pushed, 0);
    assert!(
        writes.iter().any(
            |w| matches!(w, PimdirWriteOp::DropPlacement { handle, .. } if handle.as_str() == "1")
        ),
        "the tombstone is dropped without a push: {writes:?}",
    );
}

#[test]
fn remote_delete_in_full_drops() {
    let local = synced("1", &["seen"]);
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, report) = run(&mut sync, vec![local], vec![]);

    assert!(pushes.is_none());
    assert_eq!(report.pulled, 1);
    assert!(
        writes.iter().any(
            |w| matches!(w, PimdirWriteOp::DropPlacement { handle, .. } if handle.as_str() == "1")
        ),
        "the vanished placement is dropped: {writes:?}",
    );
}

/// A create-collision conflict whose remote side went keeps its body.
#[test]
fn a_base_less_body_absent_from_the_remote_resurrects_as_a_create() {
    let mut placement = synced("1", &[]);
    placement.base = None;
    placement.object = Some(PimdirHash::from("h1"));
    placement.level = PimdirLevel::Full;
    placement.status = PimdirStatus::Conflict;
    placement.conflict_revision = Some("r9".into());

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, _report) = run(&mut sync, vec![placement], vec![]);

    assert!(matches!(
        &pushes.expect("a push")[0].kind,
        PimdirChangeKind::Add { origin: None, .. }
    ));
    let resurrected = upserted(&writes, "1").expect("a resurrected placement");
    assert_eq!(resurrected.status, PimdirStatus::Created);
    assert_eq!(resurrected.conflict_revision, None);
    assert_eq!(resurrected.object, Some(PimdirHash::from("h1")));
}

/// A probe the enumeration no longer lists is gone like any member.
#[test]
fn a_probe_absent_from_a_complete_enumeration_is_dropped() {
    let mut probe = synced("1", &["flagged"]);
    probe.link_id = None;
    probe.base = None;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, report) = run(&mut sync, vec![probe], vec![]);

    assert!(pushes.is_none());
    assert_eq!(report.pulled, 1);
    assert_eq!(report.events, [PimdirSyncEvent::Vanished("1".into())]);
    assert!(
        writes.iter().any(
            |w| matches!(w, PimdirWriteOp::DropPlacement { handle, reason, .. } if handle.as_str() == "1" && *reason == PimdirDropReason::Deleted)
        ),
        "the probe is dropped: {writes:?}",
    );
}

/// The Add carries the origin (copy, not re-upload) and the flag set.
#[test]
fn created_placement_pushes_add() {
    let mut local = created("tmp-1");
    local.flags = PimdirFlags::from_iter(["seen"]);
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, _writes, report) = run(&mut sync, vec![local], vec![]);

    match &pushes.expect("a push")[0].kind {
        PimdirChangeKind::Add { origin, flags, .. } => {
            assert!(origin.is_some());
            assert!(flags.contains("seen"), "the flag set rides the add");
        }
        other => panic!("expected an Add push, got {other:?}"),
    }
    assert_eq!(report.pushed, 1);
}

/// The placeholder is dropped and the placement rekeyed clean and based.
#[test]
fn accepted_create_rekeys_to_assigned() {
    let mut local = created("tmp-1");
    local.object = Some(PimdirHash::from("h-1"));
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let results = vec![PimdirPushResult {
        handle: PimdirHandle::from("tmp-1"),
        outcome: PimdirPushOutcome::Accepted,
        assigned: Some(PimdirHandle::from("42")),
        revision: Some("r1".into()),
    }];
    let (writes, _report) = run_push(&mut sync, vec![local], vec![], results);

    assert!(
        writes.iter().any(
            |w| matches!(w, PimdirWriteOp::DropPlacement { handle, .. } if handle.as_str() == "tmp-1")
        ),
        "the placeholder is dropped: {writes:?}",
    );
    let real = upserted(&writes, "42").expect("the placement is rekeyed to the assigned handle");
    assert_eq!(real.status, PimdirStatus::Clean);
    assert!(real.origin.is_none());
    let base = real.base.as_ref().expect("a base");
    assert_eq!(base.revision.as_deref(), Some("r1"));
    assert_eq!(base.object, Some(PimdirHash::from("h-1")));
}

#[test]
fn rejected_create_keeps_placeholder() {
    let local = created("tmp-1");
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let results = vec![PimdirPushResult {
        handle: PimdirHandle::from("tmp-1"),
        outcome: PimdirPushOutcome::Rejected,
        assigned: None,
        revision: None,
    }];
    let (writes, report) = run_push(&mut sync, vec![local], vec![], results);

    assert_eq!(report.rejected, 1);
    assert!(
        !writes
            .iter()
            .any(|w| matches!(w, PimdirWriteOp::DropPlacement { .. })),
        "a rejected create must not drop the placeholder: {writes:?}",
    );
    assert!(upserted(&writes, "tmp-1").is_none());
}

/// A tombstone carrying an origin is a move: the Remove names the target.
#[test]
fn move_pushes_remove_with_target() {
    let mut local = synced("1", &["seen"]);
    local.status = PimdirStatus::Tombstone;
    local.origin = Some(PimdirOrigin {
        collection: "archive".into(),
        handle: PimdirHandle::from("1"),
    });

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, _writes, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

    match &pushes.expect("a push")[0].kind {
        PimdirChangeKind::Remove { to: Some(to), .. } => assert_eq!(to.as_str(), "archive"),
        other => panic!("expected a move Remove, got {other:?}"),
    }
    assert_eq!(report.pushed, 1);
}

/// Without an assigned handle (no UIDPLUS) the next enumerate re-adds it.
#[test]
fn accepted_create_without_assigned_drops_placeholder() {
    let local = created("tmp-1");
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let results = vec![PimdirPushResult {
        handle: PimdirHandle::from("tmp-1"),
        outcome: PimdirPushOutcome::Accepted,
        assigned: None,
        revision: None,
    }];
    let (writes, _report) = run_push(&mut sync, vec![local], vec![], results);

    assert!(
        writes.iter().any(
            |w| matches!(w, PimdirWriteOp::DropPlacement { handle, .. } if handle.as_str() == "tmp-1")
        ),
        "the placeholder is dropped once the copy lands: {writes:?}",
    );
    assert!(upserted(&writes, "tmp-1").is_none());
}

#[test]
fn full_sync_ignores_checkpoint() {
    let mut sync = PimdirSync::new(
        "inbox",
        PimdirSyncOptions {
            push: true,
            rights: PimdirPushRights::all(),
            delete: PimdirDeletePolicy::Revert,
            conflict: PimdirConflictPolicy::Manual,
            full: true,
        },
    );
    let _ = sync.resume(None);
    let loaded = PimdirLoaded {
        placements: Vec::new(),
        checkpoint: Some(PimdirCheckpoint(b"cp".to_vec())),
    };
    match sync.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsEnumerate { cursor, .. }) => {
            assert!(cursor.is_none(), "a full sync must ignore the checkpoint");
        }
        state => panic!("expected WantsEnumerate, got {state:?}"),
    }
}

/// A synced placement with a staged edit: body "h2", base "h1" at "r1".
fn edited(handle: &str) -> PimdirPlacement {
    let mut placement = synced(handle, &[]);
    placement.status = PimdirStatus::Dirty;
    placement.object = Some(PimdirHash::from("h2"));
    placement.level = PimdirLevel::Full;
    let base = placement.base.as_mut().expect("a base");
    base.revision = Some("r1".into());
    base.object = Some(PimdirHash::from("h1"));
    placement
}

/// A remote item at the given content revision.
fn remote_rev(handle: &str, revision: &str) -> PimdirRemoteItem {
    let mut item = remote(handle, &[]);
    item.revision = Some(revision.into());
    item
}

/// A delta lists a flag change once, so the content axis must not eat it.
///
/// The accepted push rebases the row the flag merge wrote, never the one
/// read before it (SYNC §5): the last write of the handle holds the flag.
#[test]
fn a_content_push_still_pulls_a_remote_flag_change() {
    let mut local = edited("1");
    let mut item = remote_rev("1", "r1");
    item.flags = PimdirFlags::from_iter(["seen"]);
    local.base.as_mut().expect("a base").revision = Some("r1".into());

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, _report) = run(&mut sync, vec![local], vec![item]);

    match &pushes.expect("a push")[0].kind {
        PimdirChangeKind::Update { .. } => {}
        other => panic!("expected an Update push, got {other:?}"),
    }
    let rebased = upserted(&writes, "1").expect("the rebased placement");
    assert!(
        rebased.flags.contains("seen"),
        "the remote flag lands with the content push, not a run later",
    );
    assert_eq!(rebased.status, PimdirStatus::Clean);
    let base = rebased.base.as_ref().expect("a base");
    assert!(base.flags.contains("seen"), "the pulled flag is agreed on");
    assert_eq!(base.object, Some(PimdirHash::from("h2")));
}

/// The Update is gated on the base revision, which then adopts the result.
#[test]
fn local_content_edit_pushes_update_and_rebases() {
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let _ = sync.resume(None);
    let _ = sync.resume(Some(PimdirArg::Load(PimdirLoaded {
        placements: vec![edited("1")],
        checkpoint: None,
    })));

    let pushes = match sync.resume(Some(PimdirArg::Enumerate(full(vec![remote_rev(
        "1", "r1",
    )])))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsPush { changes, .. }) => changes,
        state => panic!("expected WantsPush, got {state:?}"),
    };
    match &pushes[0].kind {
        PimdirChangeKind::Update {
            handle,
            object,
            if_match,
        } => {
            assert_eq!(handle.as_str(), "1");
            assert_eq!(object, &PimdirHash::from("h2"));
            assert_eq!(if_match.as_deref(), Some("r1"));
        }
        other => panic!("expected an Update push, got {other:?}"),
    }

    let results = vec![PimdirPushResult {
        handle: PimdirHandle::from("1"),
        outcome: PimdirPushOutcome::Accepted,
        assigned: None,
        revision: Some("r2".into()),
    }];
    let writes = match sync.resume(Some(PimdirArg::Push(results))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(writes)) => writes,
        state => panic!("expected WantsWrite, got {state:?}"),
    };

    let rebased = upserted(&writes, "1").expect("a rebased placement");
    assert_eq!(rebased.status, PimdirStatus::Clean);
    let base = rebased.base.as_ref().expect("a base");
    assert_eq!(base.revision.as_deref(), Some("r2"));
    assert_eq!(base.object, Some(PimdirHash::from("h2")));
}

/// The content rebase keeps the base flags, so the flag push derives later.
#[test]
fn content_rebase_defers_a_riding_flag_edit() {
    let mut placement = edited("1");
    placement.flags = PimdirFlags::from_iter(["seen"]);

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let results = vec![PimdirPushResult {
        handle: PimdirHandle::from("1"),
        outcome: PimdirPushOutcome::Accepted,
        assigned: None,
        revision: Some("r2".into()),
    }];
    let (writes, _report) = run_push(
        &mut sync,
        vec![placement],
        vec![remote_rev("1", "r1")],
        results,
    );

    let rebased = upserted(&writes, "1").expect("a rebased placement");
    assert_eq!(
        rebased.status,
        PimdirStatus::Dirty,
        "the flag edit stays pending"
    );
    let base = rebased.base.as_ref().expect("a base");
    assert!(!base.flags.contains("seen"), "base flags stay as synced");
    assert_eq!(base.object, Some(PimdirHash::from("h2")));
}

#[test]
fn remote_content_change_refreshes_the_stale_body() {
    let mut placement = synced("1", &[]);
    placement.object = Some(PimdirHash::from("h1"));
    placement.level = PimdirLevel::Full;
    let base = placement.base.as_mut().expect("a base");
    base.revision = Some("r1".into());
    base.object = Some(PimdirHash::from("h1"));

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, report) = run(&mut sync, vec![placement], vec![remote_rev("1", "r2")]);

    assert!(pushes.is_none(), "a refresh pushes nothing");
    assert_eq!(report.refreshed, 1);
    let refreshed = upserted(&writes, "1").expect("a refreshed placement");
    assert_eq!(refreshed.object, None, "the stale body is dropped");
    assert_eq!(
        refreshed.level,
        PimdirLevel::Probed,
        "the summary is stale too"
    );
    let base = refreshed.base.as_ref().expect("a base");
    assert_eq!(base.revision.as_deref(), Some("r2"));
    assert_eq!(base.object, None);
}

/// The mark carries the observed revision; the upgrade fetches the body.
#[test]
fn divergent_content_edits_conflict() {
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, report) = run(&mut sync, vec![edited("1")], vec![remote_rev("1", "r2")]);

    assert!(pushes.is_none());
    assert_eq!(report.conflicts, 1);
    assert_eq!(report.refreshed, 0);
    let conflicted = upserted(&writes, "1").expect("a conflicted placement");
    assert_eq!(conflicted.status, PimdirStatus::Conflict);
    assert_eq!(conflicted.conflict_revision.as_deref(), Some("r2"));
    assert_eq!(
        conflicted.conflict_object, None,
        "the diverging body is wanted, not taken"
    );
    assert_eq!(
        conflicted.object,
        Some(PimdirHash::from("h2")),
        "the edit survives"
    );
}

/// With no shared ancestor there is nothing to merge, so it conflicts.
///
/// Converging on flags alone would strand the two bodies apart and
/// loop every sync; the consumer's resolution re-establishes a base.
#[test]
fn base_less_body_present_on_both_conflicts() {
    let mut placement = synced("1", &[]);
    placement.base = None;
    placement.object = Some(PimdirHash::from("h1"));
    placement.level = PimdirLevel::Full;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, report) = run(&mut sync, vec![placement], vec![remote_rev("1", "r9")]);

    assert!(pushes.is_none(), "an unresolved conflict pushes nothing");
    assert_eq!(report.conflicts, 1);
    let conflicted = upserted(&writes, "1").expect("a conflicted placement");
    assert_eq!(conflicted.status, PimdirStatus::Conflict);
    assert_eq!(conflicted.conflict_revision.as_deref(), Some("r9"));
    assert_eq!(
        conflicted.object,
        Some(PimdirHash::from("h1")),
        "the body survives for the resolution"
    );
}

/// No body and no base is a probe: agreeing on the flags, it is left alone.
#[test]
fn base_less_body_less_present_on_both_stays_flag_only() {
    let mut placement = synced("1", &["seen"]);
    placement.base = None;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, report) = run(&mut sync, vec![placement], vec![remote("1", &["seen"])]);

    assert!(pushes.is_none());
    assert_eq!(report, PimdirSyncReport::default(), "no conflict, no pull");
    assert!(
        upserted(&writes, "1").is_none(),
        "nothing to write: {writes:?}"
    );
}

#[test]
fn an_unresolved_conflict_tracks_the_latest_remote_revision() {
    let mut placement = edited("1");
    placement.status = PimdirStatus::Conflict;
    placement.conflict_revision = Some("r2".into());

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, report) = run(&mut sync, vec![placement], vec![remote_rev("1", "r3")]);

    assert!(pushes.is_none());
    assert_eq!(report.conflicts, 0, "no recount");
    let tracked = upserted(&writes, "1").expect("an updated placement");
    assert_eq!(tracked.status, PimdirStatus::Conflict);
    assert_eq!(tracked.conflict_revision.as_deref(), Some("r3"));
    assert_eq!(
        tracked.object,
        Some(PimdirHash::from("h2")),
        "the edit survives"
    );
}

/// The stored body described the old revision, a resolver must not see it.
#[test]
fn a_conflict_whose_remote_moved_drops_its_stored_body() {
    let mut placement = edited("1");
    placement.status = PimdirStatus::Conflict;
    placement.conflict_revision = Some("r2".into());
    placement.conflict_object = Some(PimdirHash::from("h-r2"));

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (_pushes, writes, _report) = run(&mut sync, vec![placement], vec![remote_rev("1", "r3")]);

    let tracked = upserted(&writes, "1").expect("an updated placement");
    assert_eq!(tracked.conflict_revision.as_deref(), Some("r3"));
    assert_eq!(
        tracked.conflict_object, None,
        "the body of the revision that moved is asked for anew"
    );
}

/// The kept ancestor pushes, gated on the revision it was decided against.
#[test]
fn a_resolution_keeping_the_ancestor_pushes_it() {
    let mut placement = edited("1");
    placement.object = Some(PimdirHash::from("h-base"));
    let base = placement.base.as_mut().expect("a base");
    base.revision = Some("r2".into());
    base.object = Some(PimdirHash::from("h-remote"));

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, _writes, report) = run(&mut sync, vec![placement], vec![remote_rev("1", "r2")]);

    assert_eq!(report.conflicts, 0);
    match &pushes.expect("a push")[0].kind {
        PimdirChangeKind::Update {
            object, if_match, ..
        } => {
            assert_eq!(object, &PimdirHash::from("h-base"));
            assert_eq!(if_match.as_deref(), Some("r2"));
        }
        other => panic!("expected an Update push, got {other:?}"),
    }
}

#[test]
fn a_resolution_taking_the_remote_body_settles_clean() {
    let mut placement = edited("1");
    placement.object = Some(PimdirHash::from("h-remote"));
    let base = placement.base.as_mut().expect("a base");
    base.revision = Some("r2".into());
    base.object = Some(PimdirHash::from("h-remote"));

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, report) = run(&mut sync, vec![placement], vec![remote_rev("1", "r2")]);

    assert!(pushes.is_none(), "the remote holds the decision already");
    assert_eq!(report.conflicts, 0);
    let settled = upserted(&writes, "1").expect("a settled placement");
    assert_eq!(settled.status, PimdirStatus::Clean);
    assert_eq!(settled.object, Some(PimdirHash::from("h-remote")));
}

/// No revision means no content signal, so neither side is ever written.
#[test]
fn an_immutable_backend_records_no_conflict_at_all() {
    let mut placement = edited("1");
    let base = placement.base.as_mut().expect("a base");
    base.revision = None;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (_pushes, writes, report) = run(&mut sync, vec![placement], vec![remote("1", &[])]);

    assert_eq!(report.conflicts, 0);
    for write in &writes {
        let PimdirWriteOp::UpsertPlacement(placement) = write else {
            continue;
        };
        assert_ne!(placement.status, PimdirStatus::Conflict);
        assert_eq!(placement.conflict_revision, None);
        assert_eq!(placement.conflict_object, None);
    }
}

/// A delta lists a flag change once, so the conflict mark must not eat it.
#[test]
fn a_content_conflict_still_pulls_the_remote_flag_change() {
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let mut item = remote_rev("1", "r2");
    item.flags = PimdirFlags::from_iter(["seen"]);
    let (pushes, writes, report) = run(&mut sync, vec![edited("1")], vec![item]);

    assert!(pushes.is_none());
    assert_eq!(report.conflicts, 1);
    let conflicted = writes
        .iter()
        .rev()
        .find_map(|w| match w {
            PimdirWriteOp::UpsertPlacement(p) if p.handle.as_str() == "1" => Some(p),
            _ => None,
        })
        .expect("a conflict write");
    assert_eq!(conflicted.status, PimdirStatus::Conflict);
    assert_eq!(conflicted.conflict_revision.as_deref(), Some("r2"));
    assert!(conflicted.flags.contains("seen"), "the flag change lands");
    assert_eq!(
        conflicted.object,
        Some(PimdirHash::from("h2")),
        "the edit survives"
    );
}

/// The synthesized remote state carries the observed conflict revision.
#[test]
fn unlisted_conflict_keeps_its_observed_remote_revision() {
    let mut placement = edited("1");
    placement.status = PimdirStatus::Conflict;
    placement.conflict_revision = Some("r2".into());

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let snapshot = delta(vec![], vec![]);
    let (pushes, writes, report) = run_snapshot(&mut sync, vec![placement], snapshot);

    assert!(pushes.is_none());
    assert_eq!(report.conflicts, 0, "no recount");
    assert!(
        upserted(&writes, "1").is_none(),
        "the conflict tracking must not regress to the base revision: {writes:?}",
    );
}

#[test]
fn remote_content_change_beats_a_local_delete() {
    let mut placement = edited("1");
    placement.status = PimdirStatus::Tombstone;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, report) = run(&mut sync, vec![placement], vec![remote_rev("1", "r2")]);

    assert!(pushes.is_none(), "the delete is not pushed");
    assert_eq!(report.pulled, 1);
    let resurrected = upserted(&writes, "1").expect("a resurrected placement");
    assert_eq!(resurrected.status, PimdirStatus::Clean);
    let base = resurrected.base.as_ref().expect("a base");
    assert_eq!(base.revision.as_deref(), Some("r2"));
}

/// The edit rides into the target's create, where an Update would race it.
#[test]
fn a_tombstone_carrying_a_staged_edit_still_removes() {
    let mut placement = edited("1");
    placement.status = PimdirStatus::Tombstone;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, _writes, _report) = run(&mut sync, vec![placement], vec![remote_rev("1", "r1")]);

    match &pushes.expect("a push")[0].kind {
        PimdirChangeKind::Remove { to, if_match, .. } => {
            assert_eq!(*to, None, "a plain remove, not a server-side move");
            assert_eq!(if_match.as_deref(), Some("r1"));
        }
        other => panic!("expected a Remove, got {other:?}"),
    }
}

/// A storage plumbing a tombstone origin through still derives the target.
#[test]
fn a_tombstone_origin_derives_a_move_remove() {
    let mut placement = synced("1", &[]);
    placement.status = PimdirStatus::Tombstone;
    placement.origin = Some(PimdirOrigin {
        collection: "archive".into(),
        handle: PimdirHandle::from("1"),
    });

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, _writes, _report) = run(&mut sync, vec![placement], vec![remote("1", &[])]);

    match &pushes.expect("a push")[0].kind {
        PimdirChangeKind::Remove { to: Some(to), .. } => assert_eq!(to.as_str(), "archive"),
        other => panic!("expected a move Remove, got {other:?}"),
    }
}

#[test]
fn remove_carries_the_base_revision_as_precondition() {
    let mut placement = edited("1");
    placement.status = PimdirStatus::Tombstone;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, _writes, _report) = run(&mut sync, vec![placement], vec![remote_rev("1", "r1")]);

    match &pushes.expect("a push")[0].kind {
        PimdirChangeKind::Remove { if_match, .. } => {
            assert_eq!(if_match.as_deref(), Some("r1"))
        }
        other => panic!("expected a Remove push, got {other:?}"),
    }
}

#[test]
fn remote_delete_with_staged_edit_resurrects_as_create() {
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, _report) = run(&mut sync, vec![edited("1")], vec![]);

    match &pushes.expect("a push")[0].kind {
        PimdirChangeKind::Add { object, origin, .. } => {
            assert_eq!(object, &Some(PimdirHash::from("h2")), "the edited body");
            assert!(origin.is_none(), "an append, not a copy");
        }
        other => panic!("expected an Add push, got {other:?}"),
    }
    let resurrected = upserted(&writes, "1").expect("a resurrected placement");
    assert_eq!(resurrected.status, PimdirStatus::Created);
    assert!(resurrected.base.is_none());
    assert_eq!(resurrected.object, Some(PimdirHash::from("h2")));
}

/// The remote side is gone, so the conflict is moot and the edit survives.
#[test]
fn remote_delete_of_a_conflicted_placement_resurrects_the_edit() {
    let mut placement = edited("1");
    placement.status = PimdirStatus::Conflict;
    placement.conflict_revision = Some("r2".into());

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, _report) = run(&mut sync, vec![placement], vec![]);

    assert!(matches!(
        &pushes.expect("a push")[0].kind,
        PimdirChangeKind::Add { origin: None, .. }
    ));
    let resurrected = upserted(&writes, "1").expect("a resurrected placement");
    assert_eq!(resurrected.status, PimdirStatus::Created);
    assert_eq!(resurrected.conflict_revision, None, "the conflict is moot");
    assert_eq!(
        resurrected.object,
        Some(PimdirHash::from("h2")),
        "the edit survives"
    );
}

/// No push on a read-only source, but the pending create keeps the edit.
#[test]
fn read_only_remote_delete_with_staged_edit_keeps_the_edit() {
    let opts = PimdirSyncOptions {
        push: false,
        rights: PimdirPushRights::all(),
        delete: PimdirDeletePolicy::Revert,
        conflict: PimdirConflictPolicy::Manual,
        full: false,
    };
    let mut sync = PimdirSync::new("inbox", opts);
    let (pushes, writes, _report) = run(&mut sync, vec![edited("1")], vec![]);

    assert!(pushes.is_none());
    let resurrected = upserted(&writes, "1").expect("a resurrected placement");
    assert_eq!(resurrected.status, PimdirStatus::Created);
    assert_eq!(resurrected.object, Some(PimdirHash::from("h2")));
}

#[test]
fn read_only_keeps_a_content_edit_dirty() {
    let mut sync = PimdirSync::new(
        "inbox",
        PimdirSyncOptions {
            push: false,
            rights: PimdirPushRights::all(),
            delete: PimdirDeletePolicy::Revert,
            conflict: PimdirConflictPolicy::Manual,
            full: false,
        },
    );
    let (pushes, writes, _report) = run(&mut sync, vec![edited("1")], vec![remote_rev("1", "r1")]);

    assert!(pushes.is_none());
    assert!(
        upserted(&writes, "1").is_none(),
        "the placement is left as is"
    );
}

/// The member stays with its cached body rather than being refetched later.
#[test]
fn read_only_delete_is_reverted_rather_than_applied() {
    let mut local = synced("1", &["seen"]);
    local.status = PimdirStatus::Tombstone;

    let opts = PimdirSyncOptions {
        push: false,
        rights: PimdirPushRights::all(),
        delete: PimdirDeletePolicy::Revert,
        conflict: PimdirConflictPolicy::Manual,
        full: false,
    };
    let mut sync = PimdirSync::new("inbox", opts);
    let (pushes, writes, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

    assert!(pushes.is_none(), "read-only source must not push");
    assert_eq!(report.pushed, 0);
    assert!(
        !writes
            .iter()
            .any(|w| matches!(w, PimdirWriteOp::DropPlacement { .. })),
        "the member is not dropped: {writes:?}",
    );
    let reverted = upserted(&writes, "1").expect("the reverted placement");
    assert_eq!(reverted.status, PimdirStatus::Clean);
}

/// A delta never re-lists an untouched member, so the revert cannot wait.
#[test]
fn a_read_only_delete_survives_a_delta_enumerate() {
    let mut local = synced("1", &["seen"]);
    local.status = PimdirStatus::Tombstone;

    let opts = PimdirSyncOptions {
        push: false,
        ..Default::default()
    };
    let mut sync = PimdirSync::new("inbox", opts);
    let (_pushes, writes, _report) = run_snapshot(&mut sync, vec![local], delta(vec![], vec![]));

    let reverted = upserted(&writes, "1").expect("the reverted placement");
    assert_eq!(reverted.status, PimdirStatus::Clean);
}

/// A delta may report a handle removed before the replica ever knew it.
#[test]
fn unknown_vanished_handle_is_ignored() {
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let snapshot = delta(vec![], vec![PimdirHandle::from("ghost")]);
    let (pushes, writes, report) = run_snapshot(&mut sync, vec![], snapshot);

    assert!(pushes.is_none());
    assert_eq!(report, PimdirSyncReport::default());
    assert_eq!(writes.len(), 1, "only the checkpoint write");
    assert!(matches!(&writes[0], PimdirWriteOp::SetCheckpoint { .. }));
}

#[test]
fn noop_flag_edit_rebases_clean() {
    let mut local = synced("1", &["seen"]);
    local.status = PimdirStatus::Dirty;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

    assert!(pushes.is_none(), "nothing to push");
    assert_eq!(report, PimdirSyncReport::default());
    let cleaned = upserted(&writes, "1").expect("a cleaning rebase");
    assert_eq!(cleaned.status, PimdirStatus::Clean);
}

#[test]
fn missing_arg_errors() {
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let _ = sync.resume(None);
    match sync.resume(None) {
        PimdirCoroutineState::Complete(Err(PimdirArgError::MissingArg)) => {}
        state => panic!("expected MissingArg, got {state:?}"),
    }
}

/// An empty report reads like a run that did nothing, so resuming errors.
#[test]
fn a_completed_sync_does_not_resume() {
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let _ = run(&mut sync, vec![], vec![]);
    match sync.resume(Some(PimdirArg::Write)) {
        PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => {}
        state => panic!("expected UnexpectedArg, got {state:?}"),
    }
    match sync.resume(None) {
        PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => {}
        state => panic!("expected UnexpectedArg, got {state:?}"),
    }
}

/// A probe the enumeration lists again as the store holds it is no pull.
#[test]
fn a_relisted_probe_with_the_same_flags_derives_nothing() {
    let mut probe = synced("1", &["seen"]);
    probe.link_id = None;
    probe.base = None;

    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (pushes, writes, report) = run(&mut sync, vec![probe], vec![remote("1", &["seen"])]);

    assert!(pushes.is_none());
    assert_eq!(report, PimdirSyncReport::default(), "nothing pulled");
    assert!(
        upserted(&writes, "1").is_none(),
        "the probe is not rewritten: {writes:?}",
    );
}

/// The engine reads the default policy as a revert; a consumer that knows
/// the binding count resolves it before handing the options over.
#[test]
fn the_delete_policy_defaults_to_auto_read_as_revert() {
    assert_eq!(
        PimdirSyncOptions::default().delete,
        PimdirDeletePolicy::Auto
    );

    let mut local = synced("1", &["seen"]);
    local.status = PimdirStatus::Tombstone;
    let mut sync = PimdirSync::new("inbox", with_rights(true, true, true, false));
    let (_pushes, writes, _report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

    assert_eq!(
        upserted(&writes, "1")
            .expect("the reverted tombstone")
            .status,
        PimdirStatus::Clean,
    );
}

#[test]
fn unexpected_arg_errors() {
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let _ = sync.resume(None);
    match sync.resume(Some(PimdirArg::Write)) {
        PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => {}
        state => panic!("expected UnexpectedArg, got {state:?}"),
    }
}

/// A writable sync (`push = true`) with the given per-kind rights.
fn with_rights(flags: bool, content: bool, add: bool, remove: bool) -> PimdirSyncOptions {
    PimdirSyncOptions {
        rights: PimdirPushRights {
            flags,
            content,
            add,
            remove,
        },
        ..Default::default()
    }
}

#[test]
fn forbidding_flags_keeps_dirty_without_push() {
    let mut local = synced("1", &[]);
    local.flags = PimdirFlags::from_iter(["seen"]);
    local.status = PimdirStatus::Dirty;

    let mut sync = PimdirSync::new("inbox", with_rights(false, true, true, true));
    let (pushes, _writes, report) = run(&mut sync, vec![local], vec![remote("1", &[])]);

    assert!(pushes.is_none(), "a forbidden flag push must not fire");
    assert_eq!(report.pushed, 0);
}

#[test]
fn forbidding_remove_reverts_the_tombstone_by_default() {
    let mut local = synced("1", &["seen"]);
    local.status = PimdirStatus::Tombstone;

    let mut sync = PimdirSync::new("inbox", with_rights(true, true, true, false));
    let (pushes, writes, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

    assert!(pushes.is_none(), "a forbidden remove must not push");
    assert!(
        !writes.iter().any(|w| matches!(
            w,
            PimdirWriteOp::DropPlacement { handle, .. } if handle.as_str() == "1"
        )),
        "a delete the source refuses is never applied to the replica: {writes:?}",
    );
    assert_eq!(
        upserted(&writes, "1")
            .expect("the tombstone is reverted")
            .status,
        PimdirStatus::Clean,
        "the default policy mirrors the source, as it does for a read-only one",
    );
    assert_eq!(report.pushed, 0);
}

/// Rights `none()` and `push = false` agree on what a refused delete does.
#[test]
fn keeping_a_refused_delete_holds_the_tombstone_either_way() {
    let mut local = synced("1", &["seen"]);
    local.status = PimdirStatus::Tombstone;

    let forbidden = PimdirSyncOptions {
        rights: PimdirPushRights {
            remove: false,
            ..PimdirPushRights::all()
        },
        delete: PimdirDeletePolicy::Keep,
        ..Default::default()
    };
    let read_only = PimdirSyncOptions {
        push: false,
        delete: PimdirDeletePolicy::Keep,
        ..Default::default()
    };

    for opts in [forbidden, read_only] {
        let mut sync = PimdirSync::new("inbox", opts);
        let (pushes, writes, _report) =
            run(&mut sync, vec![local.clone()], vec![remote("1", &["seen"])]);

        assert!(pushes.is_none(), "a refused delete must not push");
        assert!(
            upserted(&writes, "1").is_none(),
            "the tombstone is held as it is, for a later run: {writes:?}",
        );
    }
}

#[test]
fn flags_allowed_remove_forbidden_pushes_only_flags() {
    let mut dirty = synced("1", &[]);
    dirty.flags = PimdirFlags::from_iter(["seen"]);
    dirty.status = PimdirStatus::Dirty;
    let mut tomb = synced("2", &[]);
    tomb.status = PimdirStatus::Tombstone;

    let mut sync = PimdirSync::new("inbox", with_rights(true, true, true, false));
    let (pushes, _writes, _report) = run(
        &mut sync,
        vec![dirty, tomb],
        vec![remote("1", &[]), remote("2", &[])],
    );

    let pushes = pushes.expect("the permitted flag push still fires");
    assert!(
        pushes
            .iter()
            .all(|c| matches!(c.kind, PimdirChangeKind::SetFlags { .. })),
        "only the flag change may be pushed, not the delete: {pushes:?}",
    );
}

#[test]
fn forbidding_add_keeps_created_pending() {
    let mut sync = PimdirSync::new("inbox", with_rights(true, true, false, true));
    let (pushes, _writes, report) = run(&mut sync, vec![created("tmp")], vec![]);

    assert!(pushes.is_none(), "a forbidden add must not push the create");
    assert_eq!(report.pushed, 0);
}

#[test]
fn event_added_on_remote_add() {
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (_p, _w, report) = run(&mut sync, vec![], vec![remote("1", &["seen"])]);
    assert_eq!(
        report.events,
        vec![PimdirSyncEvent::Added(PimdirHandle::from("1"))]
    );
}

#[test]
fn event_flags_changed_on_remote_flag_pull() {
    let local = synced("1", &[]);
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (_p, _w, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);
    assert_eq!(
        report.events,
        vec![PimdirSyncEvent::FlagsChanged(PimdirHandle::from("1"))]
    );
}

#[test]
fn event_vanished_on_delta_vanish() {
    let local = synced("1", &["seen"]);
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let snapshot = delta(vec![], vec![PimdirHandle::from("1")]);
    let (_p, _w, report) = run_snapshot(&mut sync, vec![local], snapshot);
    assert_eq!(
        report.events,
        vec![PimdirSyncEvent::Vanished(PimdirHandle::from("1"))]
    );
}

/// The consumer made the change, so nothing is reported back to it.
#[test]
fn an_accepted_flag_push_reports_no_event() {
    let mut local = synced("1", &[]);
    local.flags = PimdirFlags::from_iter(["seen"]);
    local.status = PimdirStatus::Dirty;
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let (_p, _w, report) = run(&mut sync, vec![local], vec![remote("1", &[])]);
    assert!(report.events.is_empty(), "{:?}", report.events);
    assert_eq!(report.pushed, 1);
}

#[test]
fn event_created_on_accepted_create() {
    let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
    let results = vec![PimdirPushResult {
        handle: PimdirHandle::from("tmp"),
        outcome: PimdirPushOutcome::Accepted,
        assigned: Some(PimdirHandle::from("99")),
        revision: None,
    }];
    let (_w, report) = run_push(&mut sync, vec![created("tmp")], vec![], results);
    assert_eq!(
        report.events,
        vec![PimdirSyncEvent::Created(PimdirHandle::from("99"))]
    );
}

/// Sync options with a conflict policy, everything else default.
fn with_conflict(policy: PimdirConflictPolicy) -> PimdirSyncOptions {
    PimdirSyncOptions {
        conflict: policy,
        ..Default::default()
    }
}

#[test]
fn prefer_remote_drops_the_local_edit() {
    let mut sync = PimdirSync::new("inbox", with_conflict(PimdirConflictPolicy::PreferRemote));
    let (pushes, writes, report) = run(&mut sync, vec![edited("1")], vec![remote_rev("1", "r2")]);

    assert!(pushes.is_none(), "prefer-remote pulls, never pushes");
    assert_eq!(report.conflicts, 0);
    assert_eq!(report.refreshed, 1, "the remote content is pulled");
    let pulled = upserted(&writes, "1").expect("a pulled placement");
    assert_eq!(pulled.object, None, "the local edit is dropped");
    assert_eq!(pulled.level, PimdirLevel::Probed);
}

#[test]
fn prefer_local_overwrites_the_remote() {
    let mut sync = PimdirSync::new("inbox", with_conflict(PimdirConflictPolicy::PreferLocal));
    let (pushes, _writes, report) = run(&mut sync, vec![edited("1")], vec![remote_rev("1", "r2")]);

    let pushes = pushes.expect("prefer-local pushes the edit");
    match &pushes[0].kind {
        PimdirChangeKind::Update {
            object, if_match, ..
        } => {
            assert_eq!(object, &PimdirHash::from("h2"));
            assert_eq!(
                if_match.as_deref(),
                Some("r2"),
                "overwrites the current remote revision, not the stale base",
            );
        }
        other => panic!("expected an Update push, got {other:?}"),
    }
    assert_eq!(report.conflicts, 0);
}

#[test]
fn prefer_local_falls_back_to_conflict_when_it_cannot_push() {
    let opts = PimdirSyncOptions {
        conflict: PimdirConflictPolicy::PreferLocal,
        rights: PimdirPushRights {
            content: false,
            ..PimdirPushRights::all()
        },
        ..Default::default()
    };
    let mut sync = PimdirSync::new("inbox", opts);
    let (pushes, _writes, report) = run(&mut sync, vec![edited("1")], vec![remote_rev("1", "r2")]);

    assert!(pushes.is_none());
    assert_eq!(report.conflicts, 1, "no push right, so it stays a conflict");
}

#[test]
fn keep_both_pulls_the_remote_and_stages_the_local_body() {
    let mut sync = PimdirSync::new("inbox", with_conflict(PimdirConflictPolicy::KeepBoth));
    let (pushes, writes, report) = run(&mut sync, vec![edited("1")], vec![remote_rev("1", "r2")]);

    assert!(
        pushes.is_none(),
        "the duplicate is staged, pushed next sync"
    );
    assert_eq!(report.conflicts, 0);
    assert_eq!(
        report.refreshed, 1,
        "the remote is pulled into the placement"
    );
    let dup = writes
        .iter()
        .find_map(|w| match w {
            PimdirWriteOp::UpsertPlacement(p) if p.status == PimdirStatus::Created => Some(p),
            _ => None,
        })
        .expect("a keep-both duplicate");
    assert_eq!(
        dup.object,
        Some(PimdirHash::from("h2")),
        "the duplicate carries the local body",
    );
    assert!(
        dup.handle.as_str().contains("h2"),
        "the handle is per forked body, so two resolutions never collide",
    );
    assert!(
        dup.link_id.is_some(),
        "the duplicate needs an identity: a link id is what makes a \
         retried add idempotent and what a shared-item storage keys on",
    );
}

/// Both are staged before either is pushed, so the handles must differ.
#[test]
fn two_keep_both_duplicates_of_one_handle_do_not_collide() {
    let mut first = edited("1");
    first.object = Some(PimdirHash::from("h2"));
    let mut second = edited("1");
    second.object = Some(PimdirHash::from("h3"));

    let dup_of = |local: PimdirPlacement| {
        let mut sync = PimdirSync::new("inbox", with_conflict(PimdirConflictPolicy::KeepBoth));
        let (_pushes, writes, _report) = run(&mut sync, vec![local], vec![remote_rev("1", "r2")]);
        writes
            .iter()
            .find_map(|w| match w {
                PimdirWriteOp::UpsertPlacement(p) if p.status == PimdirStatus::Created => {
                    Some(p.clone())
                }
                _ => None,
            })
            .expect("a keep-both duplicate")
    };

    let first = dup_of(first);
    let second = dup_of(second);
    assert_ne!(first.handle, second.handle);
    assert_ne!(first.link_id, second.link_id);
}

/// Two placements forking one body in one run must not share a key.
#[test]
fn two_keep_both_duplicates_of_one_body_do_not_collide() {
    let mut first = edited("1");
    first.object = Some(PimdirHash::from("h2"));
    let mut second = edited("2");
    second.object = Some(PimdirHash::from("h2"));

    let mut sync = PimdirSync::new("inbox", with_conflict(PimdirConflictPolicy::KeepBoth));
    let (_pushes, writes, _report) = run(
        &mut sync,
        vec![first, second],
        vec![remote_rev("1", "r2"), remote_rev("2", "r2")],
    );

    let dups: Vec<&PimdirPlacement> = writes
        .iter()
        .filter_map(|w| match w {
            PimdirWriteOp::UpsertPlacement(p) if p.status == PimdirStatus::Created => Some(p),
            _ => None,
        })
        .collect();
    assert_eq!(dups.len(), 2, "one fork per resolved placement");
    assert_ne!(dups[0].handle, dups[1].handle);
    assert_ne!(
        dups[0].link_id, dups[1].link_id,
        "the placement each fork came from names it, not just the body",
    );
}

/// A reverted delete undoes the delete alone: the rest is still owed.
#[test]
fn a_reverted_delete_keeps_what_it_did_not_undo() {
    let read_only = PimdirSyncOptions {
        push: false,
        ..Default::default()
    };

    let mut edited_tomb = edited("1");
    edited_tomb.status = PimdirStatus::Tombstone;
    let mut sync = PimdirSync::new("inbox", read_only);
    let (_pushes, writes, _report) = run(&mut sync, vec![edited_tomb], vec![remote_rev("1", "r1")]);
    let reverted = upserted(&writes, "1").expect("the reverted tombstone");
    assert_eq!(reverted.status, PimdirStatus::Dirty);
    assert_eq!(reverted.staged_edit(), Some(&PimdirHash::from("h2")));

    let mut conflicted_tomb = edited("2");
    conflicted_tomb.status = PimdirStatus::Tombstone;
    conflicted_tomb.conflict_revision = Some("r2".into());
    let mut sync = PimdirSync::new("inbox", read_only);
    let (_pushes, writes, _report) = run(
        &mut sync,
        vec![conflicted_tomb],
        vec![remote_rev("2", "r1")],
    );
    let reverted = upserted(&writes, "2").expect("the reverted tombstone");
    assert_eq!(
        reverted.status,
        PimdirStatus::Conflict,
        "reverting the delete does not decide the divergence",
    );
}

/// Left behind, the destination would relocate the next plain delete.
#[test]
fn a_reverted_move_drops_the_destination_it_was_going_to() {
    let mut moved = synced("1", &[]);
    moved.status = PimdirStatus::Tombstone;
    moved.origin = Some(PimdirOrigin {
        collection: "archive".into(),
        handle: PimdirHandle::from("1"),
    });

    let mut sync = PimdirSync::new("inbox", with_rights(true, true, true, false));
    let (_pushes, writes, _report) = run(&mut sync, vec![moved], vec![remote("1", &[])]);

    let reverted = upserted(&writes, "1").expect("the reverted tombstone");
    assert_eq!(reverted.status, PimdirStatus::Clean);
    assert_eq!(reverted.origin, None);
}

/// A source refusing content pushes must not land the placement clean.
#[test]
fn a_flag_rebase_leaves_a_staged_edit_pending() {
    let mut local = edited("1");
    local.flags = PimdirFlags::from_iter(["seen"]);

    let mut sync = PimdirSync::new("inbox", with_rights(true, false, true, true));
    let (pushes, writes, _report) = run(&mut sync, vec![local], vec![remote_rev("1", "r1")]);

    assert!(
        pushes
            .expect("the permitted flag push")
            .iter()
            .all(|c| matches!(c.kind, PimdirChangeKind::SetFlags { .. })),
        "the forbidden content push is withheld",
    );
    let rebased = upserted(&writes, "1").expect("the rebased placement");
    assert_eq!(rebased.status, PimdirStatus::Dirty);
    assert_eq!(rebased.staged_edit(), Some(&PimdirHash::from("h2")));
}
