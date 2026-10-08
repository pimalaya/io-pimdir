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
            },
        )
        .unwrap();
}

#[test]
fn a_move_synced_target_first_delivers_exactly_one_copy() {
    let mut client = seeded_client();
    let opts = PimdirSyncOptions::default();
    stage_move(&mut client);

    client.sync("archive", opts.clone()).unwrap();
    client.sync("inbox", opts.clone()).unwrap();

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
            },
        )
        .unwrap();
    client.sync("archive", opts.clone()).unwrap();
    client.sync("inbox", opts.clone()).unwrap();

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

const ALT: &str = "alt:Hi|2026-08-01T10:00:00Z|a@x.y";

/// One hydrated inbox member keyed `link`, the archive synced too, holding
/// a member of the same identity when `held`.
fn moving_client(link: &str, held: bool) -> Client {
    let body = b"From: a\r\nSubject: Hi\r\n\r\nbody\r\n";

    let mut remote = MemRemote::default();
    remote.seed("inbox", "i1", link, &[], body);
    if held {
        remote.seed("archive", "a1", link, &[], b"From: a\r\n\r\nother\r\n");
    }

    let mut client = Client::new(remote);
    let opts = PimdirSyncOptions::default();
    client.sync("inbox", opts.clone()).unwrap();
    client.sync("archive", opts).unwrap();
    client
        .upgrade("inbox", vec![PimdirHandle::from("i1")], PimdirTier::Full)
        .unwrap();
    client
}

/// The move a connector carries out itself, as an IMAP `MOVE` does: the
/// member leaves the source and is listed in the target under `moved`.
fn server_move(client: &mut Client, link: &str) {
    let remote = client.remote_mut();
    let body = remote.items[&"inbox".into()][&PimdirHandle::from("i1")]
        .body
        .clone();
    remote.remove("inbox", "i1");
    remote.seed("archive", "i1-moved", link, &[], &body);
}

/// Exactly one moved copy on both sides, landed on the listed handle.
fn assert_moved(client: &Client, held: bool) {
    let mut expected = vec!["i1-moved"];
    if held {
        expected.insert(0, "a1");
    }

    assert_eq!(
        remote_members(client, "archive"),
        expected,
        "no second copy pushed",
    );
    assert!(remote_members(client, "inbox").is_empty());
    let mut landed = local_members(client, "archive");
    landed.sort();
    assert_eq!(landed, expected, "create landed");
    assert_eq!(
        client.storage().placement("archive", "i1-moved").status,
        PimdirStatus::Clean,
    );
    assert!(
        local_members(client, "inbox").is_empty(),
        "tombstone dropped"
    );
}

/// The target listing lands the staged create before the source syncs.
fn listing_first(link: &str, held: bool) {
    let mut client = moving_client(link, held);
    let opts = PimdirSyncOptions::default();
    stage_move(&mut client);

    server_move(&mut client, link);
    client.sync("archive", opts.clone()).unwrap();
    client.sync("inbox", opts).unwrap();

    assert_moved(&client, held);
}

/// The source pushes the move first, relocated by the connector, then
/// the target listing lands the staged create.
fn push_first(link: &str, held: bool) {
    let mut client = moving_client(link, held);
    let opts = PimdirSyncOptions::default();
    stage_move(&mut client);

    // NOTE: the fake relocates only into a target not holding the
    // identity yet, so a held one is moved as the connector would.
    if held {
        server_move(&mut client, link);
    }
    client.sync("inbox", opts.clone()).unwrap();
    client.sync("archive", opts).unwrap();

    assert_moved(&client, held);
}

#[test]
fn a_move_listed_before_its_push_lands_the_create() {
    listing_first("msg-a", false);
}

#[test]
fn a_move_pushed_before_its_listing_lands_the_create() {
    push_first("msg-a", false);
}

#[test]
fn a_minted_move_listed_before_its_push_lands_the_create() {
    listing_first("msg-a", true);
}

#[test]
fn a_minted_move_pushed_before_its_listing_lands_the_create() {
    push_first("msg-a", true);
}

#[test]
fn an_alt_keyed_move_listed_before_its_push_lands_the_create() {
    listing_first(ALT, false);
}

#[test]
fn an_alt_keyed_move_pushed_before_its_listing_lands_the_create() {
    push_first(ALT, false);
}

#[test]
fn a_minted_alt_keyed_move_lands_the_create() {
    listing_first(ALT, true);
    push_first(ALT, true);
}
