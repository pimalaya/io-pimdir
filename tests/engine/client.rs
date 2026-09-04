//! # Runner seams
//!
//! Error propagation from the remote seam and the coroutine into the
//! run error, as its display shapes.

use io_pimdir::{
    change::PimdirChange,
    collection::{PimdirCheckpoint, PimdirCollectionId},
    mutate::PimdirMutation,
    placement::PimdirHandle,
    remote::{PimdirFetchedItem, PimdirPushResult, PimdirRemote, PimdirRemoteSnapshot, PimdirTier},
    sync::PimdirSyncOptions,
};

use crate::common::{Client, MemRemote};

/// A remote whose every call fails.
struct BrokenRemote;

impl PimdirRemote for BrokenRemote {
    type Error = &'static str;

    fn enumerate(
        &mut self,
        _: &PimdirCollectionId,
        _: Option<PimdirCheckpoint>,
    ) -> Result<PimdirRemoteSnapshot, Self::Error> {
        Err("network unplugged")
    }

    fn fetch(
        &mut self,
        _: &PimdirCollectionId,
        _: Vec<PimdirHandle>,
        _: PimdirTier,
    ) -> Result<Vec<PimdirFetchedItem>, Self::Error> {
        Err("network unplugged")
    }

    fn push(
        &mut self,
        _: &PimdirCollectionId,
        _: Vec<PimdirChange>,
    ) -> Result<Vec<PimdirPushResult>, Self::Error> {
        Err("network unplugged")
    }
}

#[test]
fn remote_error_propagates() {
    let mut client = Client::new(BrokenRemote);

    let err = client
        .sync("inbox", PimdirSyncOptions::default())
        .unwrap_err();
    assert_eq!(err, "Pimdir remote failed: network unplugged");
}

#[test]
fn coroutine_error_propagates() {
    let mut client = Client::new(MemRemote::default());

    let err = client
        .mutate("inbox", PimdirMutation::Remove(PimdirHandle::from("nope")))
        .unwrap_err();
    assert_eq!(
        err,
        "Pimdir engine failed: Pimdir MUTATE failed: unknown handle nope",
    );
}
