//! # Cross-collection membership
//!
//! A move must deliver exactly one copy.
//!
//! A move is staged as a copy into the target plus a remove from the
//! source (see `PimdirMutation::Move`), so two independent syncs derive
//! the halves. Through the store a binding carries no origin (SYNC §3),
//! so the create uploads the body it holds, and the remove relocates the
//! source only while the target does not hold the identity yet.

use io_pimdir::{
    mutate::PimdirMutation,
    placement::{PimdirHandle, PimdirStatus},
    remote::PimdirTier,
    sync::PimdirSyncOptions,
};

use crate::common::{Client, MemRemote};

/// One hydrated inbox member.
fn seeded_client() -> Client {
    let body = b"From: a\r\nMessage-ID: <msg-a>\r\n\r\nbody\r\n";

    let mut remote = MemRemote::default();
    remote.seed("inbox", "i1", "msg-a", &[], body);

    let mut client = Client::new(remote);
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    client
        .upgrade("inbox", vec![PimdirHandle::from("i1")], PimdirTier::Full)
        .unwrap();
    client
}

/// Members the remote holds in `collection`.
fn remote_members(client: &Client, collection: &str) -> Vec<String> {
    client
        .remote()
        .handles(collection)
        .iter()
        .map(|h| h.as_str().to_string())
        .collect()
}

/// Placements the store holds in `collection`.
fn local_members(client: &Client, collection: &str) -> Vec<String> {
    client
        .storage()
        .rows(collection)
        .iter()
        .map(|p| p.handle.as_str().to_string())
        .collect()
}

fn stage_move(client: &mut Client) {
    client
        .mutate(
            "inbox",
            PimdirMutation::Move {
                handle: PimdirHandle::from("i1"),
                target: "archive".into(),
                placeholder: PimdirHandle::from("tmp-i1"),
            },
        )
        .unwrap();
}

#[test]
fn a_move_synced_target_first_delivers_exactly_one_copy() {
    let mut client = seeded_client();
    let opts = PimdirSyncOptions::default();
    stage_move(&mut client);

    client.sync("archive", opts).unwrap();
    client.sync("inbox", opts).unwrap();

    assert_eq!(
        remote_members(&client, "archive").len(),
        1,
        "the target holds exactly one member, not the copy and a move",
    );
    assert!(
        remote_members(&client, "inbox").is_empty(),
        "the source member is gone",
    );
    assert!(
        local_members(&client, "inbox").is_empty(),
        "the source tombstone is dropped once the remove is confirmed",
    );
    assert_eq!(
        local_members(&client, "archive").len(),
        1,
        "one target placement, no lingering placeholder",
    );
    assert!(
        client.storage().retained("inbox").is_empty(),
        "a move retains nothing: the archive holds the item",
    );
}

#[test]
fn a_copy_leaves_the_source_and_delivers_one_member() {
    let mut client = seeded_client();
    let opts = PimdirSyncOptions::default();

    client
        .mutate(
            "inbox",
            PimdirMutation::Copy {
                handle: PimdirHandle::from("i1"),
                target: "archive".into(),
                placeholder: PimdirHandle::from("tmp-i1"),
            },
        )
        .unwrap();
    client.sync("archive", opts).unwrap();
    client.sync("inbox", opts).unwrap();

    assert_eq!(remote_members(&client, "archive").len(), 1);
    assert_eq!(
        remote_members(&client, "inbox").len(),
        1,
        "a copy leaves the source in place",
    );
    assert_eq!(
        client.storage().placement("inbox", "i1").status,
        PimdirStatus::Clean,
    );
    assert_eq!(local_members(&client, "archive"), ["i1-copy"]);
}
