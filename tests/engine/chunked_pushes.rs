//! # Chunked pushes
//!
//! A run pushes and records in chunks, so a lost write costs one chunk
//! rather than every push the run derived.

use io_pimdir::{
    mutate::PimdirMutation,
    placement::{PimdirFlags, PimdirHandle, PimdirStatus},
    remote::PimdirTier,
    sync::{PimdirSync, PimdirSyncOptions},
};

use crate::common::{Client, MemRemote};

/// A partial second chunk: a run derives a full chunk plus this remainder.
const EXTRA: usize = 3;
const MEMBERS: usize = PimdirSync::PUSH_CHUNK + EXTRA;

fn handle(index: usize) -> String {
    format!("{index:03}")
}

/// `MEMBERS` named members, each carrying a local flag edit to push.
fn dirty_client() -> Client {
    let mut remote = MemRemote::default();
    for index in 0..MEMBERS {
        let handle = handle(index);
        remote.seed("inbox", &handle, &handle, &[], handle.as_bytes());
    }

    let mut client = Client::new(remote);
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    let handles = (0..MEMBERS)
        .map(|i| PimdirHandle::from(handle(i)))
        .collect();
    client.upgrade("inbox", handles, PimdirTier::Meta).unwrap();

    for index in 0..MEMBERS {
        client
            .mutate(
                "inbox",
                PimdirMutation::SetFlags {
                    handle: PimdirHandle::from(handle(index)),
                    flags: PimdirFlags::from_iter(["seen"]),
                },
            )
            .unwrap();
    }

    client.remote_mut().push_batches.clear();
    client
}

fn status(client: &Client, index: usize) -> PimdirStatus {
    client.storage().placement("inbox", &handle(index)).status
}

#[test]
fn a_run_pushes_and_records_one_chunk_at_a_time() {
    let mut client = dirty_client();
    let report = client.sync("inbox", PimdirSyncOptions::default()).unwrap();

    assert_eq!(report.pushed, MEMBERS);
    assert_eq!(
        client.remote().push_batches,
        [PimdirSync::PUSH_CHUNK, EXTRA],
        "the changes must go out in chunks",
    );
    for index in 0..MEMBERS {
        assert_eq!(status(&client, index), PimdirStatus::Clean);
        assert!(
            client
                .remote()
                .flags_of("inbox", &handle(index))
                .contains("seen")
        );
    }
}
