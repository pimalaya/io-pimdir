use alloc::{string::ToString, vec};

use crate::{
    load::PimdirLoaded,
    mutate::*,
    object::PimdirHash,
    placement::{PimdirBase, PimdirLevel, PimdirStatus},
};

fn loaded(handle: &str) -> PimdirLoaded {
    crate::testlog::init();
    PimdirLoaded {
        placements: vec![PimdirPlacement {
            sort_key: Default::default(),
            collection: "inbox".into(),
            handle: PimdirHandle::from(handle),
            link_id: Some(PimdirLinkId::from(handle)),
            object: None,
            level: PimdirLevel::Meta,
            summary: None,
            flags: PimdirFlags::default(),
            conflict_revision: None,
            conflict_object: None,
            status: PimdirStatus::Clean,
            base: Some(PimdirBase {
                flags: PimdirFlags::default(),
                revision: None,
                object: None,
            }),
            origin: None,
        }],
        checkpoint: None,
    }
}

#[test]
fn set_flags_marks_dirty() {
    let mutation = PimdirMutation::SetFlags {
        handle: PimdirHandle::from("1"),
        flags: PimdirFlags::from_iter(["seen"]),
    };
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);

    let ops = match mutate.resume(Some(PimdirArg::Load(loaded("1")))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };
    let PimdirWriteOp::UpsertPlacement(p) = &ops[0] else {
        panic!("expected UpsertPlacement, got {:?}", ops[0]);
    };
    assert_eq!(p.status, PimdirStatus::Dirty);
    assert!(p.flags.contains("seen"));
    assert!(p.base.is_some(), "base must be preserved for sync");
}

/// The flag rides along, so the sync never reads the row as plain dirty.
#[test]
fn set_flags_on_a_conflicted_placement_keeps_the_conflict() {
    let mutation = PimdirMutation::SetFlags {
        handle: PimdirHandle::from("1"),
        flags: PimdirFlags::from_iter(["seen"]),
    };
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);

    let mut loaded = loaded("1");
    loaded.placements[0].status = PimdirStatus::Conflict;
    loaded.placements[0].conflict_revision = Some("r2".into());

    let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };
    let PimdirWriteOp::UpsertPlacement(p) = &ops[0] else {
        panic!("expected UpsertPlacement, got {:?}", ops[0]);
    };
    assert_eq!(p.status, PimdirStatus::Conflict);
    assert_eq!(p.conflict_revision.as_deref(), Some("r2"));
    assert!(p.flags.contains("seen"));
}

/// A pending create keeps its status, else the sync never pushes the add.
#[test]
fn set_flags_on_a_created_placement_stays_created() {
    let mutation = PimdirMutation::SetFlags {
        handle: PimdirHandle::from("1"),
        flags: PimdirFlags::from_iter(["seen"]),
    };
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);

    let mut loaded = loaded("1");
    loaded.placements[0].status = PimdirStatus::Created;
    loaded.placements[0].base = None;

    let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };
    let PimdirWriteOp::UpsertPlacement(p) = &ops[0] else {
        panic!("expected UpsertPlacement, got {:?}", ops[0]);
    };
    assert_eq!(p.status, PimdirStatus::Created);
    assert!(p.flags.contains("seen"));
}

#[test]
fn remove_marks_tombstone() {
    let mutation = PimdirMutation::Remove(PimdirHandle::from("1"));
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);

    let ops = match mutate.resume(Some(PimdirArg::Load(loaded("1")))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };
    let PimdirWriteOp::UpsertPlacement(p) = &ops[0] else {
        panic!("expected UpsertPlacement, got {:?}", ops[0]);
    };
    assert_eq!(p.status, PimdirStatus::Tombstone);
}

#[test]
fn unknown_handle_errors() {
    let mutation = PimdirMutation::Remove(PimdirHandle::from("nope"));
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);

    match mutate.resume(Some(PimdirArg::Load(loaded("1")))) {
        PimdirCoroutineState::Complete(Err(PimdirMutateError::UnknownHandle(h))) => {
            assert_eq!(h, "nope");
        }
        state => panic!("expected UnknownHandle, got {state:?}"),
    }
}

/// A probe has no identity to stage anything under.
#[test]
fn a_probe_refuses_a_mutation() {
    let mutation = PimdirMutation::Remove(PimdirHandle::from("1"));
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);

    let mut probed = loaded("1");
    probed.placements[0].link_id = None;
    probed.placements[0].level = PimdirLevel::Probed;
    probed.placements[0].base = None;

    match mutate.resume(Some(PimdirArg::Load(probed))) {
        PimdirCoroutineState::Complete(Err(PimdirMutateError::Probed(h))) => {
            assert_eq!(h, "1");
        }
        state => panic!("expected Probed, got {state:?}"),
    }
}

/// No base and no origin: the shape the sync pushes as an append.
#[test]
fn add_stages_an_append_create() {
    use crate::object::{PimdirHash, PimdirObject};

    let mutation = PimdirMutation::Add {
        sort_key: Default::default(),
        handle: PimdirHandle::from("draft-1"),
        link_id: PimdirLinkId("mid:new".into()),
        flags: PimdirFlags::from_iter(["\\Draft"]),
        object: PimdirObject {
            hash: PimdirHash::from("deadbeef"),
            size: 5,
        },
        body: b"hello".to_vec(),
        summary: Some(crate::summary::stub("{\"v\":1}")),
    };
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);

    let ops = match mutate.resume(Some(PimdirArg::Load(loaded("other")))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };

    let PimdirWriteOp::StoreObject { body, object } = &ops[0] else {
        panic!("expected StoreObject, got {:?}", ops[0]);
    };
    assert_eq!(body.as_deref(), Some(&b"hello"[..]));
    assert_eq!(object.hash, PimdirHash::from("deadbeef"));

    let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
        panic!("expected UpsertPlacement, got {:?}", ops[1]);
    };
    assert_eq!(p.status, PimdirStatus::Created);
    assert!(p.base.is_none(), "no prior sync");
    assert!(p.origin.is_none(), "an append, not a server copy");
    assert_eq!(p.link_id, Some(PimdirLinkId("mid:new".into())));
    assert_eq!(p.level, PimdirLevel::Full);
    assert!(p.flags.contains("\\Draft"));
}

#[test]
fn add_rejects_a_live_link_id_collision() {
    use crate::object::{PimdirHash, PimdirObject};

    let mutation = PimdirMutation::Add {
        sort_key: Default::default(),
        handle: PimdirHandle::from("draft-1"),
        link_id: PimdirLinkId("mid:dup".into()),
        flags: PimdirFlags::default(),
        object: PimdirObject {
            hash: PimdirHash::from("deadbeef"),
            size: 1,
        },
        body: b"x".to_vec(),
        summary: None,
    };
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);

    let mut loaded = loaded("existing");
    loaded.placements[0].link_id = Some(PimdirLinkId("mid:dup".into()));

    match mutate.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Complete(Err(PimdirMutateError::LinkExists(l))) => {
            assert_eq!(l, "mid:dup");
        }
        state => panic!("expected LinkExists, got {state:?}"),
    }
}

/// The delete is in flight and the new item supersedes it.
#[test]
fn add_over_a_tombstone_link_id_is_allowed() {
    use crate::object::{PimdirHash, PimdirObject};

    let mutation = PimdirMutation::Add {
        sort_key: Default::default(),
        handle: PimdirHandle::from("draft-1"),
        link_id: PimdirLinkId("mid:gone".into()),
        flags: PimdirFlags::default(),
        object: PimdirObject {
            hash: PimdirHash::from("deadbeef"),
            size: 1,
        },
        body: b"x".to_vec(),
        summary: None,
    };
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);

    let mut loaded = loaded("existing");
    loaded.placements[0].link_id = Some(PimdirLinkId("mid:gone".into()));
    loaded.placements[0].status = PimdirStatus::Tombstone;

    match mutate.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(_)) => {}
        state => panic!("expected WantsWrite, got {state:?}"),
    }
}

#[test]
fn write_completes() {
    let mutation = PimdirMutation::Remove(PimdirHandle::from("1"));
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);
    let _ = mutate.resume(Some(PimdirArg::Load(loaded("1"))));

    match mutate.resume(Some(PimdirArg::Write)) {
        PimdirCoroutineState::Complete(Ok(())) => {}
        state => panic!("expected Complete(Ok), got {state:?}"),
    }
}

#[test]
fn missing_arg_errors() {
    let mutation = PimdirMutation::Remove(PimdirHandle::from("1"));
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);

    match mutate.resume(None) {
        PimdirCoroutineState::Complete(Err(PimdirMutateError::Arg(PimdirArgError::MissingArg))) => {
        }
        state => panic!("expected MissingArg, got {state:?}"),
    }
}

/// A caller resuming a finished coroutine is told, not handed a success.
#[test]
fn a_completed_mutate_does_not_resume() {
    let mutation = PimdirMutation::Remove(PimdirHandle::from("1"));
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);
    let _ = mutate.resume(Some(PimdirArg::Load(loaded("1"))));
    let _ = mutate.resume(Some(PimdirArg::Write));

    match mutate.resume(Some(PimdirArg::Write)) {
        PimdirCoroutineState::Complete(Err(PimdirMutateError::Arg(
            PimdirArgError::UnexpectedArg,
        ))) => {}
        state => panic!("expected UnexpectedArg, got {state:?}"),
    }
    match mutate.resume(None) {
        PimdirCoroutineState::Complete(Err(PimdirMutateError::Arg(
            PimdirArgError::UnexpectedArg,
        ))) => {}
        state => panic!("expected UnexpectedArg, got {state:?}"),
    }
}

#[test]
fn unexpected_arg_errors() {
    let mutation = PimdirMutation::Remove(PimdirHandle::from("1"));
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);

    match mutate.resume(Some(PimdirArg::Write)) {
        PimdirCoroutineState::Complete(Err(PimdirMutateError::Arg(
            PimdirArgError::UnexpectedArg,
        ))) => {}
        state => panic!("expected UnexpectedArg, got {state:?}"),
    }
}

/// The base keeps the synced state, so the next sync derives the push.
#[test]
fn edit_stages_a_dirty_body() {
    use crate::object::{PimdirHash, PimdirObject};

    let mutation = PimdirMutation::Edit {
        sort_key: Default::default(),
        handle: PimdirHandle::from("1"),
        object: PimdirObject {
            hash: PimdirHash::from("h2"),
            size: 4,
        },
        body: b"body".to_vec(),
        summary: None,
    };
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);

    let ops = match mutate.resume(Some(PimdirArg::Load(loaded("1")))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };

    assert!(
        matches!(&ops[0], PimdirWriteOp::StoreObject { object, .. } if object.hash == PimdirHash::from("h2"))
    );
    let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
        panic!("expected UpsertPlacement, got {:?}", ops[1]);
    };
    assert_eq!(p.status, PimdirStatus::Dirty);
    assert_eq!(p.object, Some(PimdirHash::from("h2")));
    assert_eq!(p.level, PimdirLevel::Full);
    assert!(p.base.is_some(), "base must be preserved for sync");
}

/// An edit says when the key moves; one that says nothing leaves it.
#[test]
fn an_edit_restates_the_sort_key_or_keeps_it() {
    use crate::object::{PimdirHash, PimdirObject};

    let edit = |sort_key: Option<PimdirSortKey>| {
        let mutation = PimdirMutation::Edit {
            sort_key,
            handle: PimdirHandle::from("1"),
            object: PimdirObject {
                hash: PimdirHash::from("h2"),
                size: 4,
            },
            body: b"body".to_vec(),
            summary: None,
        };
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);

        let mut loaded = loaded("1");
        loaded.placements[0].sort_key = PimdirSortKey::from("2026-01-01");
        let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };
        let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
            panic!("expected UpsertPlacement, got {:?}", ops[1]);
        };
        p.sort_key.clone()
    };

    assert_eq!(edit(None), PimdirSortKey::from("2026-01-01"));
    assert_eq!(
        edit(Some(PimdirSortKey::from("2026-02-02"))),
        PimdirSortKey::from("2026-02-02"),
    );
}

/// No push to pend, so no dirty status `staged_edit` would contradict.
#[test]
fn an_edit_restating_the_synced_body_stages_nothing() {
    use crate::object::{PimdirHash, PimdirObject};

    let mutation = PimdirMutation::Edit {
        sort_key: Default::default(),
        handle: PimdirHandle::from("1"),
        object: PimdirObject {
            hash: PimdirHash::from("h1"),
            size: 4,
        },
        body: b"body".to_vec(),
        summary: None,
    };
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);

    let mut loaded = loaded("1");
    loaded.placements[0].object = Some(PimdirHash::from("h1"));
    loaded.placements[0].level = PimdirLevel::Full;
    loaded.placements[0].base.as_mut().expect("a base").object = Some(PimdirHash::from("h1"));

    let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };
    let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
        panic!("expected UpsertPlacement, got {:?}", ops[1]);
    };
    assert_eq!(p.status, PimdirStatus::Clean);
    assert_eq!(p.staged_edit(), None, "the status agrees with the reading");
}

/// Keeping the ancestor is a decision the remote has to hear.
#[test]
fn resolving_a_conflict_with_the_base_body_still_pushes() {
    use crate::object::{PimdirHash, PimdirObject};

    let mutation = PimdirMutation::Edit {
        sort_key: Default::default(),
        handle: PimdirHandle::from("1"),
        object: PimdirObject {
            hash: PimdirHash::from("h1"),
            size: 4,
        },
        body: b"body".to_vec(),
        summary: None,
    };
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);

    let mut loaded = loaded("1");
    loaded.placements[0].status = PimdirStatus::Conflict;
    loaded.placements[0].object = Some(PimdirHash::from("h2"));
    loaded.placements[0].level = PimdirLevel::Full;
    loaded.placements[0].conflict_revision = Some("r2".into());
    loaded.placements[0].conflict_object = Some(PimdirHash::from("h-remote"));
    loaded.placements[0].base.as_mut().expect("a base").object = Some(PimdirHash::from("h1"));

    let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };
    let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
        panic!("expected UpsertPlacement, got {:?}", ops[1]);
    };
    assert_eq!(p.status, PimdirStatus::Dirty);
    assert_eq!(p.conflict_revision, None);
    assert_eq!(p.conflict_object, None);
}

/// The base takes revision and body together.
///
/// The revision alone would claim one the base object was never the
/// content of, and the next sync reads a resolution keeping the
/// ancestor as nothing to push.
#[test]
fn a_resolution_adopts_the_whole_remote_state_into_the_base() {
    use crate::object::{PimdirHash, PimdirObject};

    let mutation = PimdirMutation::Edit {
        sort_key: Default::default(),
        handle: PimdirHandle::from("1"),
        object: PimdirObject {
            hash: PimdirHash::from("h-base"),
            size: 4,
        },
        body: b"base".to_vec(),
        summary: None,
    };
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);

    let mut loaded = loaded("1");
    loaded.placements[0].status = PimdirStatus::Conflict;
    loaded.placements[0].object = Some(PimdirHash::from("h-local"));
    loaded.placements[0].level = PimdirLevel::Full;
    loaded.placements[0].conflict_revision = Some("r2".into());
    loaded.placements[0].conflict_object = Some(PimdirHash::from("h-remote"));
    let base = loaded.placements[0].base.as_mut().expect("a base");
    base.revision = Some("r1".into());
    base.object = Some(PimdirHash::from("h-base"));

    let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };
    let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
        panic!("expected UpsertPlacement, got {:?}", ops[1]);
    };
    let base = p.base.as_ref().expect("a base");
    assert_eq!(base.revision.as_deref(), Some("r2"));
    assert_eq!(
        base.object,
        Some(PimdirHash::from("h-remote")),
        "the base object is the body the adopted revision names",
    );
    assert_eq!(
        p.staged_edit(),
        Some(&PimdirHash::from("h-base")),
        "so the ancestor the resolution kept is a body to push",
    );
}

/// A create collision has no ancestor, and left base-less it never pushes.
#[test]
fn a_resolution_gives_a_base_less_conflict_a_base() {
    use crate::object::{PimdirHash, PimdirObject};

    let mutation = PimdirMutation::Edit {
        sort_key: Default::default(),
        handle: PimdirHandle::from("1"),
        object: PimdirObject {
            hash: PimdirHash::from("h-merged"),
            size: 6,
        },
        body: b"merged".to_vec(),
        summary: None,
    };
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);

    let mut loaded = loaded("1");
    loaded.placements[0].status = PimdirStatus::Conflict;
    loaded.placements[0].object = Some(PimdirHash::from("h-local"));
    loaded.placements[0].level = PimdirLevel::Full;
    loaded.placements[0].conflict_revision = Some("r2".into());
    loaded.placements[0].conflict_object = Some(PimdirHash::from("h-remote"));
    loaded.placements[0].base = None;

    let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };
    let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
        panic!("expected UpsertPlacement, got {:?}", ops[1]);
    };
    let base = p.base.as_ref().expect("the resolution establishes a base");
    assert_eq!(base.revision.as_deref(), Some("r2"));
    assert_eq!(base.object, Some(PimdirHash::from("h-remote")));
    assert_eq!(base.flags, p.flags, "nothing else is known of it");
}

/// A summary projected from the edited body replaces the cached one.
#[test]
fn edit_refreshes_the_projected_meta() {
    use crate::object::{PimdirHash, PimdirObject};

    let mutation = PimdirMutation::Edit {
        sort_key: Default::default(),
        handle: PimdirHandle::from("1"),
        object: PimdirObject {
            hash: PimdirHash::from("h2"),
            size: 4,
        },
        body: b"body".to_vec(),
        summary: Some(crate::summary::stub("fresh")),
    };
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);

    let ops = match mutate.resume(Some(PimdirArg::Load(loaded("1")))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };
    let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
        panic!("expected UpsertPlacement, got {:?}", ops[1]);
    };
    assert_eq!(p.summary, Some(crate::summary::stub("fresh")));
}

/// The base adopts the observed revision and the recorded pair is cleared.
#[test]
fn edit_resolves_a_conflict() {
    use crate::object::{PimdirHash, PimdirObject};

    let mutation = PimdirMutation::Edit {
        sort_key: Default::default(),
        handle: PimdirHandle::from("1"),
        object: PimdirObject {
            hash: PimdirHash::from("h3"),
            size: 6,
        },
        body: b"merged".to_vec(),
        summary: None,
    };
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);

    let mut loaded = loaded("1");
    loaded.placements[0].status = PimdirStatus::Conflict;
    loaded.placements[0].conflict_revision = Some("r2".into());
    loaded.placements[0].conflict_object = Some(PimdirHash::from("h-remote"));

    let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };
    let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
        panic!("expected UpsertPlacement, got {:?}", ops[1]);
    };

    assert_eq!(p.status, PimdirStatus::Dirty);
    assert_eq!(p.conflict_revision, None);
    assert_eq!(
        p.conflict_object, None,
        "the diverging body is dropped with the revision it named"
    );
    let base = p.base.as_ref().expect("a base");
    assert_eq!(base.revision.as_deref(), Some("r2"));
}

/// Services the target read a staged create makes, answering `holds`.
fn stage(mutate: &mut PimdirMutate, holds: Vec<PimdirPlacement>) -> Vec<PimdirWriteOp> {
    let loaded = PimdirLoaded {
        placements: holds,
        checkpoint: None,
    };
    match mutate.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    }
}

/// The staged create carries the origin, so the push is a server copy.
#[test]
fn copy_stages_created_placement_in_target() {
    let mutation = PimdirMutation::Copy {
        handle: PimdirHandle::from("1"),
        target: "archive".into(),
        placeholder: PimdirHandle::from("tmp-1"),
    };
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);

    match mutate.resume(Some(PimdirArg::Load(loaded("1")))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsLoad { collection, scope }) => {
            assert_eq!(collection.as_str(), "archive");
            assert_eq!(
                scope,
                PimdirLoadScope::Links(vec![
                    PimdirLinkId::from("1"),
                    PimdirLinkId::from("dup:1#tmp-1"),
                ]),
            );
        }
        state => panic!("expected WantsLoad, got {state:?}"),
    }

    let ops = stage(&mut mutate, Vec::new());
    let PimdirWriteOp::UpsertPlacement(p) = &ops[0] else {
        panic!("expected UpsertPlacement, got {:?}", ops[0]);
    };
    assert_eq!(p.collection.as_str(), "archive");
    assert_eq!(p.handle.as_str(), "tmp-1");
    assert_eq!(
        p.link_id,
        Some(PimdirLinkId::from("1")),
        "the identity is the source's while the target has it free",
    );
    assert_eq!(p.status, PimdirStatus::Created);
    assert!(p.base.is_none());
    let origin = p.origin.as_ref().expect("the copy carries its origin");
    assert_eq!(origin.collection.as_str(), "inbox");
    assert_eq!(origin.handle.as_str(), "1");
}

/// The target's half copies and the source's half removes.
#[test]
fn move_stages_target_create_and_source_tombstone() {
    let mutation = PimdirMutation::Move {
        handle: PimdirHandle::from("1"),
        target: "archive".into(),
        placeholder: PimdirHandle::from("tmp-1"),
    };
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);
    let _ = mutate.resume(Some(PimdirArg::Load(loaded("1"))));
    let ops = stage(&mut mutate, Vec::new());

    let PimdirWriteOp::UpsertPlacement(create) = &ops[0] else {
        panic!("expected UpsertPlacement, got {:?}", ops[0]);
    };
    assert_eq!(create.collection.as_str(), "archive");
    assert_eq!(create.handle.as_str(), "tmp-1");
    assert_eq!(create.status, PimdirStatus::Created);
    assert!(create.base.is_none());
    assert_eq!(
        create
            .origin
            .as_ref()
            .expect("the move carries its origin")
            .handle
            .as_str(),
        "1",
    );

    let PimdirWriteOp::UpsertPlacement(source) = &ops[1] else {
        panic!("expected UpsertPlacement, got {:?}", ops[1]);
    };
    assert_eq!(
        source.collection.as_str(),
        "inbox",
        "the source row, tombstoned"
    );
    assert_eq!(source.status, PimdirStatus::Tombstone);
    assert_eq!(
        source
            .origin
            .as_ref()
            .expect("a move destination, so a source-first sync relocates rather than deletes")
            .collection
            .as_str(),
        "archive",
    );
}

/// The copy lands beside the held identity as the second resource it is.
///
/// Under the same key, a storage keying by identity would keep one of
/// the two rows, and the other's body with it.
#[test]
fn a_copy_into_a_collection_holding_the_identity_is_minted() {
    let mutation = PimdirMutation::Copy {
        handle: PimdirHandle::from("1"),
        target: "archive".into(),
        placeholder: PimdirHandle::from("tmp-1"),
    };
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);
    let _ = mutate.resume(Some(PimdirArg::Load(loaded("1"))));

    let mut holder = loaded("1").placements.remove(0);
    holder.collection = "archive".into();
    holder.handle = PimdirHandle::from("a1");
    let ops = stage(&mut mutate, vec![holder]);

    let PimdirWriteOp::UpsertPlacement(copy) = &ops[0] else {
        panic!("expected UpsertPlacement, got {:?}", ops[0]);
    };
    assert_eq!(copy.link_id, Some(PimdirLinkId::from("dup:1#tmp-1")));
}

/// A row on its way out holds no key against a create, as for an `Add`.
#[test]
fn a_tombstoned_holder_does_not_block_a_copy() {
    let mutation = PimdirMutation::Copy {
        handle: PimdirHandle::from("1"),
        target: "archive".into(),
        placeholder: PimdirHandle::from("tmp-1"),
    };
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);
    let _ = mutate.resume(Some(PimdirArg::Load(loaded("1"))));

    let mut holder = loaded("1").placements.remove(0);
    holder.collection = "archive".into();
    holder.handle = PimdirHandle::from("a1");
    holder.status = PimdirStatus::Tombstone;
    let ops = stage(&mut mutate, vec![holder]);

    let PimdirWriteOp::UpsertPlacement(copy) = &ops[0] else {
        panic!("expected UpsertPlacement, got {:?}", ops[0]);
    };
    assert_eq!(copy.link_id, Some(PimdirLinkId::from("1")));
}

/// A mutation touches one row; an `Add` sees only the rows it may hit.
#[test]
fn a_mutation_reads_only_what_it_edits() {
    let mut mutate = PimdirMutate::new("inbox", PimdirMutation::Remove("7".into()));
    match mutate.resume(None) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsLoad { scope, .. }) => {
            assert_eq!(
                scope,
                PimdirLoadScope::Handles(vec![PimdirHandle::from("7")])
            );
        }
        state => panic!("expected WantsLoad, got {state:?}"),
    }

    let add = PimdirMutation::Add {
        handle: PimdirHandle::from("tmp"),
        link_id: PimdirLinkId::from("m1"),
        flags: PimdirFlags::default(),
        object: PimdirObject {
            hash: PimdirHash::from("h"),
            size: 1,
        },
        body: vec![],
        summary: None,
        sort_key: Default::default(),
    };
    let mut mutate = PimdirMutate::new("inbox", add);
    match mutate.resume(None) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsLoad { scope, .. }) => {
            assert_eq!(
                scope,
                PimdirLoadScope::Links(vec![PimdirLinkId::from("m1")]),
            );
        }
        state => panic!("expected WantsLoad, got {state:?}"),
    }
}

/// An edited tombstone is revived, its destination going with the delete.
#[test]
fn an_edit_revives_a_tombstone_and_drops_its_destination() {
    use crate::object::{PimdirHash, PimdirObject};

    let mut loaded = loaded("1");
    loaded.placements[0].status = PimdirStatus::Tombstone;
    loaded.placements[0].origin = Some(PimdirOrigin {
        collection: "archive".into(),
        handle: PimdirHandle::from("1"),
    });

    let mutation = PimdirMutation::Edit {
        sort_key: None,
        handle: PimdirHandle::from("1"),
        object: PimdirObject {
            hash: PimdirHash::from("h2"),
            size: 4,
        },
        body: b"body".to_vec(),
        summary: None,
    };
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);
    let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };

    let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
        panic!("expected UpsertPlacement, got {:?}", ops[1]);
    };
    assert_eq!(p.status, PimdirStatus::Dirty);
    assert_eq!(p.origin, None, "a revived row is going nowhere: {p:?}",);
}

/// A server copy from the origin would deliver the body the edit replaced.
#[test]
fn an_edit_on_a_pending_create_drops_its_origin() {
    use crate::object::{PimdirHash, PimdirObject};

    let mut loaded = loaded("tmp-1");
    loaded.placements[0].status = PimdirStatus::Created;
    loaded.placements[0].base = None;
    loaded.placements[0].origin = Some(PimdirOrigin {
        collection: "inbox".into(),
        handle: PimdirHandle::from("1"),
    });

    let mutation = PimdirMutation::Edit {
        sort_key: None,
        handle: PimdirHandle::from("tmp-1"),
        object: PimdirObject {
            hash: PimdirHash::from("h2"),
            size: 4,
        },
        body: b"body".to_vec(),
        summary: None,
    };
    let mut mutate = PimdirMutate::new("archive", mutation);
    let _ = mutate.resume(None);
    let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };

    let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
        panic!("expected UpsertPlacement, got {:?}", ops[1]);
    };
    assert_eq!(p.status, PimdirStatus::Created, "still a pending create");
    assert_eq!(p.object, Some(PimdirHash::from("h2")));
    assert_eq!(p.origin, None, "uploaded, never copied: {p:?}");
}

/// A flag change is not content: the delete stands, destination included.
#[test]
fn a_flag_change_leaves_a_tombstone_deleted() {
    let mut loaded = loaded("1");
    loaded.placements[0].status = PimdirStatus::Tombstone;
    loaded.placements[0].origin = Some(PimdirOrigin {
        collection: "archive".into(),
        handle: PimdirHandle::from("1"),
    });

    let mutation = PimdirMutation::SetFlags {
        handle: PimdirHandle::from("1"),
        flags: PimdirFlags::from_iter(["seen"]),
    };
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);
    let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };

    let PimdirWriteOp::UpsertPlacement(p) = &ops[0] else {
        panic!("expected UpsertPlacement, got {:?}", ops[0]);
    };
    assert_eq!(p.status, PimdirStatus::Tombstone);
    assert!(p.flags.contains("seen"), "the marker rides along");
    assert!(p.origin.is_some(), "and the move it was part of stands");
}

/// Editing the diverged tombstone a hub projects is the resolution.
#[test]
fn an_edit_resolves_a_divergence_a_tombstone_carries() {
    use crate::object::{PimdirHash, PimdirObject};

    let mut loaded = loaded("1");
    loaded.placements[0].status = PimdirStatus::Tombstone;
    loaded.placements[0].conflict_revision = Some("r2".into());
    loaded.placements[0].conflict_object = Some(PimdirHash::from("remote"));
    loaded.placements[0].base = Some(PimdirBase {
        flags: PimdirFlags::default(),
        revision: Some("r1".into()),
        object: Some(PimdirHash::from("h1")),
    });

    let mutation = PimdirMutation::Edit {
        sort_key: None,
        handle: PimdirHandle::from("1"),
        object: PimdirObject {
            hash: PimdirHash::from("merged"),
            size: 6,
        },
        body: b"merged".to_vec(),
        summary: None,
    };
    let mut mutate = PimdirMutate::new("inbox", mutation);
    let _ = mutate.resume(None);
    let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
        state => panic!("expected WantsWrite, got {state:?}"),
    };

    let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
        panic!("expected UpsertPlacement, got {:?}", ops[1]);
    };
    assert_eq!(p.status, PimdirStatus::Dirty);
    assert_eq!(p.conflict_revision, None, "the divergence is settled");
    assert_eq!(p.conflict_object, None);
    let base = p.base.as_ref().expect("a base");
    assert_eq!(
        base.revision.as_deref(),
        Some("r2"),
        "measured against the remote state it settled",
    );
    assert_eq!(base.object, Some(PimdirHash::from("remote")));
}

/// A failure names its cause, a contract break riding as the source.
#[test]
fn a_mutate_failure_says_which_it_is() {
    let unknown = PimdirMutateError::UnknownHandle("7".into());
    assert_eq!(
        unknown.to_string(),
        "Pimdir MUTATE failed: unknown handle 7",
    );
    assert!(error::Error::source(&unknown).is_none());

    let exists = PimdirMutateError::LinkExists("mid".into());
    assert_eq!(
        exists.to_string(),
        "Pimdir MUTATE failed: link id already present: mid",
    );

    let arg = PimdirMutateError::from(PimdirArgError::MissingArg);
    assert_eq!(arg.to_string(), PimdirArgError::MissingArg.to_string());
    assert!(error::Error::source(&arg).is_some());
}
