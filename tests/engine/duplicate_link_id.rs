//! # Duplicate link id
//!
//! An identity one collection holds twice is two items, never one.
//!
//! A placement is keyed by collection and link id, so the second copy
//! cannot take the key the first holds. Which copy gets the hint follows;
//! that the other goes without does not: a replica storing one of two
//! resources loses data where it noticed the problem.

use std::collections::BTreeSet;

use io_pimdir::{
    mutate::PimdirMutation,
    placement::{PimdirFlags, PimdirHandle, PimdirLinkId, PimdirStatus},
    remote::PimdirTier,
    sync::PimdirSyncOptions,
};

use crate::common::{Client, MemRemote};

/// Two resources a Posteo calendar held: one `UID`, two hrefs, two bodies.
const FIRST: &[u8] = b"From: a\r\nMessage-ID: <msg-a>\r\n\r\nthe meeting\r\n";
const SECOND: &[u8] = b"From: a\r\nMessage-ID: <msg-a>\r\n\r\nanother meeting\r\n";

/// The second copy's key: `dup:`, the hint, `#`, the handle it sits at.
const MINTED: &str = "dup:msg-a#u2";

/// A collection holding one identity twice: two handles, one hint.
fn twin_client() -> Client {
    let mut remote = MemRemote::default();
    remote.seed("inbox", "u1", "msg-a", &[], FIRST);
    remote.seed("inbox", "u2", "msg-a", &[], SECOND);

    Client::new(remote)
}

/// Syncs the collection and hydrates both copies, bodies included.
fn hydrate(client: &mut Client, handles: [&str; 2]) {
    client.sync("inbox", full()).unwrap();
    let handles = handles.iter().copied().map(PimdirHandle::from).collect();
    client.upgrade("inbox", handles, PimdirTier::Full).unwrap();
}

/// A run enumerating the whole collection, as under no `sync-collection`.
fn full() -> PimdirSyncOptions {
    PimdirSyncOptions {
        full: true,
        ..Default::default()
    }
}

fn link(client: &Client, handle: &str) -> Option<PimdirLinkId> {
    client.storage().placement("inbox", handle).link_id
}

#[test]
fn a_second_copy_is_minted_and_stored_with_its_own_body() {
    let mut client = twin_client();
    hydrate(&mut client, ["u1", "u2"]);

    let first = client.storage().placement("inbox", "u1");
    let second = client.storage().placement("inbox", "u2");

    assert_eq!(
        first.link_id,
        Some(PimdirLinkId::from("msg-a")),
        "the first copy keeps the identity it resolved",
    );
    assert_eq!(
        second.link_id,
        Some(PimdirLinkId::from(MINTED)),
        "and the second is minted from the hint and its own handle",
    );

    let object = second.object.clone().expect("the second copy has a body");
    assert_ne!(first.object, second.object, "two resources, two bodies");
    assert_eq!(
        client.storage().body(&object),
        Some(SECOND.to_vec()),
        "the body the user would otherwise never see is stored",
    );
    assert_eq!(second.status, PimdirStatus::Clean, "an ordinary item");
}

/// The mint may not depend on which copy a fetch batch returned first.
#[test]
fn the_mint_is_stable_across_a_fresh_store() {
    let mut first = twin_client();
    hydrate(&mut first, ["u1", "u2"]);

    let mut second = twin_client();
    hydrate(&mut second, ["u2", "u1"]);

    assert_eq!(link(&first, "u1"), link(&second, "u1"));
    assert_eq!(link(&first, "u2"), link(&second, "u2"));
    assert_eq!(link(&second, "u2"), Some(PimdirLinkId::from(MINTED)));
}

#[test]
fn the_second_copy_is_fetched_once_and_kept() {
    let mut client = twin_client();
    hydrate(&mut client, ["u1", "u2"]);
    let fetched = client.remote().full_fetches.len();

    for _ in 0..3 {
        client.sync("inbox", full()).unwrap();
    }

    assert_eq!(
        client.remote().full_fetches.len(),
        fetched,
        "a complete enumeration re-lists the twin and re-fetches nothing",
    );
    assert_eq!(link(&client, "u2"), Some(PimdirLinkId::from(MINTED)));
    assert_eq!(client.storage().objects(), 2, "no orphan bodies");
}

#[test]
fn a_vanish_removes_the_copy_that_went_and_no_other() {
    let mut client = twin_client();
    hydrate(&mut client, ["u1", "u2"]);

    client.remote_mut().remove("inbox", "u1");
    client.sync("inbox", full()).unwrap();

    assert!(
        !client.storage().contains("inbox", "u1"),
        "the copy the source dropped is gone",
    );
    assert_eq!(
        link(&client, "u2"),
        Some(PimdirLinkId::from(MINTED)),
        "and the survivor keeps the key it was minted under: re-canonicalising \
         it would change an identity a consumer has already shown",
    );
}

#[test]
fn each_copy_reconciles_on_its_own() {
    let mut client = twin_client();
    hydrate(&mut client, ["u1", "u2"]);

    client.remote_mut().set_flags("inbox", "u1", &["seen"]);
    client
        .mutate(
            "inbox",
            PimdirMutation::SetFlags {
                handle: PimdirHandle::from("u2"),
                flags: PimdirFlags::from_iter(["flagged"]),
            },
        )
        .unwrap();
    let report = client.sync("inbox", full()).unwrap();

    assert_eq!(report.pulled, 1);
    assert_eq!(report.pushed, 1);
    assert!(
        client
            .storage()
            .placement("inbox", "u1")
            .flags
            .contains("seen"),
        "the remote change reached the copy it names",
    );
    assert!(
        client.remote().flags_of("inbox", "u2").contains("flagged"),
        "and the staged edit reached the other, addressed as the one \
         item it is",
    );
    assert!(
        !client.remote().flags_of("inbox", "u1").contains("flagged"),
        "neither change crossed over to the other copy",
    );
}

/// A rebuild matching on the shared hint alone would lose one body.
#[test]
fn a_handle_space_change_keeps_both_copies() {
    let mut client = twin_client();
    hydrate(&mut client, ["u1", "u2"]);
    let bodies: Vec<Vec<u8>> = client
        .storage()
        .rows("inbox")
        .iter()
        .filter_map(|p| client.storage().body(p.object.as_ref()?))
        .collect();
    assert_eq!(bodies.len(), 2);

    client.remote_mut().renumber("inbox", 1);
    let report = client.rekey("inbox").unwrap();

    assert_eq!(report.rekeyed, 2, "both copies are carried over");
    assert_eq!(report.pulled, 0, "neither is read as a new member");
    let rows = client.storage().rows("inbox");
    assert_eq!(rows.len(), 2);

    let keys: BTreeSet<Option<PimdirLinkId>> = rows.iter().map(|p| p.link_id.clone()).collect();
    assert_eq!(keys.len(), 2, "two rows, two keys: {rows:#?}");
    assert!(
        keys.contains(&Some(PimdirLinkId::from(MINTED))),
        "the second copy keeps the key it was minted under: {keys:?}",
    );
    for placement in &rows {
        let object = placement.object.clone().expect("a carried body");
        let body = client.storage().body(&object);
        assert!(
            body.is_some_and(|body| bodies.contains(&body)),
            "each copy keeps the body it had: {placement:#?}",
        );
    }
}
