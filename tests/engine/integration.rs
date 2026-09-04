//! # End-to-end lifecycle
//!
//! Concept validation over the real store and a fake remote.
//!
//! Exercises the whole lifecycle: initial pull, fully offline open,
//! progressive upgrade with cross-collection object dedup, local
//! mutation, push, remote pull, and a divergent flag merge where both
//! sides survive.

use io_pimdir::{
    mutate::PimdirMutation,
    placement::{PimdirFlags, PimdirHandle, PimdirLevel, PimdirStatus},
    remote::PimdirTier,
    sync::PimdirSyncOptions,
};

use crate::common::{Client, MemRemote};

/// Two inbox members plus an archive copy of the first: the dedup case.
fn seeded_client() -> Client {
    let body_a = b"From: a\r\nMessage-ID: <msg-a>\r\n\r\nshared body\r\n";
    let body_b = b"From: b\r\nMessage-ID: <msg-b>\r\n\r\nother body\r\n";

    let mut remote = MemRemote::default();
    remote.seed("inbox", "i1", "msg-a", &[], body_a);
    remote.seed("inbox", "i2", "msg-b", &[], body_b);
    remote.seed("archive", "a1", "msg-a", &["seen"], body_a);

    Client::new(remote)
}

#[test]
fn full_offline_lifecycle() {
    let mut client = seeded_client();

    let report = client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    assert_eq!(report.pulled, 2, "both inbox items pulled");
    assert_eq!(report.pushed, 0);

    let calls_before = client.remote().calls;
    let loaded = client.open("inbox").unwrap();
    assert_eq!(loaded.placements.len(), 2);
    assert!(
        loaded
            .placements
            .iter()
            .all(|p| p.level == PimdirLevel::Probed)
    );
    assert_eq!(
        client.remote().calls,
        calls_before,
        "open must not hit the remote",
    );

    let report = client
        .upgrade(
            "inbox",
            vec![PimdirHandle::from("i1"), PimdirHandle::from("i2")],
            PimdirTier::Meta,
        )
        .unwrap();
    assert_eq!(report.upgraded, 2);
    assert_eq!(
        client.storage().placement("inbox", "i1").level,
        PimdirLevel::Meta
    );

    let report = client
        .upgrade("inbox", vec![PimdirHandle::from("i1")], PimdirTier::Full)
        .unwrap();
    assert_eq!(report.fetched, 1);
    assert_eq!(report.deduped, 0);
    assert_eq!(
        client.storage().placement("inbox", "i1").level,
        PimdirLevel::Full
    );
    assert_eq!(client.storage().objects(), 1, "one stored body");

    // NOTE: a Meta fetch resolves a1's link id, which enumerate does not
    // carry, so the Full upgrade dedups the shared body by it, fetch-free.
    client
        .sync("archive", PimdirSyncOptions::default())
        .unwrap();
    client
        .upgrade("archive", vec![PimdirHandle::from("a1")], PimdirTier::Meta)
        .unwrap();
    let fetches_before = client.remote().full_fetches.len();

    let report = client
        .upgrade("archive", vec![PimdirHandle::from("a1")], PimdirTier::Full)
        .unwrap();
    assert_eq!(report.deduped, 1, "shared body deduped");
    assert_eq!(report.fetched, 0);
    assert_eq!(
        client.remote().full_fetches.len(),
        fetches_before,
        "dedup must skip the network fetch",
    );
    assert_eq!(client.storage().objects(), 1, "still one stored body");
    assert_eq!(
        client.storage().placement("inbox", "i1").object,
        client.storage().placement("archive", "a1").object,
    );

    client
        .mutate(
            "inbox",
            PimdirMutation::SetFlags {
                handle: PimdirHandle::from("i1"),
                flags: PimdirFlags::from_iter(["seen"]),
            },
        )
        .unwrap();
    assert_eq!(
        client.storage().placement("inbox", "i1").status,
        PimdirStatus::Dirty,
    );

    let report = client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    assert_eq!(report.pushed, 1, "local seen flag pushed");
    assert!(client.remote().flags_of("inbox", "i1").contains("seen"));
    assert_eq!(
        client.storage().placement("inbox", "i1").status,
        PimdirStatus::Clean,
        "pushed placement is rebased clean",
    );

    client.remote_mut().set_flags("inbox", "i2", &["flagged"]);
    let report = client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    assert_eq!(report.pulled, 1);
    assert!(
        client
            .storage()
            .placement("inbox", "i2")
            .flags
            .contains("flagged")
    );

    client
        .mutate(
            "inbox",
            PimdirMutation::SetFlags {
                handle: PimdirHandle::from("i1"),
                flags: PimdirFlags::from_iter(["draft"]),
            },
        )
        .unwrap();
    client
        .remote_mut()
        .set_flags("inbox", "i1", &["seen", "important"]);

    let report = client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    assert_eq!(report.conflicts, 0, "flags never conflict");
    assert_eq!(report.pushed, 1);
    assert_eq!(report.pulled, 1);

    let placement = client.storage().placement("inbox", "i1");
    let merged = &placement.flags;
    assert!(merged.contains("draft"), "the local addition survives");
    assert!(merged.contains("important"), "the remote addition survives");
    assert!(!merged.contains("seen"), "the local removal wins");
    assert_eq!(
        client.remote().flags_of("inbox", "i1"),
        merged,
        "both sides converged on the merged set",
    );
    assert_eq!(placement.status, PimdirStatus::Clean);
}

/// A copy of a synced body pushes as a server-side copy: the store
/// derives the origin from the source binding (SYNC §3).
#[test]
fn offline_copy_creates_pushes_and_rekeys() {
    let mut client = seeded_client();
    let opts = PimdirSyncOptions::default();

    client.sync("inbox", opts).unwrap();
    client
        .upgrade("inbox", vec![PimdirHandle::from("i2")], PimdirTier::Full)
        .unwrap();

    client
        .mutate(
            "inbox",
            PimdirMutation::Copy {
                handle: PimdirHandle::from("i2"),
                target: "archive".into(),
                placeholder: PimdirHandle::from("tmp-i2"),
            },
        )
        .unwrap();
    let staged = client.storage().placement("archive", "tmp-i2");
    assert_eq!(staged.status, PimdirStatus::Created);
    assert_eq!(
        staged.origin.as_ref().map(|o| o.handle.as_str()),
        Some("i2"),
        "the origin the server copies from",
    );
    assert_eq!(
        client.storage().placement("inbox", "i2").status,
        PimdirStatus::Clean,
        "the copy source is untouched",
    );

    let report = client.sync("archive", opts).unwrap();
    assert_eq!(report.pushed, 1);
    assert!(
        !client.storage().contains("archive", "tmp-i2"),
        "the placeholder is dropped once the copy is confirmed",
    );
    let real = client.storage().placement("archive", "i2-copy");
    assert_eq!(real.status, PimdirStatus::Clean);
    assert!(real.base.is_some());
    assert!(real.origin.is_none());
}
