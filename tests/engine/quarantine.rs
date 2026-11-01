//! # A body failing its name is fetched again
//!
//! A `Full` item whose body was found failing its name, and moved into
//! quarantine/ by the owner, holds no body: the next `Full` upgrade
//! fetches it again where the source holds it, and a dedup never links
//! another placement onto it (pimdir STORAGE §7, SYNC §6).

use std::fs;

use io_pimdir::{
    client::blobs::PimdirBlobs, placement::PimdirHandle, remote::PimdirTier,
    sync::PimdirSyncOptions,
};

use crate::common::{Client, MemRemote, hash};

const BODY: &[u8] = b"From: a\r\nMessage-ID: <msg-a>\r\n\r\nshared body\r\n";

/// Flips the first byte of the body under `BODY`'s name.
fn corrupt(client: &Client) {
    let path = PimdirBlobs::open(client.path()).path(&hash(BODY)).unwrap();
    let mut bytes = fs::read(&path).unwrap();
    bytes[0] ^= 1;
    fs::write(&path, &bytes).unwrap();
}

/// How many files quarantine/ holds.
fn quarantined(client: &Client) -> usize {
    fs::read_dir(client.path().join("quarantine"))
        .map(|entries| entries.count())
        .unwrap_or(0)
}

#[test]
fn a_full_item_whose_body_fails_its_name_is_fetched_again() {
    let mut remote = MemRemote::default();
    remote.seed("inbox", "i1", "msg-a", &[], BODY);
    let mut client = Client::new(remote);
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    let i1 = vec![PimdirHandle::from("i1")];

    let report = client
        .upgrade("inbox", i1.clone(), PimdirTier::Full)
        .unwrap();
    assert_eq!(report.fetched, 1);
    let report = client
        .upgrade("inbox", i1.clone(), PimdirTier::Full)
        .unwrap();
    assert_eq!(report.fetched, 0, "a sound body is not fetched again");

    corrupt(&client);
    let report = client.upgrade("inbox", i1, PimdirTier::Full).unwrap();
    assert_eq!(report.fetched, 1, "{report:?}");
    assert_eq!(quarantined(&client), 1, "the failing file kept aside");
    assert_eq!(client.storage().body(&hash(BODY)).as_deref(), Some(BODY));
}

#[test]
fn a_dedup_never_links_onto_a_body_failing_its_name() {
    let mut remote = MemRemote::default();
    remote.seed("inbox", "i1", "msg-a", &[], BODY);
    remote.seed("archive", "a1", "msg-a", &["seen"], BODY);
    let mut client = Client::new(remote);
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    client
        .sync("archive", PimdirSyncOptions::default())
        .unwrap();
    client
        .upgrade("inbox", vec![PimdirHandle::from("i1")], PimdirTier::Full)
        .unwrap();

    corrupt(&client);
    let report = client
        .upgrade("archive", vec![PimdirHandle::from("a1")], PimdirTier::Full)
        .unwrap();
    assert_eq!((report.deduped, report.fetched), (0, 1), "{report:?}");
    assert_eq!(quarantined(&client), 1);
    assert_eq!(client.storage().body(&hash(BODY)).as_deref(), Some(BODY));
}
