//! A mail member naming a message another source already holds, joined
//! on whichever page of a round lists it or after both sources listed it
//! (SYNC §3, §6, §9): an immutable kind never owes a body, so its binding
//! derives no `Update`, and a flag set on one server reaches the other.

use std::convert::Infallible;

use io_pimdir::{
    change::{PimdirChange, PimdirChangeKind},
    collection::{PimdirCheckpoint, PimdirCollectionId, PimdirCursor},
    placement::{PimdirHandle, PimdirStatus},
    remote::{
        PimdirEnumerate, PimdirEnumerated, PimdirFetchedItem, PimdirListing, PimdirPushOutcome,
        PimdirPushResult, PimdirRemote, PimdirRemoteItem, PimdirRemoteSnapshot, PimdirTier,
    },
    sync::PimdirSyncOptions,
};

use crate::common::{Client, MemRemote};

/// A mail source: immutable bodies (no revision, an `Update` rejected, as
/// an IMAP connector answers one), a round listed `page` members
/// at a time in handle order, its checkpoint on the first page.
struct Mail {
    inner: MemRemote,
    page: usize,
    rest: Vec<PimdirRemoteItem>,
    pushed: Vec<PimdirChangeKind>,
}

impl Mail {
    fn new(page: usize) -> Self {
        Self {
            inner: MemRemote::default(),
            page,
            rest: Vec::new(),
            pushed: Vec::new(),
        }
    }

    fn next(&mut self, checkpoint: Option<PimdirCheckpoint>) -> PimdirEnumerated {
        let n = self.page.min(self.rest.len());
        let items: Vec<PimdirRemoteItem> = self.rest.drain(..n).collect();
        let cursor = (!self.rest.is_empty()).then(|| PimdirCursor(b"next".to_vec()));
        PimdirEnumerated::Page(PimdirRemoteSnapshot::page(items, cursor, checkpoint))
    }
}

impl PimdirRemote for Mail {
    type Error = Infallible;

    fn enumerate(
        &mut self,
        collection: &PimdirCollectionId,
        request: PimdirEnumerate,
    ) -> Result<PimdirEnumerated, Infallible> {
        match &request.listing {
            PimdirListing::Round { cursor: None, .. } => {
                let PimdirEnumerated::Page(full) = self.inner.enumerate(collection, request)?
                else {
                    unreachable!()
                };
                self.rest = full.items;
                Ok(self.next(full.checkpoint))
            }
            PimdirListing::Round {
                cursor: Some(_), ..
            } => Ok(self.next(None)),
            _ => self.inner.enumerate(collection, request),
        }
    }

    fn fetch(
        &mut self,
        collection: &PimdirCollectionId,
        handles: Vec<PimdirHandle>,
        tier: PimdirTier,
    ) -> Result<Vec<PimdirFetchedItem>, Infallible> {
        self.inner.fetch(collection, handles, tier)
    }

    fn push(
        &mut self,
        collection: &PimdirCollectionId,
        changes: Vec<PimdirChange>,
    ) -> Result<Vec<PimdirPushResult>, Infallible> {
        let mut results = Vec::new();
        for change in changes {
            self.pushed.push(change.kind.clone());
            if let PimdirChangeKind::Update { handle, .. } = &change.kind {
                results.push(PimdirPushResult {
                    handle: handle.clone(),
                    outcome: PimdirPushOutcome::Rejected,
                    assigned: None,
                    revision: None,
                    relocated: false,
                });
                continue;
            }
            results.extend(self.inner.push(collection, vec![change])?);
        }
        Ok(results)
    }
}

/// Source a holds M hydrated and seen; source b lists its own copy of M
/// under `shared` beside another member under `other`, one member a page.
fn scenario(shared: &str, other: &str) -> (Client<Mail>, Client<Mail>) {
    let mut a = Client::with_source(Mail::new(usize::MAX), "a");
    a.store
        .ensure_collection("inbox", "message/rfc822")
        .unwrap();
    a.remote_mut()
        .inner
        .seed("inbox", "a1", "msg-m", &["seen"], b"body m");
    a.sync("inbox", PimdirSyncOptions::default()).unwrap();
    a.upgrade("inbox", vec![PimdirHandle::from("a1")], PimdirTier::Full)
        .unwrap();

    let mut b = a.sharing(Mail::new(1), "b");
    b.remote_mut()
        .inner
        .seed("inbox", shared, "msg-m", &[], b"body m");
    b.remote_mut()
        .inner
        .seed("inbox", other, "msg-other", &[], b"body other");
    (a, b)
}

/// The `Update`s b pushed so far.
fn updates(b: &Client<Mail>) -> usize {
    b.remote()
        .pushed
        .iter()
        .filter(|kind| matches!(kind, PimdirChangeKind::Update { .. }))
        .count()
}

/// M on b's first page of two is joined at the write: its binding owes
/// no body, and a flag set later on a's server reaches b's.
#[test]
fn a_member_joined_on_a_page_before_the_last_owes_no_body() {
    let (mut a, mut b) = scenario("b1", "b2");
    for _ in 0..4 {
        b.sync("inbox", PimdirSyncOptions::default()).unwrap();
    }
    assert_eq!(updates(&b), 0, "an immutable member derived an Update");
    assert_eq!(
        b.storage().placement("inbox", "b1").status,
        PimdirStatus::Clean
    );

    a.remote_mut()
        .inner
        .set_flags("inbox", "a1", &["seen", "flagged"]);
    a.sync("inbox", PimdirSyncOptions::default()).unwrap();
    for _ in 0..2 {
        b.sync("inbox", PimdirSyncOptions::default()).unwrap();
    }
    assert_eq!(updates(&b), 0, "an immutable member derived an Update");
    assert!(
        b.remote().inner.flags_of("inbox", "b1").contains("flagged"),
        "the flag a set never reached b"
    );
}

/// M on b's last page is landed, the control.
#[test]
fn a_member_landed_on_the_last_page_owes_no_body() {
    let (_, mut b) = scenario("b9", "b2");
    for _ in 0..3 {
        b.sync("inbox", PimdirSyncOptions::default()).unwrap();
    }
    assert_eq!(updates(&b), 0);
    assert_eq!(
        b.storage().placement("inbox", "b9").status,
        PimdirStatus::Clean
    );
}

/// No paging needed: both sources list M at `Meta`, then both hydrate it.
#[test]
fn a_member_both_sources_listed_before_a_hydration_owes_no_body() {
    let mut a = Client::with_source(Mail::new(usize::MAX), "a");
    a.store
        .ensure_collection("inbox", "message/rfc822")
        .unwrap();
    a.remote_mut()
        .inner
        .seed("inbox", "a1", "msg-m", &[], b"body m");
    let mut b = a.sharing(Mail::new(usize::MAX), "b");
    b.remote_mut()
        .inner
        .seed("inbox", "b1", "msg-m", &[], b"body m");

    a.sync("inbox", PimdirSyncOptions::default()).unwrap();
    b.sync("inbox", PimdirSyncOptions::default()).unwrap();
    a.upgrade("inbox", vec![PimdirHandle::from("a1")], PimdirTier::Full)
        .unwrap();
    b.upgrade("inbox", vec![PimdirHandle::from("b1")], PimdirTier::Full)
        .unwrap();
    for _ in 0..2 {
        b.sync("inbox", PimdirSyncOptions::default()).unwrap();
    }
    assert_eq!(updates(&b), 0, "an immutable member derived an Update");
    assert_eq!(
        b.storage().placement("inbox", "b1").status,
        PimdirStatus::Clean
    );
}

/// A first pairing of two copies of M carrying different flags ends with
/// the same flags on both servers whatever page M sits on at b: the flag
/// sets each server answers once both settled.
fn first_pairing(shared: &str, a_flags: &[&str], b_flags: &[&str]) -> (String, String) {
    let mut a = Client::with_source(Mail::new(usize::MAX), "a");
    a.store
        .ensure_collection("inbox", "message/rfc822")
        .unwrap();
    a.remote_mut()
        .inner
        .seed("inbox", "a1", "msg-m", a_flags, b"body m");
    a.sync("inbox", PimdirSyncOptions::default()).unwrap();
    a.upgrade("inbox", vec![PimdirHandle::from("a1")], PimdirTier::Full)
        .unwrap();
    let mut b = a.sharing(Mail::new(1), "b");
    b.remote_mut()
        .inner
        .seed("inbox", shared, "msg-m", b_flags, b"body m");
    b.remote_mut()
        .inner
        .seed("inbox", "b5", "msg-other", &[], b"body other");
    for _ in 0..3 {
        b.sync("inbox", PimdirSyncOptions::default()).unwrap();
        a.sync("inbox", PimdirSyncOptions::default()).unwrap();
    }
    (
        format!("{:?}", a.remote().inner.flags_of("inbox", "a1")),
        format!("{:?}", b.remote().inner.flags_of("inbox", shared)),
    )
}

#[test]
fn a_first_pairing_ends_alike_whatever_the_page() {
    for (a_flags, b_flags) in [(&["seen"][..], &[][..]), (&[][..], &["seen"][..])] {
        let early = first_pairing("b1", a_flags, b_flags);
        let last = first_pairing("b9", a_flags, b_flags);
        assert_eq!(early, last, "a={a_flags:?} b={b_flags:?}");
        assert_eq!(early.0, early.1, "a={a_flags:?} b={b_flags:?}");
    }
}
