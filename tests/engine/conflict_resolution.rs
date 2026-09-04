//! # Conflict resolution
//!
//! What each way of resolving a content conflict sends to the remote.
//!
//! A `Manual` conflict holds three bodies: local, base, and remote at the
//! observed revision. A consumer resolves with an `Edit` carrying its
//! decision, made only when it reaches the remote, so every case asserts
//! what the server holds afterwards rather than what the placement says.

use io_pimdir::{
    mutate::PimdirMutation,
    object::PimdirObject,
    placement::{PimdirHandle, PimdirStatus},
    remote::PimdirTier,
    sync::PimdirSyncOptions,
};

use crate::common::{Client, MemRemote, hash};

const BASE: &[u8] = b"the ancestor body";
const LOCAL: &[u8] = b"the local body";
const REMOTE: &[u8] = b"the remote body";
const CUSTOM: &[u8] = b"a hand-merged body";

/// A client whose single inbox member is conflicted.
///
/// The base holds `BASE`, the placement `LOCAL`, and the remote `REMOTE`
/// at the recorded conflict revision, the diverging body in the store.
fn conflicted_client() -> Client {
    let mut remote = MemRemote::default();
    remote.mutable = true;
    remote.seed("inbox", "m1", "l1", &[], BASE);

    let mut client = Client::new(remote);
    let opts = PimdirSyncOptions::default();
    client.sync("inbox", opts).unwrap();
    client
        .upgrade("inbox", vec![PimdirHandle::from("m1")], PimdirTier::Full)
        .unwrap();

    edit(&mut client, LOCAL);
    client.remote_mut().edit("inbox", "m1", REMOTE);
    client.sync("inbox", opts).unwrap();

    // NOTE: the engine fetches nothing itself, so the upgrade supplies the
    // diverging body a resolver reads.
    client
        .upgrade("inbox", vec![PimdirHandle::from("m1")], PimdirTier::Full)
        .unwrap();

    let placement = client.storage().placement("inbox", "m1");
    assert_eq!(placement.status, PimdirStatus::Conflict);
    assert_eq!(placement.conflict_revision.as_deref(), Some("1"));
    assert_eq!(placement.conflict_object, Some(hash(REMOTE)));

    client
}

fn edit(client: &mut Client, body: &[u8]) {
    let object = PimdirObject {
        hash: hash(body),
        size: body.len(),
    };
    client
        .mutate(
            "inbox",
            PimdirMutation::Edit {
                handle: PimdirHandle::from("m1"),
                object,
                body: body.to_vec(),
                summary: None,
                sort_key: None,
            },
        )
        .unwrap();
}

/// The body the server holds for the inbox member.
///
/// A pushed body reaches it as its object hash, an uploaded one as bytes.
fn server_body(client: &Client) -> Vec<u8> {
    client
        .remote()
        .items
        .get(&"inbox".into())
        .and_then(|c| c.get(&PimdirHandle::from("m1")))
        .map(|item| item.body.clone())
        .expect("the server holds the member")
}

/// Resolves with `body` and syncs: server body, pushed count, status.
fn resolve_and_sync(body: &[u8]) -> (Vec<u8>, usize, PimdirStatus) {
    let mut client = conflicted_client();
    edit(&mut client, body);

    let report = client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    let placement = client.storage().placement("inbox", "m1");

    (server_body(&client), report.pushed, placement.status)
}

/// Resolving with the ancestor is a decision the remote has to hear.
///
/// The replica holding the ancestor while the server holds its own edit
/// is exactly the divergence the resolution settled.
#[test]
fn resolving_with_the_ancestor_body_pushes_it() {
    let (body, pushed, status) = resolve_and_sync(BASE);

    assert_eq!(body, hash(BASE).as_str().as_bytes(), "the ancestor pushed");
    assert_eq!(pushed, 1);
    assert_eq!(status, PimdirStatus::Clean);
}

#[test]
fn resolving_with_the_local_body_pushes_it() {
    let (body, pushed, status) = resolve_and_sync(LOCAL);

    assert_eq!(body, hash(LOCAL).as_str().as_bytes(), "the local body");
    assert_eq!(pushed, 1);
    assert_eq!(status, PimdirStatus::Clean);
}

#[test]
fn resolving_with_a_hand_merged_body_pushes_it() {
    let (body, pushed, status) = resolve_and_sync(CUSTOM);

    assert_eq!(body, hash(CUSTOM).as_str().as_bytes(), "the merged body");
    assert_eq!(pushed, 1);
    assert_eq!(status, PimdirStatus::Clean);
}

/// Adopting the remote wholesale derives no push and lands clean.
#[test]
fn resolving_with_the_remote_body_pushes_nothing_and_settles() {
    let (body, pushed, status) = resolve_and_sync(REMOTE);

    assert_eq!(body, REMOTE, "the remote body is untouched");
    assert_eq!(pushed, 0, "the remote already holds the decision");
    assert_eq!(status, PimdirStatus::Clean);
}

/// A remote that moved on since the decision is a fresh divergence.
///
/// The resolution is kept and conflicted anew rather than overwriting
/// an edit nobody has seen.
#[test]
fn a_resolution_is_measured_against_the_state_it_settled() {
    let mut client = conflicted_client();
    edit(&mut client, BASE);
    client.remote_mut().edit("inbox", "m1", b"a later body");

    let report = client.sync("inbox", PimdirSyncOptions::default()).unwrap();

    assert_eq!(report.pushed, 0, "an unseen remote edit is not overwritten");
    assert_eq!(report.conflicts, 1);
    assert_eq!(server_body(&client), b"a later body");
    let placement = client.storage().placement("inbox", "m1");
    assert_eq!(placement.status, PimdirStatus::Conflict);
    assert_eq!(
        placement.object,
        Some(hash(BASE)),
        "the resolution survives as the local side of the new divergence",
    );
}
