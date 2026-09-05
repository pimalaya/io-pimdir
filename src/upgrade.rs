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
                patched.conflict_object = Some(hash);
                self.ops.push(PimdirWriteOp::UpsertPlacement(patched));
                self.report.fetched += 1;
                self.report.upgraded += 1;
                continue;
            }

            // NOTE: never re-identifies a linked item: tiers may disagree on
            // the link, an ENVELOPE naming a Message-ID the body parser misses.
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

                    if let Some(base) = &mut patched.base {
                        base.revision = item.revision.clone();
                        base.object = Some(hash.clone());
                    }

                    patched.object = Some(hash);
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
                    let hit = placement
                        .link_id
                        .as_ref()
                        .filter(|_| !is_mutable(placement) && !is_conflicted(placement))
                        .and_then(|link| known.get(link).cloned());

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
mod tests;
