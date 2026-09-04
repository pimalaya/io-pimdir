//! # Conflict retention
//!
//! A content conflict crossed with retention (STORAGE §11).
//!
//! The store answers a `DropPlacement` by retaining the item and hiding
//! it from the seam. A conflicted placement holds three bodies, so the
//! tests pin whether the drop is reached, what the retained item keeps,
//! and whether a later sync settles it.

use io_pimdir::{
    client::reader::PimdirItem,
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
const RETURNED: &[u8] = b"the body it came back with";

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
    client
        .upgrade("inbox", vec![PimdirHandle::from("m1")], PimdirTier::Full)
        .unwrap();

    let placement = client.storage().rows("inbox");
    assert_eq!(placement.len(), 1);
    assert_eq!(placement[0].status, PimdirStatus::Conflict);
    assert_eq!(placement[0].conflict_revision.as_deref(), Some("1"));
    assert_eq!(placement[0].conflict_object, Some(hash(REMOTE)));

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

/// What the server holds for the inbox, as handle and body.
fn server(client: &Client) -> Vec<(String, Vec<u8>)> {
    client
        .remote()
        .items
        .get(&"inbox".into())
        .into_iter()
        .flatten()
        .map(|(handle, item)| (handle.as_str().to_string(), item.body.clone()))
        .collect()
}

/// The one item retention holds for the inbox.
fn retained(client: &Client) -> PimdirItem {
    let retained = client.storage().retained("inbox");
    assert_eq!(retained.len(), 1, "one retained item: {retained:?}");
    retained.into_iter().next().unwrap()
}

/// The remote withdrawing its side of a conflict makes it moot.
///
/// Edit beats delete, so the item is resurrected as a pending create
/// rather than dropped, and retention never sees it.
#[test]
fn a_conflicted_item_the_remote_deletes_never_reaches_the_retained_row_path() {
    let mut client = conflicted_client();
    client.remote_mut().remove("inbox", "m1");

    let report = client.sync("inbox", PimdirSyncOptions::default()).unwrap();

    assert_eq!(report.pushed, 1, "the local body is re-uploaded");
    assert_eq!(report.conflicts, 0, "the divergence is over, not re-run");
    assert_eq!(
        server(&client),
        vec![(
            "app-1".to_string(),
            hash(LOCAL).as_str().as_bytes().to_vec()
        )],
        "the local side of the divergence survives on the server",
    );

    let live = client.storage().rows("inbox");
    assert_eq!(
        live.len(),
        1,
        "one live row, not a resurrection plus a copy"
    );
    assert_eq!(live[0].status, PimdirStatus::Clean);
    assert_eq!(live[0].object, Some(hash(LOCAL)));
    assert_eq!(
        live[0].conflict_revision, None,
        "the conflict pair goes with the remote body it described",
    );
    assert_eq!(live[0].conflict_object, None);
    assert!(
        client.storage().retained("inbox").is_empty(),
        "retention never saw it",
    );
}

/// The one way a conflicted row reaches the drop: both sides deleted it.
///
/// The item is retained with the local side of the divergence, and no
/// later sync settles or revives it.
#[test]
fn a_retained_conflict_keeps_the_local_body_and_is_never_settled_by_a_later_sync() {
    let mut client = conflicted_client();
    client
        .mutate("inbox", PimdirMutation::Remove(PimdirHandle::from("m1")))
        .unwrap();
    client.remote_mut().remove("inbox", "m1");

    client.sync("inbox", PimdirSyncOptions::default()).unwrap();

    assert!(
        client.storage().rows("inbox").is_empty(),
        "hidden from the seam"
    );
    let kept = retained(&client);
    assert_eq!(kept.link_id, PimdirLinkId::from("l1"));
    assert_eq!(kept.object, Some(hash(LOCAL)), "the local side");
    assert!(kept.retention.is_some());

    let delta = client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    assert_eq!(delta, PimdirSyncReport::default(), "quiescent delta sync");
    let full = client
        .sync(
            "inbox",
            PimdirSyncOptions {
                full: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(full, PimdirSyncReport::default(), "quiescent full sync");
    assert_eq!(retained(&client), kept, "the retained item is untouched");
}

/// The same item returning upstream revives the retained one.
///
/// The arrival under a fresh handle carries the identity retention holds,
/// so the item comes back live with what the remote holds, its retained
/// body superseded rather than kept beside it.
#[test]
fn an_item_coming_back_revives_the_retained_one() {
    let mut client = conflicted_client();
    client
        .mutate("inbox", PimdirMutation::Remove(PimdirHandle::from("m1")))
        .unwrap();
    client.remote_mut().remove("inbox", "m1");
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();

    client.remote_mut().seed("inbox", "m2", "l1", &[], RETURNED);
    let report = client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    client
        .upgrade("inbox", vec![PimdirHandle::from("m2")], PimdirTier::Full)
        .unwrap();

    assert_eq!(report.pulled, 1, "a plain arrival");
    assert_eq!(report.conflicts, 0);
    assert_eq!(report.pushed, 0, "the retained item pushes nothing");

    let live = client.storage().rows("inbox");
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].handle, PimdirHandle::from("m2"));
    assert_eq!(live[0].status, PimdirStatus::Clean);
    assert_eq!(
        live[0].object,
        Some(hash(RETURNED)),
        "the arrival carries what the remote holds, not the retained body",
    );
    assert_eq!(live[0].conflict_revision, None);
    assert!(
        client.storage().retained("inbox").is_empty(),
        "the item is live again, so nothing is retained",
    );
}
