//! A rekey recognises every old copy of an identity by the key it carries
//! (SYNC §8): a copy minted over a handle a rekey has since replaced, a
//! copy staged beside the identity and delivered, a copy whose bare
//! sibling is gone. Each is carried with its body and pending state, and
//! no other source is told to remove it.

use io_pimdir::{
    mutate::PimdirMutation,
    placement::{PimdirFlags, PimdirHandle},
    remote::PimdirTier,
    sync::PimdirSyncOptions,
};

use crate::common::{Client, MemRemote};

const FIRST: &[u8] = b"From: a\r\nMessage-ID: <msg-a>\r\n\r\nthe meeting\r\n";
const SECOND: &[u8] = b"From: a\r\nMessage-ID: <msg-a>\r\n\r\nanother meeting\r\n";

/// The inbox rows: link id, whether a body is held, and the flags.
fn rows(client: &Client) -> Vec<(String, bool, PimdirFlags)> {
    let mut rows: Vec<_> = client
        .storage()
        .rows("inbox")
        .iter()
        .map(|p| {
            let key = p.link_id.as_ref().map_or("-", |l| l.as_str());
            (key.to_string(), p.object.is_some(), p.flags.clone())
        })
        .collect();
    rows.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
    rows
}

/// A mailbox holding one `Message-ID` twice, hydrated, then renumbered
/// twice, a flag staged on the minted copy between the two.
#[test]
fn a_minted_copy_survives_a_second_rekey() {
    let mut remote = MemRemote::default();
    remote.seed("inbox", "u1", "msg-a", &[], FIRST);
    remote.seed("inbox", "u2", "msg-a", &[], SECOND);
    let mut client = Client::new(remote);
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    let both = vec![PimdirHandle::from("u1"), PimdirHandle::from("u2")];
    client.upgrade("inbox", both, PimdirTier::Full).unwrap();

    client.remote_mut().renumber("inbox", 1);
    let first = client.rekey("inbox").unwrap();
    assert_eq!((first.rekeyed, first.pulled), (2, 0));

    client
        .mutate(
            "inbox",
            PimdirMutation::SetFlags {
                handle: PimdirHandle::from("v1-1"),
                flags: PimdirFlags::from_iter(["flagged"]),
                base: None,
            },
        )
        .unwrap();
    let before = rows(&client);

    client.remote_mut().renumber("inbox", 2);
    let second = client.rekey("inbox").unwrap();
    assert_eq!(
        (second.rekeyed, second.pulled, second.dropped),
        (2, 0, 0),
        "the second rekey carries both copies as the first did"
    );
    assert_eq!(rows(&client), before, "bodies and the staged flag kept");
    assert!(client.storage().retained("inbox").is_empty());
}

/// The same beside a second source mirroring the mailbox: a renumbering
/// on one side removes nothing from the other server.
#[test]
fn a_second_rekey_beside_another_source() {
    let mut a = Client::with_source(MemRemote::default(), "a");
    a.remote_mut().seed("inbox", "u1", "msg-a", &[], FIRST);
    a.remote_mut().seed("inbox", "u2", "msg-a", &[], SECOND);
    let mut b = a.sharing(MemRemote::default(), "b");

    let all = |client: &mut Client| {
        client.sync("inbox", PimdirSyncOptions::default()).unwrap();
        let handles: Vec<PimdirHandle> = client
            .open("inbox")
            .unwrap()
            .placements
            .iter()
            .map(|p| p.handle.clone())
            .collect();
        client.upgrade("inbox", handles, PimdirTier::Full).unwrap();
    };
    for _ in 0..3 {
        all(&mut a);
        all(&mut b);
    }
    assert_eq!(b.remote().handles("inbox").len(), 2);

    a.remote_mut().renumber("inbox", 1);
    a.rekey("inbox").unwrap();
    all(&mut a);
    all(&mut b);

    a.remote_mut().renumber("inbox", 2);
    a.rekey("inbox").unwrap();
    b.sync("inbox", PimdirSyncOptions::default()).unwrap();
    assert_eq!(
        b.remote().handles("inbox").len(),
        2,
        "a renumbering on a removed a member from b's server"
    );
}

/// A `Copy` staged beside the identity it copies, delivered, then one
/// rekey.
#[test]
fn a_delivered_copy_survives_a_rekey() {
    let mut remote = MemRemote::default();
    remote.seed("inbox", "u1", "msg-a", &[], FIRST);
    let mut client = Client::new(remote);
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    client
        .upgrade("inbox", vec![PimdirHandle::from("u1")], PimdirTier::Full)
        .unwrap();
    client
        .mutate(
            "inbox",
            PimdirMutation::Copy {
                handle: PimdirHandle::from("u1"),
                target: "inbox".into(),
            },
        )
        .unwrap();
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    let before = rows(&client);

    client.remote_mut().renumber("inbox", 1);
    let report = client.rekey("inbox").unwrap();
    assert_eq!((report.rekeyed, report.pulled), (2, 0));
    assert_eq!(rows(&client), before);
}

/// A minted copy whose bare sibling was deleted, then one rekey.
#[test]
fn a_lone_minted_copy_survives_a_rekey() {
    let mut remote = MemRemote::default();
    remote.seed("inbox", "u1", "msg-a", &[], FIRST);
    remote.seed("inbox", "u2", "msg-a", &[], SECOND);
    let mut client = Client::new(remote);
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    let both = vec![PimdirHandle::from("u1"), PimdirHandle::from("u2")];
    client.upgrade("inbox", both, PimdirTier::Full).unwrap();
    client.remote_mut().remove("inbox", "u1");
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    let before = rows(&client);

    client.remote_mut().renumber("inbox", 1);
    let report = client.rekey("inbox").unwrap();
    assert_eq!((report.rekeyed, report.pulled), (1, 0));
    assert_eq!(rows(&client), before);
}
