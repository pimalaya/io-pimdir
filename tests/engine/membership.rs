//! # Cross-collection membership
//!
//! A move must deliver exactly one copy.
//!
//! A move is staged as a copy into the target plus a remove from the
//! source (see `PimdirMutation::Move`), so two independent syncs derive
//! the halves. The create copies from its origin or uploads the body it
//! holds, and the remove relocates into the target's pending create, a
//! copy the target held before the move proving no delivery (SYNC §3).

use std::collections::BTreeSet;

use io_pimdir::{
    mutate::PimdirMutation,
    placement::{PimdirFlags, PimdirHandle, PimdirStatus},
    remote::PimdirTier,
    sync::PimdirSyncOptions,
};

use crate::common::{Client, MemRemote, hash};

/// One hydrated inbox member, the archive and the trash synced empty: a
/// copy or a move stages its create under a source syncing its target.
fn seeded_client() -> Client {
    let body = b"From: a\r\nMessage-ID: <msg-a>\r\n\r\nbody\r\n";

    let mut remote = MemRemote::default();
    remote.seed("inbox", "i1", "msg-a", &[], body);

    let mut client = Client::new(remote);
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    client
        .sync("archive", PimdirSyncOptions::default())
        .unwrap();
    client.sync("trash", PimdirSyncOptions::default()).unwrap();
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

/// One inbox member keyed `link`, hydrated when `hydrated`, the archive
/// synced too, holding a member of the same identity when `held`.
fn moving_client(link: &str, held: bool, hydrated: bool) -> Client {
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
    if hydrated {
        client
            .upgrade("inbox", vec![PimdirHandle::from("i1")], PimdirTier::Full)
            .unwrap();
    }
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
fn assert_moved(client: &Client, held: bool, delivered: &str) {
    let mut expected = vec![delivered];
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
        client.storage().placement("archive", delivered).status,
        PimdirStatus::Clean,
    );
    assert!(
        local_members(client, "inbox").is_empty(),
        "tombstone dropped"
    );
}

/// The target listing lands the staged create before the source syncs.
fn listing_first(link: &str, held: bool) {
    let mut client = moving_client(link, held, true);
    let opts = PimdirSyncOptions::default();
    stage_move(&mut client);

    server_move(&mut client, link);
    client.sync("archive", opts.clone()).unwrap();
    client.sync("inbox", opts).unwrap();

    assert_moved(&client, held, "i1-moved");
}

/// The source pushes the move first, relocated by the connector, then
/// the target listing lands the staged create.
fn push_first(link: &str, held: bool, hydrated: bool) {
    let mut client = moving_client(link, held, hydrated);
    let opts = PimdirSyncOptions::default();
    stage_move(&mut client);

    client.sync("inbox", opts.clone()).unwrap();
    client.sync("archive", opts).unwrap();

    assert_moved(&client, held, "i1-moved");
}

/// The target pushes the create first, a copy from its origin, then the
/// source's remove is a plain delete.
fn copy_first(link: &str, held: bool, hydrated: bool) {
    let mut client = moving_client(link, held, hydrated);
    let opts = PimdirSyncOptions::default();
    stage_move(&mut client);

    client.sync("archive", opts.clone()).unwrap();
    client.sync("inbox", opts).unwrap();

    assert_moved(&client, held, "i1-copy");
}

#[test]
fn a_move_listed_before_its_push_lands_the_create() {
    listing_first("msg-a", false);
}

#[test]
fn a_move_pushed_before_its_listing_lands_the_create() {
    push_first("msg-a", false, true);
}

#[test]
fn a_minted_move_listed_before_its_push_lands_the_create() {
    listing_first("msg-a", true);
}

#[test]
fn a_minted_move_pushed_before_its_listing_lands_the_create() {
    push_first("msg-a", true, true);
}

#[test]
fn an_alt_keyed_move_listed_before_its_push_lands_the_create() {
    listing_first(ALT, false);
}

#[test]
fn an_alt_keyed_move_pushed_before_its_listing_lands_the_create() {
    push_first(ALT, false, true);
}

#[test]
fn a_minted_alt_keyed_move_lands_the_create() {
    listing_first(ALT, true);
    push_first(ALT, true, true);
}

/// A body-less source relocates beside a held copy rather than deleting.
#[test]
fn a_cold_minted_move_pushed_before_its_listing_relocates() {
    push_first("msg-a", false, false);
    push_first("msg-a", true, false);
}

/// A body-less source's create copies from its origin beside a held copy.
#[test]
fn a_cold_minted_move_synced_target_first_copies_from_its_origin() {
    copy_first("msg-a", false, false);
    copy_first("msg-a", true, false);
    copy_first("msg-a", true, true);
}

/// A flag set staged before a move rides into the target with it: the
/// server relocates the member with the flags it held, so a relocation
/// stating its handle lands the create on the flags it carried, and the
/// staged set is pushed, never pulled back over (SYNC §4, §5).
#[test]
fn a_flag_set_staged_before_a_stated_relocation_is_pushed() {
    let body = b"From: a\r\nMessage-ID: <msg-a>\r\n\r\nbody\r\n";
    let mut remote = MemRemote::default();
    remote.states_moved = true;
    remote.seed("inbox", "i1", "msg-a", &["\\Flagged"], body);
    let mut client = Client::new(remote);
    let opts = PimdirSyncOptions::default();
    client.sync("inbox", opts.clone()).unwrap();
    client.sync("archive", opts.clone()).unwrap();
    client
        .upgrade("inbox", vec![PimdirHandle::from("i1")], PimdirTier::Full)
        .unwrap();

    let staged = PimdirFlags::from_iter(["\\Seen", "$Important"]);
    client
        .mutate(
            "inbox",
            PimdirMutation::SetFlags {
                handle: PimdirHandle::from("i1"),
                flags: staged.clone(),
                base: None,
            },
        )
        .unwrap();
    stage_move(&mut client);

    client.sync("inbox", opts.clone()).unwrap();
    client.sync("archive", opts.clone()).unwrap();
    client.sync("archive", opts).unwrap();

    assert_eq!(remote_members(&client, "archive"), ["i1-moved"]);
    assert_eq!(
        client.remote().flags_of("archive", "i1-moved"),
        &staged,
        "the staged flags reach the server",
    );
    let landed = client.storage().placement("archive", "i1-moved");
    assert_eq!(landed.flags, staged, "the staged flags stay in the store");
    assert_eq!(landed.status, PimdirStatus::Clean);
}

/// Every copy of one message into a collection takes a key of its own,
/// however many it already holds: the key is minted again over each held
/// one, the third copy's included (STORAGE §9, SYNC §7).
#[test]
fn a_fourth_copy_into_a_collection_takes_a_key_of_its_own() {
    let mut client = seeded_client();
    let opts = PimdirSyncOptions::default();
    client.sync("archive", opts.clone()).unwrap();

    for _ in 0..4 {
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
    }

    assert_eq!(remote_members(&client, "archive").len(), 4);
    let rows = client.storage().rows("archive");
    let keys: BTreeSet<_> = rows.iter().filter_map(|p| p.link_id.clone()).collect();
    assert_eq!(keys.len(), 4, "four keys for four copies: {keys:?}");
    assert!(
        rows.iter().all(|p| p.status == PimdirStatus::Clean),
        "every copy landed",
    );
}

/// A copy into a collection whose member its source moved out, the move
/// not pushed yet, revives the tombstone under the handle it is bound to
/// rather than staging a create beside it (STORAGE §9, §10; SYNC §7):
/// the member stays where it was and the trash takes a copy of it.
#[test]
fn a_copy_over_a_member_moved_out_unpushed_revives_it() {
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
    let landed = client.storage().rows("archive")[0].handle.clone();
    client
        .mutate(
            "archive",
            PimdirMutation::Move {
                handle: landed.clone(),
                target: "trash".into(),
            },
        )
        .unwrap();

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
    client.sync("trash", opts.clone()).unwrap();

    assert_eq!(remote_members(&client, "archive"), [landed.as_str()]);
    assert_eq!(remote_members(&client, "trash").len(), 1);
    assert_eq!(remote_members(&client, "inbox").len(), 1);
    for collection in ["inbox", "archive", "trash"] {
        let rows = client.storage().rows(collection);
        assert_eq!(rows.len(), 1, "{collection}: {rows:?}");
        assert_eq!(rows[0].status, PimdirStatus::Clean, "{collection}");
    }
}

/// A client over `remote` whose inbox and archive are declared mail
/// collections, as an owner declares a mailbox before its first sync.
fn mail_client(remote: MemRemote) -> Client {
    let client = Client::new(remote);
    for collection in ["inbox", "archive"] {
        client
            .store
            .ensure_collection(collection, "message/rfc822")
            .unwrap();
    }
    client
}

/// Raises every bodiless member of `collection` to `Full`, as an owner
/// hydrating a collection does after a sync.
fn hydrate(client: &mut Client, collection: &str) {
    let handles: Vec<PimdirHandle> = client
        .storage()
        .rows(collection)
        .into_iter()
        .filter(|p| p.object.is_none() && !p.handle.is_provisional())
        .map(|p| p.handle)
        .collect();
    if !handles.is_empty() {
        client
            .upgrade(collection, handles, PimdirTier::Full)
            .unwrap();
    }
}

/// Every member of `collection` holding a body holds the one its server
/// holds under its handle, never another message's stating its
/// `Message-ID` (SYNC §6).
fn assert_own_bodies(client: &Client, collection: &str) {
    let rows = client.storage().rows(collection);
    let served = client.remote().items.get(&collection.into()).unwrap();
    for row in &rows {
        let server = served
            .get(&row.handle)
            .unwrap_or_else(|| panic!("{} on the server", row.handle.as_str()));
        assert_eq!(
            row.object,
            Some(hash(&server.body)),
            "{collection}/{} holds the body of the member it is bound to: {rows:?}",
            row.handle.as_str(),
        );
    }
}

/// A move into a collection holding another message stating its
/// `Message-ID`, listed and linked by agreement before the move, leaves
/// each member holding its own body: the move's create taking the body
/// the other one linked releases it from the other one, whose next
/// upgrade fetches it rather than linking it again (SYNC §6).
#[test]
fn a_move_beside_a_message_of_its_id_linked_before_keeps_both_bodies() {
    let moved = b"From: a\r\nMessage-ID: <msg-a>\r\n\r\nMOVED\r\n";
    let other = b"From: a\r\nMessage-ID: <msg-a>\r\n\r\nOTHER\r\n";
    let mut remote = MemRemote::default();
    remote.states_moved = true;
    remote.seed("inbox", "i1", "msg-a", &[], moved);
    let mut client = mail_client(remote);
    let opts = PimdirSyncOptions::default();
    client.sync("inbox", opts.clone()).unwrap();
    hydrate(&mut client, "inbox");
    client
        .remote_mut()
        .seed("archive", "a1", "msg-a", &[], other);
    client.sync("archive", opts.clone()).unwrap();
    hydrate(&mut client, "archive");

    stage_move(&mut client);
    client.sync("inbox", opts.clone()).unwrap();
    client.sync("archive", opts.clone()).unwrap();
    client.sync("archive", opts).unwrap();
    hydrate(&mut client, "archive");

    assert_eq!(remote_members(&client, "archive"), ["a1", "i1-moved"]);
    assert_eq!(client.storage().rows("archive").len(), 2);
    assert_own_bodies(&client, "archive");
}

/// A copy beside such a message keeps both bodies too, though the copied
/// message stays where it was, agreeing with the other one: the other
/// one, released, is fetched, never linked from it again (SYNC §6).
#[test]
fn a_copy_beside_a_message_of_its_id_linked_before_keeps_both_bodies() {
    let copied = b"From: a\r\nMessage-ID: <msg-a>\r\n\r\nCOPIED\r\n";
    let other = b"From: a\r\nMessage-ID: <msg-a>\r\n\r\nOTHER\r\n";
    let mut remote = MemRemote::default();
    remote.seed("inbox", "i1", "msg-a", &[], copied);
    let mut client = mail_client(remote);
    let opts = PimdirSyncOptions::default();
    client.sync("inbox", opts.clone()).unwrap();
    hydrate(&mut client, "inbox");
    client
        .remote_mut()
        .seed("archive", "a1", "msg-a", &[], other);
    client.sync("archive", opts.clone()).unwrap();
    hydrate(&mut client, "archive");

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
    client.sync("archive", opts).unwrap();
    hydrate(&mut client, "archive");

    assert_eq!(remote_members(&client, "archive").len(), 2);
    assert_eq!(client.storage().rows("archive").len(), 2);
    assert_own_bodies(&client, "archive");
    assert_own_bodies(&client, "inbox");
}

/// A listing bringing two messages stating one `Message-ID` links neither
/// body by agreement from a third another collection holds, even when the
/// member under the bare hint is upgraded alone: the store's load reads
/// the other one beside it (SYNC §6, STORAGE §14).
#[test]
fn a_listing_bringing_two_messages_of_one_id_links_neither() {
    let held = b"From: a\r\nMessage-ID: <msg-a>\r\n\r\nHELD\r\n";
    let other = b"From: a\r\nMessage-ID: <msg-a>\r\n\r\nOTHER\r\n";
    let mut remote = MemRemote::default();
    remote.seed("inbox", "i1", "msg-a", &[], held);
    let mut client = mail_client(remote);
    let opts = PimdirSyncOptions::default();
    client.sync("inbox", opts.clone()).unwrap();
    hydrate(&mut client, "inbox");
    client
        .remote_mut()
        .seed("archive", "a1", "msg-a", &[], other);
    client
        .remote_mut()
        .seed("archive", "a2", "msg-a", &[], held);
    client.sync("archive", opts).unwrap();

    client
        .upgrade("archive", vec![PimdirHandle::from("a1")], PimdirTier::Full)
        .unwrap();
    hydrate(&mut client, "archive");

    assert_own_bodies(&client, "archive");
}
