//! An item conflict recorded before its incoming body was read, settled by
//! the upgrade under the collection's `prefer-incoming` (SYNC §6, §9): the
//! item takes the body it fetched, and with it the summary and sort key
//! that body derives (STORAGE §9.3, Annex A), not those of the body it held.

use std::convert::Infallible;

use io_pimdir::{
    change::PimdirChange,
    client::PimdirStore,
    collection::PimdirCollectionId,
    hub::{PimdirHubConflict, PimdirHubItem},
    mutate::PimdirMutation,
    object::PimdirObject,
    placement::{PimdirHandle, PimdirLinkId},
    remote::{
        PimdirEnumerate, PimdirEnumerated, PimdirFetchedItem, PimdirPushResult, PimdirRemote,
        PimdirRemoteMeta, PimdirTier,
    },
    summary::{self, PimdirDerivation, PimdirSummary},
    sync::PimdirSyncOptions,
};

use crate::common::{Client, MemRemote, hash};

const KIND: &str = "text/vcard";

/// A DAV address book whose members are described as Annex A derives
/// them from their bodies, a listing carrying the body only when
/// `carries`.
struct Cards {
    inner: MemRemote,
    carries: bool,
}

impl Cards {
    fn new(carries: bool) -> Self {
        let mut inner = MemRemote::default();
        inner.mutable = true;
        Self { inner, carries }
    }

    fn body(&self, collection: &PimdirCollectionId, handle: &PimdirHandle) -> Vec<u8> {
        self.inner.items[collection][handle].body.clone()
    }
}

impl PimdirRemote for Cards {
    type Error = Infallible;

    fn enumerate(
        &mut self,
        collection: &PimdirCollectionId,
        request: PimdirEnumerate,
    ) -> Result<PimdirEnumerated, Infallible> {
        let PimdirEnumerated::Page(mut page) = self.inner.enumerate(collection, request)? else {
            unreachable!()
        };
        for item in &mut page.items {
            let body = self.body(collection, &item.handle);
            let meta = PimdirRemoteMeta::new(derived(&body));
            item.meta = match self.carries {
                true => meta.with_body(item.meta.body.take().unwrap()),
                false => meta,
            };
        }
        Ok(PimdirEnumerated::Page(page))
    }

    fn fetch(
        &mut self,
        collection: &PimdirCollectionId,
        handles: Vec<PimdirHandle>,
        tier: PimdirTier,
    ) -> Result<Vec<PimdirFetchedItem>, Infallible> {
        let mut fetched = self.inner.fetch(collection, handles, tier)?;
        for item in &mut fetched {
            let derivation = derived(&self.body(collection, &item.handle));
            item.summary = derivation.summary;
            item.sort_key = derivation.sort_key;
        }
        Ok(fetched)
    }

    fn push(
        &mut self,
        collection: &PimdirCollectionId,
        changes: Vec<PimdirChange>,
    ) -> Result<Vec<PimdirPushResult>, Infallible> {
        self.inner.push(collection, changes)
    }
}

fn card(name: &str) -> Vec<u8> {
    format!(
        "BEGIN:VCARD\r\nVERSION:4.0\r\nUID:card-1\r\nFN:{name}\r\n\
         EMAIL:ines.simon@example.com\r\nEND:VCARD\r\n"
    )
    .into_bytes()
}

fn derived(body: &[u8]) -> PimdirDerivation {
    summary::derive(KIND, body).expect("a vCard derives")
}

/// Source a holds the card as "Ines Simon", b joins it unread as "Their
/// Copy" under the collection's `conflict`, then b's upgrade reads it.
fn upgraded(conflict: PimdirHubConflict) -> PimdirHubItem {
    let mut a = Client::with_source(Cards::new(true), "a");
    a.store.ensure_collection("cards", KIND).unwrap();
    PimdirStore::open(a.path())
        .unwrap()
        .set_collection_conflict("cards", conflict)
        .unwrap();
    a.remote_mut()
        .inner
        .seed("cards", "r1", "card-1", &[], &card("Ines Simon"));
    a.sync("cards", PimdirSyncOptions::default()).unwrap();

    let theirs = card("Their Copy");
    let mut b = a.sharing(Cards::new(false), "b");
    b.remote_mut()
        .inner
        .seed("cards", "r1", "card-1", &[], &theirs);
    b.sync("cards", PimdirSyncOptions::default()).unwrap();
    assert!(
        b.hub("cards").items[&PimdirLinkId::from("card-1")].conflicted,
        "an unread join is the item's conflict",
    );

    b.upgrade("cards", vec![PimdirHandle::from("r1")], PimdirTier::Full)
        .unwrap();

    b.hub("cards").items[&PimdirLinkId::from("card-1")].clone()
}

#[test]
fn an_upgrade_adopting_the_incoming_body_restates_its_summary() {
    let item = upgraded(PimdirHubConflict::PreferIncoming);

    let theirs = card("Their Copy");
    let want = derived(&theirs);
    assert!(!item.conflicted, "the policy settles it");
    assert_eq!(item.object, Some(hash(&theirs)), "the incoming body wins");
    assert_eq!(item.sort_key, want.sort_key, "its own sort key");
    assert_eq!(
        item.summary,
        want.summary.map(PimdirSummary::stored),
        "its own summary",
    );
}

#[test]
fn an_upgrade_keeping_the_items_body_keeps_its_summary() {
    for conflict in [PimdirHubConflict::Manual, PimdirHubConflict::PreferExisting] {
        let item = upgraded(conflict);

        let ours = card("Ines Simon");
        let want = derived(&ours);
        assert_eq!(item.object, Some(hash(&ours)), "{conflict:?} keeps it");
        assert_eq!(item.sort_key, want.sort_key, "{conflict:?}");
        assert_eq!(
            item.summary,
            want.summary.map(PimdirSummary::stored),
            "{conflict:?}",
        );
    }
}

/// A card pulled unread, edited offline, then upgraded (SYNC §6): the
/// fetched body lands as the base and the local edit stays the item's
/// body, so the item keeps the summary and sort key the edit states, not
/// the fetch's, which describe the base.
#[test]
fn an_upgrade_reading_the_base_under_an_edit_keeps_its_summary() {
    let mut client = Client::new(Cards::new(false));
    client.store.ensure_collection("cards", KIND).unwrap();
    client
        .remote_mut()
        .inner
        .seed("cards", "r1", "card-1", &[], &card("Ines Simon"));
    client.sync("cards", PimdirSyncOptions::default()).unwrap();

    let edit = card("Our Edit");
    let want = derived(&edit);
    client
        .mutate(
            "cards",
            PimdirMutation::Edit {
                handle: PimdirHandle::from("r1"),
                object: PimdirObject {
                    hash: hash(&edit),
                    size: edit.len(),
                },
                body: edit.clone(),
                summary: want.summary.clone(),
                sort_key: Some(want.sort_key.clone()),
                if_match: None,
                overrule: false,
                resolves: None,
            },
        )
        .unwrap();
    client
        .upgrade("cards", vec![PimdirHandle::from("r1")], PimdirTier::Full)
        .unwrap();

    let placement = client.storage().placement("cards", "r1");
    let base = placement.base.as_ref().expect("a base");
    assert_eq!(
        base.object,
        Some(hash(&card("Ines Simon"))),
        "the base read"
    );
    assert_eq!(placement.object, Some(hash(&edit)), "the edit stays");
    assert_eq!(placement.sort_key, want.sort_key);
    assert_eq!(placement.summary, want.summary.map(PimdirSummary::stored));
}
