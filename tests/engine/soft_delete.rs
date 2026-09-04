//! # Retention
//!
//! A remote delete reaches the store as a `DropPlacement`, and the store
//! retains the item rather than deleting it (STORAGE §11): hidden from
//! the sync seam, so the copy survives every later sync, and readable
//! from the trash view for a restore.

use io_pimdir::{
    placement::{PimdirHandle, PimdirLinkId},
    remote::PimdirTier,
    sync::{PimdirSyncOptions, PimdirSyncReport},
};

use crate::common::{Client, MemRemote, hash};

#[test]
fn a_remote_expunge_retains_the_item_and_hides_it_from_the_seam() {
    let mut remote = MemRemote::default();
    remote.seed("inbox", "1", "m1", &[], b"body");
    let mut client = Client::new(remote);

    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    client
        .upgrade("inbox", vec![PimdirHandle::from("1")], PimdirTier::Full)
        .unwrap();
    assert_eq!(
        client.storage().rows("inbox").len(),
        1,
        "the item is pulled"
    );

    client.remote_mut().remove("inbox", "1");
    let report = client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    assert_eq!(report.pulled, 1, "the vanish is observed");

    assert!(
        client.storage().rows("inbox").is_empty(),
        "hidden from the seam"
    );
    let retained = client.storage().retained("inbox");
    assert_eq!(retained.len(), 1, "the copy is kept for restore");
    assert_eq!(retained[0].link_id, PimdirLinkId::from("m1"));
    assert_eq!(retained[0].object, Some(hash(b"body")), "body included");
    assert_eq!(
        retained[0].retention.as_ref().and_then(|r| r.by.as_deref()),
        Some("left"),
        "stamped with the source whose removal retired it",
    );

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
    assert_eq!(
        client.storage().retained("inbox").len(),
        1,
        "still retained after re-sync",
    );
}
