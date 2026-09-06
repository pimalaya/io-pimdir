//! # Upgrade coroutine
//!
//! Raises placements of one collection to a higher detail level: a pure
//! pull, never a merge (SYNC §6).
//!
//! At [`PimdirTier::Full`] the link ids are resolved against the object
//! store first, so a body already stored under another collection is
//! linked with no network round-trip: one body backs an item appearing
//! in several collections, which is what the unified view relies on.
//!
//! A fetch is also what reads an identity, so this is where it settles.
//! A collection holds one link id once, so a second copy of an identity
//! is linked under a key minted from it rather than left unlinked: a
//! source holding two resources holds two items, whatever its protocol.
//! A hint a pending create of the same source holds is that create
//! arriving, relocated or appended by another client, and lands it.

use core::mem;

use alloc::{collections::BTreeMap, vec::Vec};

use log::{debug, trace};

use crate::{
    change::{PimdirDropReason, PimdirWriteOp},
    collection::PimdirCollectionId,
    coroutine::*,
    load::PimdirLoadScope,
    object::PimdirObject,
    placement::{
        PimdirBase, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement, PimdirStatus,
    },
    remote::{PimdirFetchedBody, PimdirFetchedItem, PimdirTier},
    summary::PimdirSummary,
};

/// What an upgrade did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PimdirUpgradeReport {
    /// Placements raised to the requested level.
    pub upgraded: usize,
    /// Bodies fetched from the remote.
    pub fetched: usize,
    /// Bodies linked from the object store without a fetch.
    pub deduped: usize,
}

/// I/O-free UPGRADE coroutine.
pub struct PimdirUpgrade {
    collection: PimdirCollectionId,
    handles: Vec<PimdirHandle>,
    tier: PimdirTier,
    placements: BTreeMap<PimdirHandle, PimdirPlacement>,
    /// Fetched items held between the fetch and the identity check.
    fetched: Vec<PimdirFetchedItem>,
    ops: Vec<PimdirWriteOp>,
    report: PimdirUpgradeReport,
    state: State,
}

impl PimdirUpgrade {
    /// Creates a coroutine raising `handles` in `collection` to `tier`.
    pub fn new(
        collection: impl Into<PimdirCollectionId>,
        handles: Vec<PimdirHandle>,
        tier: PimdirTier,
    ) -> Self {
        let collection = collection.into();
        debug!(
            "upgrade {} handles in {} to {tier:?}",
            handles.len(),
            collection.as_str(),
        );

        Self {
            collection,
            handles,
            tier,
            placements: BTreeMap::new(),
            fetched: Vec::new(),
            ops: Vec::new(),
            report: PimdirUpgradeReport::default(),
            state: State::Start,
        }
    }

    /// Requested handles that still need work for the target tier.
    ///
    /// The level is a claim and the payload the fact: a row reading high
    /// enough while holding nothing is revisited. A conflicted placement
    /// needs the body the remote holds instead, not its own local one.
    fn pending_handles(&self) -> Vec<PimdirHandle> {
        self.handles
            .iter()
            .filter(|h| match self.placements.get(h) {
                Some(p) => match self.tier {
                    PimdirTier::Meta => p.level < PimdirLevel::Meta || p.summary.is_none(),
                    PimdirTier::Full => match is_conflicted(p) {
                        true => p.conflict_object.is_none(),
                        false => p.level < PimdirLevel::Full || p.object.is_none(),
                    },
                },
                None => false,
            })
            .cloned()
            .collect()
    }

    /// Applies the fetched items and yields the write batch.
    fn write_fetched(
        &mut self,
    ) -> PimdirCoroutineState<PimdirYield, <Self as PimdirCoroutine>::Return> {
        // NOTE: extended as the batch resolves, since both copies of a
        // duplicate commonly arrive unlinked in one batch.
        let mut claimed: BTreeMap<PimdirLinkId, PimdirHandle> = self
            .placements
            .values()
            .filter_map(|p| Some((p.link_id.clone()?, p.handle.clone())))
            .collect();

        // NOTE: claimed in handle order, so a store rebuilt from scratch
        // mints the same keys whatever order the fetch reported in.
        let mut fetched = mem::take(&mut self.fetched);
        fetched.sort_by(|a, b| a.handle.cmp(&b.handle));

        for item in fetched {
            let Some(placement) = self.placements.get(&item.handle) else {
                continue;
            };
            let mut patched = placement.clone();

            if self.tier == PimdirTier::Full && is_conflicted(&patched) {
                let Some(body) = item.body else {
                    continue;
                };
                let (object, bytes) = stored_body(body);
                let hash = object.hash.clone();

                self.ops.push(PimdirWriteOp::StoreObject {
                    object,
                    body: bytes,
                });
                // NOTE: a diverging body equal to the placement's own is no
                // divergence, the push whose record was lost having landed
                // (SYNC §5): the base adopts it and the conflict clears.
                if patched.object.as_ref() == Some(&hash) {
                    patched.status = PimdirStatus::Clean;
                    patched.conflict_revision = None;
                    patched.conflict_object = None;
                    patched.base = Some(PimdirBase {
                        flags: patched
                            .base
                            .as_ref()
                            .map(|base| base.flags.clone())
                            .unwrap_or_else(|| patched.flags.clone()),
                        revision: item.revision.clone(),
                        object: Some(hash),
                    });
                } else {
                    patched.conflict_object = Some(hash);
                }
                self.ops.push(PimdirWriteOp::UpsertPlacement(patched));
                self.report.fetched += 1;
                self.report.upgraded += 1;
                continue;
            }

            // NOTE: a mutable resource stating another hint under its handle
            // is a new identity there (SYNC §6): keyed afresh, the storage
            // retiring the old binding as a changed hash: key is. A minted
            // key never equals its hint and stays.
            let mutable = item.revision.is_some() || is_mutable(&patched);
            let restated = patched
                .link_id
                .as_ref()
                .is_some_and(|key| *key != item.link_id && !key.as_str().starts_with("dup:"));
            if mutable && restated {
                debug!(
                    "handle {} states a new identity, keying it afresh",
                    item.handle.as_str()
                );
                patched.link_id = None;
                patched.object = None;
                patched.base = None;
                patched.status = PimdirStatus::Clean;
                patched.conflict_revision = None;
                patched.conflict_object = None;
            }

            // NOTE: never re-identifies a linked item of an immutable kind:
            // tiers may disagree on the link, an ENVELOPE naming a
            // Message-ID the body parser misses.
            if patched.link_id.is_none() {
                if let Some(create) = self.take_pending_create(&item.link_id, &item.handle) {
                    claimed.insert(item.link_id.clone(), item.handle.clone());
                    self.land(create, &patched, item);
                    continue;
                }

                let link_id = item
                    .link_id
                    .claim(&item.handle, |key| claimed.contains_key(key));
                claimed.insert(link_id.clone(), item.handle.clone());
                patched.link_id = Some(link_id);
                // NOTE: a probe carries no base (SYNC §3); naming it is
                // what agrees with the source on what it reported.
                patched.base.get_or_insert_with(|| PimdirBase {
                    flags: patched.flags.clone(),
                    revision: item.revision.clone(),
                    object: None,
                });
            }
            patched.summary = item.summary;
            // NOTE: unlike the link id, the sort key is a projection of the
            // content, not an identity, so the latest derivation wins.
            patched.sort_key = item.sort_key;

            match (self.tier, item.body) {
                (PimdirTier::Full, Some(body)) => {
                    let (object, bytes) = stored_body(body);
                    let hash = object.hash.clone();
                    self.ops.push(PimdirWriteOp::StoreObject {
                        object,
                        body: bytes,
                    });

                    // NOTE: a fetch moves the base (SYNC §6). Over a local
                    // edit it is the base's body when the revision stayed,
                    // the local body kept, and the both-changed case when
                    // the revision moved: the fetched body lands as the
                    // diverging one and the placement conflicts.
                    let edited = patched.staged_edit().is_some();
                    let base_revision = patched.base.as_ref().and_then(|b| b.revision.clone());
                    if edited && item.revision.is_some() && item.revision != base_revision {
                        patched.status = PimdirStatus::Conflict;
                        patched.conflict_revision = item.revision.clone();
                        patched.conflict_object = Some(hash);
                    } else {
                        if let Some(base) = &mut patched.base {
                            base.revision = item.revision.clone();
                            base.object = Some(hash.clone());
                        }
                        if !edited {
                            patched.object = Some(hash);
                        }
                    }

                    patched.level = PimdirLevel::Full;
                    self.report.fetched += 1;
                }
                // NOTE: a row holding a body stays full whichever tier
                // answered last, else the next full upgrade refetches it.
                _ => {
                    patched.level = match patched.object {
                        Some(_) => PimdirLevel::Full,
                        None => PimdirLevel::Meta,
                    };
                    self.report.fetched += 1;
                }
            }

            self.ops.push(PimdirWriteOp::UpsertPlacement(patched));
            self.report.upgraded += 1;
        }

        debug!("write {} storage ops", self.ops.len());
        self.state = State::Writing;
        let ops = mem::take(&mut self.ops);
        PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops))
    }

    /// Takes the pending create of this source holding `hint`, if any.
    ///
    /// A `Created` placement with no base under a provisional handle is
    /// the create a fetched `hint` delivers (SYNC §6); taken out so a
    /// second arrival of the same hint in the batch is minted instead.
    fn take_pending_create(
        &mut self,
        hint: &PimdirLinkId,
        arrived: &PimdirHandle,
    ) -> Option<PimdirPlacement> {
        let provisional = self
            .placements
            .values()
            .find(|p| {
                p.link_id.as_ref() == Some(hint)
                    && p.status == PimdirStatus::Created
                    && p.base.is_none()
                    && &p.handle != arrived
            })
            .map(|p| p.handle.clone())?;

        self.placements.remove(&provisional)
    }

    /// Lands a pending create under the handle its arrival was fetched at.
    ///
    /// A `Superseded` drop of the provisional handle, then the create
    /// moved onto the fetched one with a base of what the fetch reported:
    /// the probe's flags, the revision, and the body at `Full` or else the
    /// create's own. The flags, body, summary and sort key staged on the
    /// create stay, so an edit made on it still pushes (SYNC §6).
    fn land(&mut self, create: PimdirPlacement, probe: &PimdirPlacement, item: PimdirFetchedItem) {
        debug!(
            "land the pending create {} under {}",
            create.handle.as_str(),
            item.handle.as_str(),
        );
        self.ops.push(PimdirWriteOp::DropPlacement {
            collection: self.collection.clone(),
            handle: create.handle.clone(),
            reason: PimdirDropReason::Superseded,
        });

        let mut landed = create;
        landed.handle = item.handle;
        let mut base_object = landed.object.clone();

        if let (PimdirTier::Full, Some(body)) = (self.tier, item.body) {
            let (object, bytes) = stored_body(body);
            let hash = object.hash.clone();
            self.ops.push(PimdirWriteOp::StoreObject {
                object,
                body: bytes,
            });
            base_object = Some(hash.clone());
            landed.object.get_or_insert(hash);
        }

        if landed.summary.is_none() {
            landed.summary = item.summary;
        }
        if landed.sort_key.is_unknown() {
            landed.sort_key = item.sort_key;
        }
        landed.level = match landed.object {
            Some(_) => PimdirLevel::Full,
            None => PimdirLevel::Meta,
        };
        landed.base = Some(PimdirBase {
            flags: probe.flags.clone(),
            revision: item.revision,
            object: base_object,
        });
        landed.status = match landed.base.as_ref() {
            Some(base) if base.flags == landed.flags && base.object == landed.object => {
                PimdirStatus::Clean
            }
            _ => PimdirStatus::Dirty,
        };
        landed.origin = None;

        self.ops.push(PimdirWriteOp::UpsertPlacement(landed));
        self.report.fetched += 1;
        self.report.upgraded += 1;
    }
}

impl PimdirCoroutine for PimdirUpgrade {
    type Yield = PimdirYield;
    type Return = Result<PimdirUpgradeReport, PimdirArgError>;

    fn resume(
        &mut self,
        arg: Option<PimdirArg>,
    ) -> PimdirCoroutineState<Self::Yield, Self::Return> {
        match (&self.state, arg) {
            (State::Start, None) => {
                debug!("load target items from storage");
                self.state = State::Loading;
                PimdirCoroutineState::Yielded(PimdirYield::WantsLoad {
                    collection: self.collection.clone(),
                    scope: PimdirLoadScope::Handles(self.handles.clone()),
                })
            }

            (State::Loading, Some(PimdirArg::Load(loaded))) => {
                self.placements = loaded
                    .placements
                    .into_iter()
                    .map(|p| (p.handle.clone(), p))
                    .collect();

                let pending = self.pending_handles();
                if pending.is_empty() {
                    debug!("nothing to upgrade");
                    self.state = State::Done;
                    return PimdirCoroutineState::Complete(Ok(self.report));
                }

                match self.tier {
                    PimdirTier::Meta => {
                        debug!("fetch {} items at meta tier", pending.len());
                        self.state = State::Fetching;
                        PimdirCoroutineState::Yielded(PimdirYield::WantsFetch {
                            collection: self.collection.clone(),
                            handles: pending,
                            tier: PimdirTier::Meta,
                        })
                    }
                    PimdirTier::Full => {
                        let links: Vec<_> = pending
                            .iter()
                            .filter_map(|h| self.placements.get(h))
                            .filter(|p| !is_mutable(p) && !is_conflicted(p))
                            .filter_map(|p| p.link_id.clone())
                            .collect();

                        if links.is_empty() {
                            debug!("fetch {} items at full tier", pending.len());
                            self.state = State::Fetching;
                            return PimdirCoroutineState::Yielded(PimdirYield::WantsFetch {
                                collection: self.collection.clone(),
                                handles: pending,
                                tier: PimdirTier::Full,
                            });
                        }

                        debug!("look up {} link ids in object store", links.len());
                        trace!("link ids: {links:?}");
                        self.state = State::LookingUp;
                        PimdirCoroutineState::Yielded(PimdirYield::WantsLookupObject(links))
                    }
                }
            }

            (State::LookingUp, Some(PimdirArg::LookupObject(known))) => {
                let mut to_fetch = Vec::new();

                for handle in self.pending_handles() {
                    let Some(placement) = self.placements.get(&handle) else {
                        continue;
                    };
                    // NOTE: linked only when the size the source served
                    // agrees with the held body (SYNC §6): two messages
                    // under one Message-ID with different bytes are the
                    // wrong merge, and the summary's size is the witness.
                    let hit = placement
                        .link_id
                        .as_ref()
                        .filter(|_| !is_mutable(placement) && !is_conflicted(placement))
                        .and_then(|link| known.get(link))
                        .filter(|object| size_agrees(placement, object.size))
                        .map(|object| object.hash.clone());

                    match hit {
                        Some(hash) => {
                            let mut patched = placement.clone();
                            // NOTE: the base moves with the body, or the
                            // link reads as a local edit on every sync.
                            if let Some(base) = &mut patched.base {
                                base.object = Some(hash.clone());
                            }
                            patched.object = Some(hash);
                            patched.level = PimdirLevel::Full;
                            self.ops.push(PimdirWriteOp::UpsertPlacement(patched));
                            self.report.upgraded += 1;
                            self.report.deduped += 1;
                        }
                        None => to_fetch.push(handle),
                    }
                }

                if to_fetch.is_empty() {
                    debug!("linked {} bodies from store, no fetch", self.report.deduped);
                    self.state = State::Writing;
                    let ops = mem::take(&mut self.ops);
                    return PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops));
                }

                debug!(
                    "fetch {} bodies, {} linked from store",
                    to_fetch.len(),
                    self.report.deduped,
                );
                self.state = State::Fetching;
                PimdirCoroutineState::Yielded(PimdirYield::WantsFetch {
                    collection: self.collection.clone(),
                    handles: to_fetch,
                    tier: PimdirTier::Full,
                })
            }

            (State::Fetching, Some(PimdirArg::Fetch(items))) => {
                trace!("fetched {} items", items.len());

                // NOTE: checked against the whole collection, not the batch,
                // else hydrating only the second copy would link it. The
                // minted key rides along, a source may spell its own like one,
                // and the hint's holders include a pending create to land.
                let fresh: Vec<PimdirLinkId> = items
                    .iter()
                    .filter(|item| {
                        self.placements
                            .get(&item.handle)
                            .is_some_and(|p| p.link_id.is_none())
                    })
                    .flat_map(|item| [item.link_id.clone(), item.link_id.minted(&item.handle)])
                    .collect();

                self.fetched = items;
                if fresh.is_empty() {
                    return self.write_fetched();
                }

                debug!(
                    "check {} fresh link ids against the collection",
                    fresh.len()
                );
                self.state = State::CheckingLinks;
                PimdirCoroutineState::Yielded(PimdirYield::WantsLoad {
                    collection: self.collection.clone(),
                    scope: PimdirLoadScope::Links(fresh),
                })
            }

            (State::CheckingLinks, Some(PimdirArg::Load(loaded))) => {
                for placement in loaded.placements {
                    self.placements
                        .entry(placement.handle.clone())
                        .or_insert(placement);
                }
                self.write_fetched()
            }

            (State::Writing, Some(PimdirArg::Write)) => {
                debug!(
                    "upgraded {} items ({} fetched, {} linked from store)",
                    self.report.upgraded, self.report.fetched, self.report.deduped,
                );
                self.state = State::Done;
                PimdirCoroutineState::Complete(Ok(self.report))
            }

            (State::Done, _) | (_, Some(_)) => {
                PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg))
            }
            (_, None) => PimdirCoroutineState::Complete(Err(PimdirArgError::MissingArg)),
        }
    }
}

/// Splits a fetched body into the object to record and the bytes to store.
///
/// A body the consumer already streamed into its blob store has no bytes.
fn stored_body(body: PimdirFetchedBody) -> (PimdirObject, Option<Vec<u8>>) {
    match body {
        PimdirFetchedBody::Inline { hash, bytes } => {
            let object = PimdirObject {
                hash,
                size: bytes.len(),
            };

            (object, Some(bytes))
        }
        PimdirFetchedBody::Persisted { hash, size } => (PimdirObject { hash, size }, None),
    }
}

/// Whether the placement holds the local side of a divergence.
///
/// A fetch of it answers what the remote holds instead, so it is fetched
/// rather than linked from the store: the conflict is about bytes the
/// remote alone has.
fn is_conflicted(placement: &PimdirPlacement) -> bool {
    placement.status == PimdirStatus::Conflict
}

/// Whether the content is mutable, which a last-synced revision marks.
///
/// Such a placement is fetched rather than linked from the store: a link
/// id says two copies are the same item, not that they hold the same
/// bytes.
fn is_mutable(placement: &PimdirPlacement) -> bool {
    placement
        .base
        .as_ref()
        .is_some_and(|base| base.revision.is_some())
}

/// Whether a held body's size agrees with the size the source served in
/// the placement's summary, a summary stating none agreeing with any.
fn size_agrees(placement: &PimdirPlacement, size: usize) -> bool {
    match &placement.summary {
        Some(PimdirSummary::Mail(mail)) => mail.size.is_none_or(|served| served == size as u64),
        _ => true,
    }
}

/// What the coroutine is doing while it waits for the caller.
enum State {
    Start,
    Loading,
    LookingUp,
    Fetching,
    CheckingLinks,
    Writing,
    Done,
}

#[cfg(test)]
mod tests {
    use alloc::{collections::BTreeMap, string::String, vec};

    use crate::{
        load::PimdirLoaded,
        object::PimdirHash,
        placement::{PimdirBase, PimdirFlags, PimdirLinkId, PimdirOrigin, PimdirStatus},
        remote::{PimdirFetchedBody, PimdirFetchedItem},
        upgrade::*,
    };

    /// The placement an `UpsertPlacement` op writes for `handle`, if any.
    fn upserted<'a>(ops: &'a [PimdirWriteOp], handle: &str) -> Option<&'a PimdirPlacement> {
        ops.iter().rev().find_map(|op| match op {
            PimdirWriteOp::UpsertPlacement(p) if p.handle.as_str() == handle => Some(p),
            _ => None,
        })
    }

    fn probed(handle: &str, link: Option<&str>, level: PimdirLevel) -> PimdirPlacement {
        PimdirPlacement {
            sort_key: Default::default(),
            collection: "inbox".into(),
            handle: PimdirHandle::from(handle),
            link_id: link.map(PimdirLinkId::from),
            object: None,
            level,
            summary: None,
            flags: PimdirFlags::default(),
            status: PimdirStatus::Clean,
            conflict_revision: None,
            conflict_object: None,
            base: None,
            origin: None,
        }
    }

    #[test]
    fn full_dedup_links_without_fetch() {
        crate::testlog::init();
        let loaded = PimdirLoaded {
            placements: vec![probed("2", Some("msg-a"), PimdirLevel::Meta)],
            checkpoint: None,
        };
        let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("2")], PimdirTier::Full);
        let _ = up.resume(None);

        let links = match up.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsLookupObject(links)) => links,
            state => panic!("expected WantsLookupObject, got {state:?}"),
        };
        assert_eq!(links, vec![PimdirLinkId::from("msg-a")]);

        let mut known = BTreeMap::new();
        known.insert(
            PimdirLinkId::from("msg-a"),
            PimdirObject {
                hash: PimdirHash::from("h-a"),
                size: 1,
            },
        );

        let ops = match up.resume(Some(PimdirArg::LookupObject(known))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite (no fetch), got {state:?}"),
        };
        let PimdirWriteOp::UpsertPlacement(p) = &ops[0] else {
            panic!("expected UpsertPlacement, got {:?}", ops[0]);
        };
        assert_eq!(p.level, PimdirLevel::Full);
        assert_eq!(p.object, Some(PimdirHash::from("h-a")));

        let report = match up.resume(Some(PimdirArg::Write)) {
            PimdirCoroutineState::Complete(Ok(report)) => report,
            state => panic!("expected Complete(Ok), got {state:?}"),
        };
        assert_eq!(report.deduped, 1);
        assert_eq!(report.fetched, 0);
    }

    #[test]
    fn full_miss_fetches_and_stores() {
        let loaded = PimdirLoaded {
            placements: vec![probed("1", Some("msg-b"), PimdirLevel::Meta)],
            checkpoint: None,
        };
        let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Full);
        let _ = up.resume(None);
        let _ = up.resume(Some(PimdirArg::Load(loaded)));

        let handles = match up.resume(Some(PimdirArg::LookupObject(BTreeMap::new()))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsFetch { handles, tier, .. }) => {
                assert_eq!(tier, PimdirTier::Full);
                handles
            }
            state => panic!("expected WantsFetch, got {state:?}"),
        };
        assert_eq!(handles, vec![PimdirHandle::from("1")]);

        let items = vec![PimdirFetchedItem {
            sort_key: Default::default(),
            handle: PimdirHandle::from("1"),
            link_id: PimdirLinkId::from("msg-b"),
            summary: Some(crate::summary::stub("hdr")),
            body: Some(PimdirFetchedBody::Inline {
                hash: PimdirHash::from("h-b"),
                bytes: b"body".to_vec(),
            }),
            revision: None,
        }];
        let ops = match up.resume(Some(PimdirArg::Fetch(items))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };
        assert!(matches!(ops[0], PimdirWriteOp::StoreObject { .. }));

        let report = match up.resume(Some(PimdirArg::Write)) {
            PimdirCoroutineState::Complete(Ok(report)) => report,
            state => panic!("expected Complete(Ok), got {state:?}"),
        };
        assert_eq!(report.fetched, 1);
        assert_eq!(report.deduped, 0);
    }

    #[test]
    fn fetch_results_are_matched_by_handle_not_order() {
        let loaded = PimdirLoaded {
            placements: vec![
                probed("1", Some("msg-a"), PimdirLevel::Meta),
                probed("2", Some("msg-b"), PimdirLevel::Meta),
            ],
            checkpoint: None,
        };
        let mut up = PimdirUpgrade::new(
            "inbox",
            vec![PimdirHandle::from("1"), PimdirHandle::from("2")],
            PimdirTier::Full,
        );
        let _ = up.resume(None);
        let _ = up.resume(Some(PimdirArg::Load(loaded)));
        let _ = up.resume(Some(PimdirArg::LookupObject(BTreeMap::new())));

        let items = vec![
            PimdirFetchedItem {
                sort_key: Default::default(),
                handle: PimdirHandle::from("2"),
                link_id: PimdirLinkId::from("msg-b"),
                summary: Some(crate::summary::stub("h")),
                body: Some(PimdirFetchedBody::Inline {
                    hash: PimdirHash::from("h-b"),
                    bytes: b"bbb".to_vec(),
                }),
                revision: None,
            },
            PimdirFetchedItem {
                sort_key: Default::default(),
                handle: PimdirHandle::from("1"),
                link_id: PimdirLinkId::from("msg-a"),
                summary: Some(crate::summary::stub("h")),
                body: Some(PimdirFetchedBody::Inline {
                    hash: PimdirHash::from("h-a"),
                    bytes: b"aaaaa".to_vec(),
                }),
                revision: None,
            },
        ];
        let ops = match up.resume(Some(PimdirArg::Fetch(items))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };

        let object_for = |handle: &str| {
            ops.iter().find_map(|op| match op {
                PimdirWriteOp::UpsertPlacement(p) if p.handle.as_str() == handle => {
                    p.object.clone()
                }
                _ => None,
            })
        };
        assert_eq!(object_for("1"), Some(PimdirHash::from("h-a")));
        assert_eq!(object_for("2"), Some(PimdirHash::from("h-b")));
    }

    #[test]
    fn a_full_fetch_keeps_an_already_resolved_link_id() {
        let loaded = PimdirLoaded {
            placements: vec![probed("1", Some("mid:real"), PimdirLevel::Meta)],
            checkpoint: None,
        };
        let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Full);
        let _ = up.resume(None);
        let _ = up.resume(Some(PimdirArg::Load(loaded)));
        let _ = up.resume(Some(PimdirArg::LookupObject(BTreeMap::new())));

        let items = vec![PimdirFetchedItem {
            sort_key: Default::default(),
            handle: PimdirHandle::from("1"),
            link_id: PimdirLinkId::from("alt:divergent"),
            summary: Some(crate::summary::stub("hdr")),
            body: Some(PimdirFetchedBody::Persisted {
                hash: PimdirHash::from("h"),
                size: 10,
            }),
            revision: None,
        }];
        let ops = match up.resume(Some(PimdirArg::Fetch(items))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };
        let placement = ops
            .iter()
            .find_map(|op| match op {
                PimdirWriteOp::UpsertPlacement(p) => Some(p),
                _ => None,
            })
            .expect("a placement upsert");
        assert_eq!(
            placement.link_id,
            Some(PimdirLinkId::from("mid:real")),
            "the Full fetch keeps the Meta-resolved link, not the body's"
        );
        assert_eq!(placement.level, PimdirLevel::Full);
    }

    #[test]
    fn a_meta_fetch_still_sets_the_link_of_an_unlinked_item() {
        let loaded = PimdirLoaded {
            placements: vec![probed("1", None, PimdirLevel::Probed)],
            checkpoint: None,
        };
        let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Meta);
        let _ = up.resume(None);
        let _ = up.resume(Some(PimdirArg::Load(loaded)));
        let _ = up.resume(Some(PimdirArg::LookupObject(BTreeMap::new())));

        let items = vec![PimdirFetchedItem {
            sort_key: Default::default(),
            handle: PimdirHandle::from("1"),
            link_id: PimdirLinkId::from("mid:resolved"),
            summary: Some(crate::summary::stub("hdr")),
            body: None,
            revision: None,
        }];
        match up.resume(Some(PimdirArg::Fetch(items))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsLoad {
                scope: PimdirLoadScope::Links(links),
                ..
            }) => assert_eq!(
                links,
                vec![
                    PimdirLinkId::from("mid:resolved"),
                    PimdirLinkId::from("dup:mid:resolved#1"),
                ],
            ),
            state => panic!("expected WantsLoad, got {state:?}"),
        }
        let ops = match up.resume(Some(PimdirArg::Load(PimdirLoaded::default()))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };
        let placement = ops
            .iter()
            .find_map(|op| match op {
                PimdirWriteOp::UpsertPlacement(p) => Some(p),
                _ => None,
            })
            .expect("a placement upsert");
        assert_eq!(
            placement.link_id,
            Some(PimdirLinkId::from("mid:resolved")),
            "a probed item takes the fetched link"
        );
    }

    /// The link ids the upgrade wrote, by handle.
    fn links(ops: &[PimdirWriteOp]) -> BTreeMap<&str, Option<&str>> {
        ops.iter()
            .filter_map(|op| match op {
                PimdirWriteOp::UpsertPlacement(p) => {
                    Some((p.handle.as_str(), p.link_id.as_ref().map(|l| l.as_str())))
                }
                _ => None,
            })
            .collect()
    }

    /// The write batch of a meta upgrade of `handles` resolving to `link`.
    ///
    /// The fresh identity is checked against the `stored` placements.
    fn upgrade_twins(
        handles: &[&str],
        link: &str,
        loaded: Vec<PimdirPlacement>,
        stored: Vec<PimdirPlacement>,
    ) -> Vec<PimdirWriteOp> {
        crate::testlog::init();
        let requested = handles.iter().copied().map(PimdirHandle::from).collect();
        let mut up = PimdirUpgrade::new("inbox", requested, PimdirTier::Meta);
        let _ = up.resume(None);
        let _ = up.resume(Some(PimdirArg::Load(PimdirLoaded {
            placements: loaded,
            checkpoint: None,
        })));

        let items = handles
            .iter()
            .map(|handle| PimdirFetchedItem {
                sort_key: Default::default(),
                handle: PimdirHandle::from(*handle),
                link_id: PimdirLinkId::from(link),
                summary: Some(crate::summary::stub("hdr")),
                body: None,
                revision: None,
            })
            .collect();

        let ops = match up.resume(Some(PimdirArg::Fetch(items))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => Some(ops),
            PimdirCoroutineState::Yielded(PimdirYield::WantsLoad {
                scope: PimdirLoadScope::Links(_),
                ..
            }) => None,
            state => panic!("expected WantsWrite or a link check, got {state:?}"),
        };

        match ops {
            Some(ops) => ops,
            None => match up.resume(Some(PimdirArg::Load(PimdirLoaded {
                placements: stored,
                checkpoint: None,
            }))) {
                PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
                state => panic!("expected WantsWrite, got {state:?}"),
            },
        }
    }

    /// Both copies are hydrated by one batch, neither linked yet.
    #[test]
    fn a_second_copy_of_one_identity_is_minted() {
        let ops = upgrade_twins(
            &["u1", "u2"],
            "m1",
            vec![
                probed("u1", None, PimdirLevel::Probed),
                probed("u2", None, PimdirLevel::Probed),
            ],
            Vec::new(),
        );

        assert_eq!(
            links(&ops),
            BTreeMap::from([("u1", Some("m1")), ("u2", Some("dup:m1#u2"))]),
            "the first copy keeps the hint, the second is minted from it \
             and its own handle",
        );
    }

    /// Only the second copy is hydrated: the holder comes from the check.
    #[test]
    fn the_mint_is_decided_against_the_collection_not_the_batch() {
        let ops = upgrade_twins(
            &["u2"],
            "m1",
            vec![probed("u2", None, PimdirLevel::Probed)],
            vec![probed("u1", Some("m1"), PimdirLevel::Meta)],
        );

        assert_eq!(
            links(&ops),
            BTreeMap::from([("u2", Some("dup:m1#u2"))]),
            "a batch that never names the holder still mints",
        );
    }

    #[test]
    fn a_minted_copy_is_not_minted_again() {
        let ops = upgrade_twins(
            &["u2"],
            "m1",
            vec![probed("u2", Some("dup:m1#u2"), PimdirLevel::Probed)],
            vec![probed("u1", Some("m1"), PimdirLevel::Meta)],
        );

        assert_eq!(
            links(&ops),
            BTreeMap::from([("u2", Some("dup:m1#u2"))]),
            "no dup:dup:m1#u2#u2",
        );
    }

    /// A pending create of `link` under a provisional handle, body `h-c`.
    fn pending_create(handle: &str, link: &str, flags: &[&str]) -> PimdirPlacement {
        let mut create = probed(handle, Some(link), PimdirLevel::Full);
        create.object = Some(PimdirHash::from("h-c"));
        create.flags = PimdirFlags::from_iter(flags.iter().copied());
        create.summary = Some(crate::summary::stub("staged"));
        create.status = PimdirStatus::Created;
        create.origin = Some(PimdirOrigin {
            collection: "sent".into(),
            handle: PimdirHandle::from("9"),
        });
        create
    }

    /// A hint a pending create holds is that create arriving (SYNC §6).
    ///
    /// The provisional handle is superseded in the same batch, the binding
    /// moves onto the fetched handle, and the base is what the fetch reported
    /// while the staged flags and body stay.
    #[test]
    fn a_fetched_hint_held_by_a_pending_create_lands_it() {
        let mut probe = probed("7", None, PimdirLevel::Probed);
        probe.flags = PimdirFlags::from_iter(["seen"]);
        let ops = upgrade_twins(
            &["7"],
            "m1",
            vec![probe],
            vec![pending_create("tmp-1", "m1", &["seen"])],
        );

        assert_eq!(
            ops[0],
            PimdirWriteOp::DropPlacement {
                collection: "inbox".into(),
                handle: PimdirHandle::from("tmp-1"),
                reason: PimdirDropReason::Superseded,
            },
            "the provisional handle goes first: {ops:?}",
        );
        let landed = upserted(&ops, "7").expect("the create under the fetched handle");
        assert_eq!(landed.link_id, Some(PimdirLinkId::from("m1")), "not minted");
        assert_eq!(landed.status, PimdirStatus::Clean);
        assert_eq!(
            landed.object,
            Some(PimdirHash::from("h-c")),
            "the body stays"
        );
        assert_eq!(landed.level, PimdirLevel::Full);
        assert_eq!(landed.summary, Some(crate::summary::stub("staged")));
        assert_eq!(landed.origin, None, "landed, so nothing left to copy from");
        let base = landed.base.as_ref().expect("landing bases the create");
        assert_eq!(base.flags, PimdirFlags::from_iter(["seen"]));
        assert_eq!(base.object, Some(PimdirHash::from("h-c")));
        assert!(
            upserted(&ops, "tmp-1").is_none(),
            "the provisional row is not rewritten: {ops:?}",
        );
    }

    /// The flags and body staged on the create still push after landing.
    #[test]
    fn a_landed_create_keeps_its_staged_edit_pending() {
        let mut probe = probed("7", None, PimdirLevel::Probed);
        probe.flags = PimdirFlags::from_iter(["seen"]);
        let ops = upgrade_twins(
            &["7"],
            "m1",
            vec![probe],
            vec![pending_create("tmp-1", "m1", &["seen", "flagged"])],
        );

        let landed = upserted(&ops, "7").expect("the landed create");
        assert_eq!(landed.status, PimdirStatus::Dirty);
        assert!(landed.flags.contains("flagged"), "the staged flag stays");
        let base = landed.base.as_ref().expect("a base");
        assert!(
            !base.flags.contains("flagged"),
            "the base is what was fetched"
        );
    }

    /// A hint a based binding holds is a second copy, minted as before.
    #[test]
    fn a_hint_held_by_a_based_binding_is_still_minted() {
        let ops = upgrade_twins(
            &["7"],
            "m1",
            vec![probed("7", None, PimdirLevel::Probed)],
            vec![based("u1", "m1", None)],
        );

        assert_eq!(links(&ops), BTreeMap::from([("7", Some("dup:m1#7"))]));
        assert!(
            !ops.iter()
                .any(|op| matches!(op, PimdirWriteOp::DropPlacement { .. })),
            "nothing is superseded: {ops:?}",
        );
    }

    /// A source is free to spell its own identity like a minted key.
    #[test]
    fn a_mint_never_takes_a_key_the_collection_holds() {
        let ops = upgrade_twins(
            &["u2"],
            "m1",
            vec![probed("u2", None, PimdirLevel::Probed)],
            vec![
                probed("u1", Some("m1"), PimdirLevel::Meta),
                probed("u3", Some("dup:m1#u2"), PimdirLevel::Meta),
            ],
        );

        assert_eq!(
            links(&ops),
            BTreeMap::from([("u2", Some("dup:dup:m1#u2#u2"))]),
            "the copy takes a key of its own rather than u3's",
        );
    }

    #[test]
    fn a_meta_fetch_keeps_a_body_holding_row_full() {
        let mut stored = probed("1", Some("msg-a"), PimdirLevel::Full);
        stored.object = Some(PimdirHash::from("h1"));
        let loaded = PimdirLoaded {
            placements: vec![stored],
            checkpoint: None,
        };
        let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Meta);
        let _ = up.resume(None);
        let _ = up.resume(Some(PimdirArg::Load(loaded)));

        let items = vec![PimdirFetchedItem {
            sort_key: Default::default(),
            handle: PimdirHandle::from("1"),
            link_id: PimdirLinkId::from("msg-a"),
            summary: Some(crate::summary::stub("hdr")),
            body: None,
            revision: None,
        }];
        let ops = match up.resume(Some(PimdirArg::Fetch(items))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };

        let upserted = ops
            .iter()
            .find_map(|op| match op {
                PimdirWriteOp::UpsertPlacement(p) => Some(p),
                _ => None,
            })
            .expect("the summarised placement");
        assert_eq!(upserted.level, PimdirLevel::Full);
        assert_eq!(upserted.summary, Some(crate::summary::stub("hdr")));
    }

    #[test]
    fn a_persisted_body_stores_the_object_without_bytes() {
        let loaded = PimdirLoaded {
            placements: vec![probed("1", Some("msg-b"), PimdirLevel::Meta)],
            checkpoint: None,
        };
        let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Full);
        let _ = up.resume(None);
        let _ = up.resume(Some(PimdirArg::Load(loaded)));
        let _ = up.resume(Some(PimdirArg::LookupObject(BTreeMap::new())));

        let items = vec![PimdirFetchedItem {
            sort_key: Default::default(),
            handle: PimdirHandle::from("1"),
            link_id: PimdirLinkId::from("msg-b"),
            summary: Some(crate::summary::stub("hdr")),
            body: Some(PimdirFetchedBody::Persisted {
                hash: PimdirHash::from("h-b"),
                size: 4096,
            }),
            revision: None,
        }];
        let ops = match up.resume(Some(PimdirArg::Fetch(items))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };
        match &ops[0] {
            PimdirWriteOp::StoreObject { object, body } => {
                assert_eq!(object.hash, PimdirHash::from("h-b"));
                assert_eq!(object.size, 4096, "size comes from the report, not bytes");
                assert!(body.is_none(), "no bytes: the fetch already persisted them");
            }
            other => panic!("expected StoreObject, got {other:?}"),
        }
        assert!(matches!(
            &ops[1],
            PimdirWriteOp::UpsertPlacement(p)
                if p.object == Some(PimdirHash::from("h-b")) && p.level == PimdirLevel::Full
        ));
    }

    #[test]
    fn full_fetch_stamps_the_base_revision_and_object() {
        let mut placement = probed("1", Some("msg-b"), PimdirLevel::Meta);
        placement.base = Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: None,
            object: None,
        });
        let loaded = PimdirLoaded {
            placements: vec![placement],
            checkpoint: None,
        };

        let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Full);
        let _ = up.resume(None);
        let _ = up.resume(Some(PimdirArg::Load(loaded)));
        let _ = up.resume(Some(PimdirArg::LookupObject(BTreeMap::new())));

        let items = vec![PimdirFetchedItem {
            sort_key: Default::default(),
            handle: PimdirHandle::from("1"),
            link_id: PimdirLinkId::from("msg-b"),
            summary: Some(crate::summary::stub("hdr")),
            body: Some(PimdirFetchedBody::Inline {
                hash: PimdirHash::from("h-b"),
                bytes: b"body".to_vec(),
            }),
            revision: Some("r7".into()),
        }];
        let ops = match up.resume(Some(PimdirArg::Fetch(items))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };

        let patched = ops
            .iter()
            .find_map(|op| match op {
                PimdirWriteOp::UpsertPlacement(p) => Some(p),
                _ => None,
            })
            .expect("an upserted placement");
        let base = patched.base.as_ref().expect("a base");
        assert_eq!(base.revision.as_deref(), Some("r7"));
        assert_eq!(base.object, Some(PimdirHash::from("h-b")));
        assert_eq!(patched.object, Some(PimdirHash::from("h-b")));
    }

    #[test]
    fn meta_upgrade_fetches_headers() {
        let loaded = PimdirLoaded {
            placements: vec![probed("1", None, PimdirLevel::Probed)],
            checkpoint: None,
        };
        let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Meta);
        let _ = up.resume(None);

        match up.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsFetch { tier, .. }) => {
                assert_eq!(tier, PimdirTier::Meta);
            }
            state => panic!("expected WantsFetch Meta, got {state:?}"),
        }
    }

    #[test]
    fn already_full_completes_without_work() {
        let mut placement = probed("1", Some("x"), PimdirLevel::Full);
        placement.object = Some(PimdirHash::from("h1"));
        let loaded = PimdirLoaded {
            placements: vec![placement],
            checkpoint: None,
        };
        let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Full);
        let _ = up.resume(None);

        match up.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Complete(Ok(report)) => assert_eq!(report.upgraded, 0),
            state => panic!("expected Complete(Ok), got {state:?}"),
        }
    }

    /// Else a row recorded full with no body would be skipped forever.
    #[test]
    fn a_full_row_holding_no_body_is_upgraded_again() {
        let loaded = PimdirLoaded {
            placements: vec![probed("1", None, PimdirLevel::Full)],
            checkpoint: None,
        };
        let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Full);
        let _ = up.resume(None);

        match up.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsFetch { handles, .. }) => {
                assert_eq!(handles, vec![PimdirHandle::from("1")]);
            }
            state => panic!("expected WantsFetch, got {state:?}"),
        }
    }

    #[test]
    fn a_meta_row_holding_no_summary_is_upgraded_again() {
        let loaded = PimdirLoaded {
            placements: vec![probed("1", None, PimdirLevel::Meta)],
            checkpoint: None,
        };
        let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Meta);
        let _ = up.resume(None);

        match up.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsFetch { handles, .. }) => {
                assert_eq!(handles, vec![PimdirHandle::from("1")]);
            }
            state => panic!("expected WantsFetch, got {state:?}"),
        }
    }

    #[test]
    fn missing_arg_errors() {
        let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Meta);
        let _ = up.resume(None);
        match up.resume(None) {
            PimdirCoroutineState::Complete(Err(PimdirArgError::MissingArg)) => {}
            state => panic!("expected MissingArg, got {state:?}"),
        }
    }

    /// An empty report would pass for a run that did nothing.
    #[test]
    fn a_completed_upgrade_does_not_resume() {
        let mut placement = probed("1", Some("x"), PimdirLevel::Full);
        placement.object = Some(PimdirHash::from("h1"));
        let loaded = PimdirLoaded {
            placements: vec![placement],
            checkpoint: None,
        };
        let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Full);
        let _ = up.resume(None);
        let _ = up.resume(Some(PimdirArg::Load(loaded)));

        match up.resume(Some(PimdirArg::Write)) {
            PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => {}
            state => panic!("expected UnexpectedArg, got {state:?}"),
        }
        match up.resume(None) {
            PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => {}
            state => panic!("expected UnexpectedArg, got {state:?}"),
        }
    }

    #[test]
    fn unexpected_arg_errors() {
        let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Meta);
        let _ = up.resume(None);
        match up.resume(Some(PimdirArg::Write)) {
            PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => {}
            state => panic!("expected UnexpectedArg, got {state:?}"),
        }
    }

    #[test]
    fn unknown_handle_completes_without_work() {
        let loaded = PimdirLoaded {
            placements: vec![probed("1", None, PimdirLevel::Probed)],
            checkpoint: None,
        };
        let mut up =
            PimdirUpgrade::new("inbox", vec![PimdirHandle::from("nope")], PimdirTier::Meta);
        let _ = up.resume(None);

        match up.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Complete(Ok(report)) => assert_eq!(report.upgraded, 0),
            state => panic!("expected Complete(Ok), got {state:?}"),
        }
    }

    #[test]
    fn full_without_link_ids_fetches_directly() {
        let loaded = PimdirLoaded {
            placements: vec![probed("1", None, PimdirLevel::Probed)],
            checkpoint: None,
        };
        let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Full);
        let _ = up.resume(None);

        match up.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsFetch { tier, handles, .. }) => {
                assert_eq!(tier, PimdirTier::Full);
                assert_eq!(handles, vec![PimdirHandle::from("1")]);
            }
            state => panic!("expected WantsFetch Full, got {state:?}"),
        }
    }

    #[test]
    fn fetched_unknown_handle_is_skipped() {
        let loaded = PimdirLoaded {
            placements: vec![probed("1", None, PimdirLevel::Probed)],
            checkpoint: None,
        };
        let mut up = PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Meta);
        let _ = up.resume(None);
        let _ = up.resume(Some(PimdirArg::Load(loaded)));

        let items = vec![PimdirFetchedItem {
            sort_key: Default::default(),
            handle: PimdirHandle::from("ghost"),
            link_id: PimdirLinkId::from("msg-x"),
            summary: Some(crate::summary::stub("hdr")),
            body: None,
            revision: None,
        }];
        let ops = match up.resume(Some(PimdirArg::Fetch(items))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };
        assert!(ops.is_empty(), "nothing to write: {ops:?}");

        match up.resume(Some(PimdirArg::Write)) {
            PimdirCoroutineState::Complete(Ok(report)) => assert_eq!(report.upgraded, 0),
            state => panic!("expected Complete(Ok), got {state:?}"),
        }
    }

    #[test]
    fn full_mixes_dedup_hits_and_fetch_misses() {
        crate::testlog::init();
        let loaded = PimdirLoaded {
            placements: vec![
                probed("1", Some("msg-a"), PimdirLevel::Meta),
                probed("2", Some("msg-b"), PimdirLevel::Meta),
            ],
            checkpoint: None,
        };
        let mut up = PimdirUpgrade::new(
            "inbox",
            vec![PimdirHandle::from("1"), PimdirHandle::from("2")],
            PimdirTier::Full,
        );
        let _ = up.resume(None);
        let _ = up.resume(Some(PimdirArg::Load(loaded)));

        let mut known = BTreeMap::new();
        known.insert(
            PimdirLinkId::from("msg-a"),
            PimdirObject {
                hash: PimdirHash::from("h-a"),
                size: 1,
            },
        );

        let handles = match up.resume(Some(PimdirArg::LookupObject(known))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsFetch { handles, .. }) => handles,
            state => panic!("expected WantsFetch for the miss, got {state:?}"),
        };
        assert_eq!(
            handles,
            vec![PimdirHandle::from("2")],
            "only the miss fetches"
        );

        let items = vec![PimdirFetchedItem {
            sort_key: Default::default(),
            handle: PimdirHandle::from("2"),
            link_id: PimdirLinkId::from("msg-b"),
            summary: Some(crate::summary::stub("hdr")),
            body: Some(PimdirFetchedBody::Inline {
                hash: PimdirHash::from("h-b"),
                bytes: b"body".to_vec(),
            }),
            revision: None,
        }];
        let _ = up.resume(Some(PimdirArg::Fetch(items)));

        let report = match up.resume(Some(PimdirArg::Write)) {
            PimdirCoroutineState::Complete(Ok(report)) => report,
            state => panic!("expected Complete(Ok), got {state:?}"),
        };
        assert_eq!(report.upgraded, 2);
        assert_eq!(report.deduped, 1);
        assert_eq!(report.fetched, 1);
    }

    /// A placement reconciled once: based, summarised, at `revision`.
    fn based(handle: &str, link: &str, revision: Option<&str>) -> PimdirPlacement {
        let mut placement = probed(handle, Some(link), PimdirLevel::Meta);
        placement.base = Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: revision.map(String::from),
            object: None,
        });
        placement
    }

    /// Runs a full upgrade of one placement past the link lookup.
    ///
    /// The lookup is answered with `known`.
    fn upgrade_with_lookup(
        placement: PimdirPlacement,
        known: BTreeMap<PimdirLinkId, PimdirHash>,
    ) -> PimdirCoroutineState<PimdirYield, Result<PimdirUpgradeReport, PimdirArgError>> {
        let known: BTreeMap<PimdirLinkId, PimdirObject> = known
            .into_iter()
            .map(|(link, hash)| (link, PimdirObject { hash, size: 1 }))
            .collect();
        let handle = placement.handle.clone();
        let loaded = PimdirLoaded {
            placements: vec![placement],
            checkpoint: None,
        };
        let mut up = PimdirUpgrade::new("inbox", vec![handle], PimdirTier::Full);
        let _ = up.resume(None);

        match up.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsLookupObject(_)) => {
                up.resume(Some(PimdirArg::LookupObject(known)))
            }
            state => state,
        }
    }

    /// A base left behind would read as a local edit on every sync.
    #[test]
    fn a_deduped_body_rebases_so_the_placement_reads_clean() {
        let known = BTreeMap::from([(PimdirLinkId::from("msg-a"), PimdirHash::from("h-a"))]);

        let ops = match upgrade_with_lookup(based("2", "msg-a", None), known) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite (no fetch), got {state:?}"),
        };
        let PimdirWriteOp::UpsertPlacement(placement) = &ops[0] else {
            panic!("expected UpsertPlacement, got {:?}", ops[0]);
        };

        assert_eq!(placement.object, Some(PimdirHash::from("h-a")));
        assert_eq!(placement.level, PimdirLevel::Full);
        assert_eq!(
            placement.base.as_ref().and_then(|base| base.object.clone()),
            Some(PimdirHash::from("h-a")),
            "the base holds the linked body, so nothing reads as edited"
        );
    }

    #[test]
    fn a_mutable_placement_is_fetched_rather_than_linked() {
        let known = BTreeMap::from([(PimdirLinkId::from("uid:card-1"), PimdirHash::from("h-a"))]);

        let state = upgrade_with_lookup(based("card-1.vcf", "uid:card-1", Some("etag-1")), known);

        match state {
            PimdirCoroutineState::Yielded(PimdirYield::WantsFetch { handles, tier, .. }) => {
                assert_eq!(tier, PimdirTier::Full);
                assert_eq!(handles, vec![PimdirHandle::from("card-1.vcf")]);
            }
            state => panic!("expected WantsFetch, got {state:?}"),
        }
    }

    /// A based, mutable card holding the local side of a divergence.
    fn conflicted(conflict_object: Option<&str>) -> PimdirPlacement {
        let mut placement = based("card-1.vcf", "uid:card-1", Some("etag-1"));
        placement.object = Some(PimdirHash::from("h-local"));
        placement.level = PimdirLevel::Full;
        placement.status = PimdirStatus::Conflict;
        placement.conflict_revision = Some(String::from("etag-2"));
        placement.conflict_object = conflict_object.map(PimdirHash::from);
        placement
    }

    /// Runs a full upgrade of one placement up to its yield after the load.
    fn upgrade_full(
        placement: PimdirPlacement,
    ) -> (
        PimdirUpgrade,
        PimdirCoroutineState<PimdirYield, Result<PimdirUpgradeReport, PimdirArgError>>,
    ) {
        let handle = placement.handle.clone();
        let loaded = PimdirLoaded {
            placements: vec![placement],
            checkpoint: None,
        };
        let mut up = PimdirUpgrade::new("inbox", vec![handle], PimdirTier::Full);
        let _ = up.resume(None);
        let state = up.resume(Some(PimdirArg::Load(loaded)));

        (up, state)
    }

    /// It reads full and holds a body, so the level rule would skip it.
    #[test]
    fn a_conflicted_placement_asks_for_the_diverging_body() {
        let (_up, state) = upgrade_full(conflicted(None));

        match state {
            PimdirCoroutineState::Yielded(PimdirYield::WantsFetch { handles, tier, .. }) => {
                assert_eq!(tier, PimdirTier::Full);
                assert_eq!(handles, vec![PimdirHandle::from("card-1.vcf")]);
            }
            state => panic!("expected WantsFetch, got {state:?}"),
        }

        let (_up, state) = upgrade_full(conflicted(Some("h-remote")));

        match state {
            PimdirCoroutineState::Complete(Ok(report)) => assert_eq!(report.fetched, 0),
            state => panic!("expected Complete(Ok), got {state:?}"),
        }
    }

    /// Read as the local body, it would drop the edit under conflict.
    #[test]
    fn a_fetched_body_lands_as_the_conflict_object() {
        let (mut up, _state) = upgrade_full(conflicted(None));

        let items = vec![PimdirFetchedItem {
            handle: PimdirHandle::from("card-1.vcf"),
            link_id: PimdirLinkId::from("uid:card-1"),
            summary: Some(crate::summary::stub("remote")),
            sort_key: Default::default(),
            body: Some(PimdirFetchedBody::Inline {
                hash: PimdirHash::from("h-remote"),
                bytes: b"remote".to_vec(),
            }),
            revision: Some(String::from("etag-2")),
        }];

        let ops = match up.resume(Some(PimdirArg::Fetch(items))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };
        let PimdirWriteOp::UpsertPlacement(placement) = &ops[1] else {
            panic!("expected UpsertPlacement, got {:?}", ops[1]);
        };

        assert_eq!(
            placement.conflict_object,
            Some(PimdirHash::from("h-remote"))
        );
        assert_eq!(
            placement.object,
            Some(PimdirHash::from("h-local")),
            "the local side of the divergence is untouched"
        );
        assert_eq!(
            placement
                .base
                .as_ref()
                .and_then(|base| base.revision.clone()),
            Some(String::from("etag-1")),
            "nor does the fetch rebase what it never merged"
        );
    }
}
