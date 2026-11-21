//! The cross-source merge of one upsert as a pure verdict (SYNC §9): what
//! a storage applying upserts itself reads to keep the body a policy did
//! not adopt, and to report a divergence, as the hub does.

use std::collections::BTreeMap;

use io_pimdir::{
    collection::PimdirCollectionId,
    hub::{
        PimdirBinding, PimdirHubConflict, PimdirHubItem, PimdirHubLoser, PimdirHubLoserSide,
        PimdirHubVerdict, PimdirSourceId,
    },
    object::PimdirHash,
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
};

fn hash(name: &str) -> PimdirHash {
    PimdirHash::from(name)
}

fn base(revision: &str, object: &str) -> PimdirBase {
    PimdirBase {
        flags: PimdirFlags::default(),
        revision: Some(revision.into()),
        object: Some(hash(object)),
    }
}

/// An item holding `shared`, bound by `a` and `b`, each having agreed on
/// `agreed`.
fn item(shared: Option<&str>, agreed: &str) -> PimdirHubItem {
    let binding = |handle: &str| PimdirBinding {
        handle: PimdirHandle::from(handle),
        base: Some(base("r1", agreed)),
        conflicted: false,
        conflict_revision: None,
        conflict_object: None,
        shared_object: Some(hash(agreed)),
        server_object_id: None,
    };
    PimdirHubItem {
        flags: PimdirFlags::default(),
        object: shared.map(hash),
        summary: None,
        sort_key: Default::default(),
        level: PimdirLevel::Full,
        deleted: false,
        conflicted: false,
        conflict_object: None,
        sources: BTreeMap::from([
            (PimdirSourceId::from("a"), binding("a1")),
            (PimdirSourceId::from("b"), binding("b1")),
        ]),
    }
}

/// `b`'s upsert pulling `object` at revision `r2`.
fn pulled(object: Option<&str>, base_object: &str) -> PimdirPlacement {
    PimdirPlacement {
        collection: PimdirCollectionId::from("c"),
        handle: PimdirHandle::from("b1"),
        link_id: Some(PimdirLinkId::from("k")),
        object: object.map(hash),
        level: PimdirLevel::Full,
        summary: None,
        sort_key: Default::default(),
        flags: PimdirFlags::default(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(base("r2", base_object)),
        origin: None,
        server_object_id: None,
    }
}

fn reconcile(
    mut item: PimdirHubItem,
    placement: &PimdirPlacement,
    policy: PimdirHubConflict,
) -> (PimdirHubItem, PimdirHubVerdict) {
    let verdict = item.reconcile(&PimdirSourceId::from("b"), placement, policy);
    (item, verdict)
}

/// Both sources moved from `o0`, `a` to `o1` folded in, `b` to `o2`: two
/// read bodies diverge, and each policy keeps one and loses the other.
#[test]
fn two_read_bodies_lose_the_one_the_policy_does_not_adopt() {
    let upsert = pulled(Some("o2"), "o2");

    let (after, verdict) = reconcile(item(Some("o1"), "o0"), &upsert, PimdirHubConflict::Manual);
    assert!(verdict.diverged && after.conflicted);
    assert_eq!(after.object, Some(hash("o1")));
    assert_eq!(after.conflict_object, Some(hash("o2")));
    assert_eq!(verdict.lost, None);

    let (after, verdict) = reconcile(
        item(Some("o1"), "o0"),
        &upsert,
        PimdirHubConflict::PreferIncoming,
    );
    assert!(!verdict.diverged && !after.conflicted);
    assert_eq!(after.object, Some(hash("o2")));
    let lost = PimdirHubLoser {
        object: hash("o1"),
        side: PimdirHubLoserSide::Shared,
    };
    assert_eq!(verdict.lost, Some(lost));

    let (after, verdict) = reconcile(
        item(Some("o1"), "o0"),
        &upsert,
        PimdirHubConflict::PreferExisting,
    );
    assert_eq!(after.object, Some(hash("o1")));
    assert!(verdict.kept_shared);
    let lost = PimdirHubLoser {
        object: hash("o2"),
        side: PimdirHubLoserSide::Incoming,
    };
    assert_eq!(verdict.lost, Some(lost));
}

/// A divergence one of whose bodies nobody read is recorded whatever the
/// policy, losing nothing.
#[test]
fn an_unread_body_diverges_whatever_the_policy() {
    let upsert = pulled(None, "o0");
    let mut upsert = upsert;
    upsert.base = Some(PimdirBase {
        object: None,
        ..base("r2", "o0")
    });

    for policy in [
        PimdirHubConflict::Manual,
        PimdirHubConflict::PreferIncoming,
        PimdirHubConflict::PreferExisting,
    ] {
        let (after, verdict) = reconcile(item(Some("o1"), "o0"), &upsert, policy);
        assert!(verdict.diverged && after.conflicted, "{policy:?}");
        assert_eq!(verdict.lost, None, "{policy:?}");
    }
}

/// A body filling an item a pull emptied settles the item conflict
/// recorded without it, losing the side the policy does not adopt.
#[test]
fn a_fill_settles_the_conflict_recorded_without_it() {
    let mut emptied = item(None, "o0");
    emptied.conflicted = true;
    emptied.conflict_object = Some(hash("o9"));
    for binding in emptied.sources.values_mut() {
        binding.base = Some(PimdirBase {
            object: None,
            ..base("r1", "o0")
        });
    }
    let upsert = pulled(Some("o2"), "o2");

    let (after, verdict) = reconcile(emptied.clone(), &upsert, PimdirHubConflict::PreferExisting);
    assert!(!after.conflicted);
    assert_eq!(after.object, Some(hash("o2")));
    let lost = PimdirHubLoser {
        object: hash("o9"),
        side: PimdirHubLoserSide::Diverging,
    };
    assert_eq!(verdict.lost, Some(lost));

    let (after, verdict) = reconcile(emptied, &upsert, PimdirHubConflict::PreferIncoming);
    assert_eq!(after.object, Some(hash("o9")));
    assert!(verdict.unsummarized);
    let lost = PimdirHubLoser {
        object: hash("o2"),
        side: PimdirHubLoserSide::Incoming,
    };
    assert_eq!(verdict.lost, Some(lost));
}
