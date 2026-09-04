//! # Conflict across a rekey
//!
//! A content conflict crossed with a handle-space rebuild.
//!
//! An IMAP UIDVALIDITY bump renumbers every handle, and rekey carries the
//! local state over by link id. A conflicted placement has the most to
//! lose: local body, observed remote revision and remote body at it, none
//! of which the new handle space carries.
//!
//! Both orders a consumer can hit run here, the rekey before and after
//! the resolution, and the conflict is neither resolved, dropped nor
//! duplicated by the renumbering.

use std::collections::BTreeMap;

use io_pimdir::{
    collection::PimdirCheckpoint,
    mutate::PimdirMutation,
    object::PimdirObject,
    placement::{PimdirHandle, PimdirLinkId, PimdirStatus},
    remote::PimdirTier,
    sync::{PimdirSyncOptions, PimdirSyncReport},
};

use crate::common::{Client, MemRemote, hash};

const BASE: &[u8] = b"the ancestor body";
const LOCAL: &[u8] = b"the local body";
const REMOTE: &[u8] = b"the remote body";
const CUSTOM: &[u8] = b"a hand-merged body";
const LATER: &[u8] = b"a later remote body";

/// A client whose inbox holds a conflicted `m1` and a clean bystander `m2`.
///
/// The base holds `BASE`, the placement `LOCAL`, and the remote `REMOTE`
/// at the recorded conflict revision, the diverging body in the store.
fn conflicted_client() -> Client {
    let mut remote = MemRemote::default();
    remote.mutable = true;
    remote.seed("inbox", "m1", "l1", &[], BASE);
    remote.seed("inbox", "m2", "l2", &["seen"], b"the bystander body");

    let mut client = Client::new(remote);
    let opts = PimdirSyncOptions::default();
    client.sync("inbox", opts).unwrap();
    client
        .upgrade(
            "inbox",
            vec![PimdirHandle::from("m1"), PimdirHandle::from("m2")],
            PimdirTier::Full,
        )
        .unwrap();

    edit(&mut client, "m1", LOCAL);
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

fn edit(client: &mut Client, handle: &str, body: &[u8]) {
    let object = PimdirObject {
        hash: hash(body),
        size: body.len(),
    };
    client
        .mutate(
            "inbox",
            PimdirMutation::Edit {
                handle: PimdirHandle::from(handle),
                object,
                body: body.to_vec(),
                summary: None,
                sort_key: None,
            },
        )
        .unwrap();
}

/// Renumbers the inbox onto a second generation, returning `m1`'s new handle.
fn renumber(client: &mut Client) -> PimdirHandle {
    let mapping: BTreeMap<PimdirHandle, PimdirHandle> = client.remote_mut().renumber("inbox", 2);
    mapping
        .get(&PimdirHandle::from("m1"))
        .expect("the conflicted member is renumbered")
        .clone()
}

fn checkpoint(client: &Client) -> Option<PimdirCheckpoint> {
    client.storage().checkpoint("inbox")
}

/// The handles the replica holds for the inbox.
fn handles(client: &Client) -> Vec<String> {
    client
        .storage()
        .rows("inbox")
        .iter()
        .map(|p| p.handle.as_str().to_string())
        .collect()
}

/// The body the server holds under `handle`.
///
/// A pushed body reaches it as its object hash, an uploaded one as bytes.
fn server_body(client: &Client, handle: &PimdirHandle) -> Vec<u8> {
    client
        .remote()
        .items
        .get(&"inbox".into())
        .and_then(|c| c.get(handle))
        .map(|item| item.body.clone())
        .expect("the server holds the member")
}

/// A renumbering carries a conflict whole and resolves nothing.
///
/// The local body, the observed revision and the remote body at it all
/// live in the old handle's row; a rekey keeping the row but not the pair
/// would leave a conflict nobody can resolve.
#[test]
fn a_rekey_carries_a_conflict_whole_onto_the_new_handle() {
    let mut client = conflicted_client();
    let before = checkpoint(&client);
    let generation = client.store.generation("inbox").unwrap();
    let new = renumber(&mut client);

    let report = client.rekey("inbox").unwrap();

    assert_eq!(report.rekeyed, 2, "both members carried over");
    assert_eq!(report.pulled, 0, "nothing read as a new arrival");
    assert_eq!(report.dropped, 0, "no pending state lost");

    let placement = client.storage().placement("inbox", new.as_str());
    assert_eq!(placement.status, PimdirStatus::Conflict);
    assert_eq!(placement.link_id, Some(PimdirLinkId::from("l1")));
    assert_eq!(placement.object, Some(hash(LOCAL)), "the local side");
    assert_eq!(placement.conflict_revision.as_deref(), Some("1"));
    assert_eq!(
        placement.conflict_object,
        Some(hash(REMOTE)),
        "the diverging body survives the renumbering",
    );
    assert_eq!(
        placement.base.as_ref().and_then(|b| b.object.clone()),
        Some(hash(BASE)),
        "and so does the ancestor the merge reconciles against",
    );

    let mut handles = handles(&client);
    handles.sort();
    assert_eq!(handles, vec!["v2-0".to_string(), "v2-1".to_string()]);
    assert_ne!(checkpoint(&client), before, "the checkpoint advanced");
    assert_eq!(
        client.store.generation("inbox").unwrap(),
        generation.map(|g| g + 1),
        "the store bumped the collection's generation",
    );

    let report = client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    assert_eq!(report, PimdirSyncReport::default(), "a settled spine");
    let placement = client.storage().placement("inbox", new.as_str());
    assert_eq!(placement.status, PimdirStatus::Conflict);
    assert_eq!(placement.conflict_object, Some(hash(REMOTE)));
}

/// Rekey then resolve: the decision reaches the server under the new handle.
#[test]
fn a_conflict_carried_by_a_rekey_still_resolves_to_the_remote() {
    let mut client = conflicted_client();
    let new = renumber(&mut client);
    client.rekey("inbox").unwrap();

    edit(&mut client, new.as_str(), CUSTOM);
    let report = client.sync("inbox", PimdirSyncOptions::default()).unwrap();

    assert_eq!(report.pushed, 1);
    assert_eq!(report.conflicts, 0, "the divergence is settled, not re-run");
    assert_eq!(
        server_body(&client, &new),
        hash(CUSTOM).as_str().as_bytes(),
        "the resolution reached the remote",
    );
    assert_eq!(
        client.storage().placement("inbox", new.as_str()).status,
        PimdirStatus::Clean,
    );
}

/// Resolve then rekey: the decision survives as the pending push it is.
///
/// It keeps the remote state it settled as its base, and is neither
/// undone nor conflicted anew.
#[test]
fn a_resolution_survives_a_rekey_and_still_reaches_the_remote() {
    let mut client = conflicted_client();
    edit(&mut client, "m1", CUSTOM);
    let new = renumber(&mut client);

    let report = client.rekey("inbox").unwrap();
    assert_eq!(report.rekeyed, 2);
    assert_eq!(
        report.dropped, 0,
        "the resolution is not pending state lost"
    );

    let placement = client.storage().placement("inbox", new.as_str());
    assert_eq!(placement.status, PimdirStatus::Dirty, "a pending push");
    assert_eq!(placement.object, Some(hash(CUSTOM)));
    assert_eq!(placement.conflict_revision, None, "nothing left to resolve");
    assert_eq!(placement.conflict_object, None);
    assert_eq!(
        placement.base.as_ref().and_then(|b| b.object.clone()),
        Some(hash(REMOTE)),
        "the base is still the state the resolution settled",
    );

    let report = client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    assert_eq!(report.pushed, 1);
    assert_eq!(report.conflicts, 0);
    assert_eq!(
        server_body(&client, &new),
        hash(CUSTOM).as_str().as_bytes(),
        "the resolution reached the remote across the renumbering",
    );
}

/// A rekey observing a newer revision keeps the conflict, drops the body.
///
/// The stored diverging body describes the revision recorded beside it,
/// so the upgrade pass re-asks for it: a resolver merging against bytes
/// the remote no longer holds would decide against the wrong version.
#[test]
fn a_rekey_over_a_newer_revision_keeps_the_conflict_and_re_asks_for_the_body() {
    let mut client = conflicted_client();
    client.remote_mut().edit("inbox", "m1", LATER);
    let new = renumber(&mut client);

    client.rekey("inbox").unwrap();

    let placement = client.storage().placement("inbox", new.as_str());
    assert_eq!(placement.status, PimdirStatus::Conflict);
    assert_eq!(placement.conflict_revision.as_deref(), Some("2"));
    assert_eq!(placement.conflict_object, None, "the stale body is dropped");
    assert_eq!(placement.object, Some(hash(LOCAL)), "the local side stays");

    client
        .upgrade("inbox", vec![new.clone()], PimdirTier::Full)
        .unwrap();
    let placement = client.storage().placement("inbox", new.as_str());
    assert_eq!(placement.status, PimdirStatus::Conflict);
    assert_eq!(placement.conflict_object, Some(hash(LATER)));
}
