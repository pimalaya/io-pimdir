//! # Rekey coroutine
//!
//! Rebuilds a collection after a handle-space change (an IMAP UIDVALIDITY
//! bump), carrying local state over to the new handles by link id
//! (SYNC §8).
//!
//! A plain full sync would read every old handle as deleted upstream and
//! drop cached bodies and pending changes with them. The rebuild instead
//! enumerates the new spine, resolves its link ids at the meta tier in
//! bounded chunks, and carries each old placement onto the new handle of
//! the same item.
//!
//! The cache survives without a refetch, flag deltas re-derive against
//! the new base, tombstones keep their pending remove and staged edits
//! their body. An edit whose item found no new home survives as a
//! pending create; other unmatched pending state is dropped and counted.
//!
//! Pending creates are local staging, not spine, and stay untouched. A
//! member whose fetched revision differs from the one its base held
//! changed on the remote while the handles did: it is carried as the
//! pull a sync would make, or as a conflict when it also holds a local
//! edit, never with a base claiming a revision it never reconciled.
//!
//! Identity keys the match, so two copies of one identity stay two items:
//! a source reports the shared hint, never the minted key, so the first
//! copy in handle order takes the hint and the next is carried onto its
//! minted key. A row carried nowhere is dropped as the deletion it is.

use core::mem;

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
    vec::Vec,
};

use log::{debug, trace};

use crate::{
    change::{PimdirDropReason, PimdirWriteOp},
    collection::{PimdirCheckpoint, PimdirCollectionId},
    coroutine::*,
    load::PimdirLoadScope,
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirSortKey, PimdirStatus,
    },
    remote::{PimdirRemoteItem, PimdirTier},
    summary::PimdirSummary,
};

/// What a rekey did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PimdirRekeyReport {
    /// Old placements carried over to their new handle.
    pub rekeyed: usize,
    /// New members with no old placement to carry, pulled fresh.
    pub pulled: usize,
    /// Old placements whose pending state matched nothing and was dropped.
    ///
    /// No link id was resolved before the change, or the item is gone
    /// from the new spine.
    pub dropped: usize,
}

/// I/O-free REKEY coroutine.
pub struct PimdirRekey {
    collection: PimdirCollectionId,
    old: Vec<PimdirPlacement>,
    items: Vec<PimdirRemoteItem>,
    checkpoint: Option<PimdirCheckpoint>,
    /// The new handles no meta fetch has resolved yet, in chunks.
    unresolved: Vec<PimdirHandle>,
    /// What the meta fetches resolved, by new handle.
    resolved: BTreeMap<PimdirHandle, Resolved>,
    report: PimdirRekeyReport,
    state: State,
}

impl PimdirRekey {
    /// How many handles one meta fetch of the new spine names.
    ///
    /// A handle-space change touches every member, so the fetch resolving
    /// their identities goes in bounded requests rather than one naming
    /// the whole mailbox; the rebuild itself still lands in one batch.
    pub const FETCH_CHUNK: usize = 256;

    /// Creates a coroutine rebuilding `collection` onto its new handles.
    pub fn new(collection: impl Into<PimdirCollectionId>) -> Self {
        let collection = collection.into();
        debug!("rekey collection {}", collection.as_str());

        Self {
            collection,
            old: Vec::new(),
            items: Vec::new(),
            checkpoint: None,
            unresolved: Vec::new(),
            resolved: BTreeMap::new(),
            report: PimdirRekeyReport::default(),
            state: State::Start,
        }
    }

    /// Yields the next meta fetch chunk, or the rebuild once all resolved.
    fn step(&mut self) -> PimdirCoroutineState<PimdirYield, <Self as PimdirCoroutine>::Return> {
        if self.unresolved.is_empty() {
            trace!("resolved {} link ids", self.resolved.len());
            self.state = State::Writing;
            let writes = self.rebuild();
            return PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(writes));
        }

        let size = self.unresolved.len().min(Self::FETCH_CHUNK);
        let handles: Vec<PimdirHandle> = self.unresolved.drain(..size).collect();
        debug!(
            "resolve {} new link ids at meta tier, {} left after them",
            handles.len(),
            self.unresolved.len(),
        );
        self.state = State::Fetching;
        PimdirCoroutineState::Yielded(PimdirYield::WantsFetch {
            collection: self.collection.clone(),
            handles,
            tier: PimdirTier::Meta,
        })
    }

    /// Builds the write batch, carrying old placements over by link id.
    ///
    /// Drops the old spine and upserts one placement per new member,
    /// carried when an old placement resolves to the same item and fresh
    /// otherwise.
    fn rebuild(&mut self) -> Vec<PimdirWriteOp> {
        let mut writes = Vec::new();
        let links = mem::take(&mut self.resolved);

        let (staged, old): (Vec<PimdirPlacement>, Vec<PimdirPlacement>) = mem::take(&mut self.old)
            .into_iter()
            .partition(|p| p.status == PimdirStatus::Created);

        let mut old_by_link: BTreeMap<PimdirLinkId, PimdirPlacement> = old
            .iter()
            .filter_map(|p| Some((p.link_id.clone()?, p.clone())))
            .collect();

        // NOTE: walked in handle order, since which copy of a hint keeps
        // the bare key decides what every other copy takes.
        let mut items = mem::take(&mut self.items);
        items.sort_by(|a, b| a.handle.cmp(&b.handle));
        items.dedup_by(|a, b| a.handle == b.handle);

        let mut written = BTreeSet::new();
        let mut carried_over = BTreeSet::new();
        // NOTE: seeded with the pending creates' keys, which stay taken
        // since the rebuild leaves those rows where they are.
        let mut claimed: BTreeSet<PimdirLinkId> = staged
            .into_iter()
            .filter_map(|placement| placement.link_id)
            .collect();
        for item in items {
            let resolved = links.get(&item.handle);
            let key = resolved.map(|resolved| {
                Self::key_of(&resolved.link_id, &item.handle, &claimed, &old_by_link)
            });
            let carried = key.as_ref().and_then(|key| old_by_link.remove(key));
            if let Some(key) = key.clone() {
                claimed.insert(key);
            }

            written.insert(item.handle.clone());
            match carried {
                Some(old) => {
                    carried_over.insert(old.handle.clone());
                    writes.push(PimdirWriteOp::UpsertPlacement(
                        self.carry(old, &item, resolved),
                    ));
                    self.report.rekeyed += 1;
                }
                None => {
                    writes.push(PimdirWriteOp::UpsertPlacement(
                        self.fresh(&item, resolved, key),
                    ));
                    self.report.pulled += 1;
                }
            }
        }

        for placement in &old {
            if carried_over.contains(&placement.handle) {
                continue;
            }
            let edited = matches!(
                placement.status,
                PimdirStatus::Dirty | PimdirStatus::Conflict
            ) && placement.object.is_some()
                && placement
                    .base
                    .as_ref()
                    .is_none_or(|b| b.object != placement.object);
            if edited {
                let mut resurrected = placement.clone();
                resurrected.status = PimdirStatus::Created;
                resurrected.conflict_revision = None;
                resurrected.conflict_object = None;
                resurrected.base = None;
                resurrected.origin = None;
                written.insert(resurrected.handle.clone());
                writes.push(PimdirWriteOp::UpsertPlacement(resurrected));
                carried_over.insert(placement.handle.clone());
                self.report.rekeyed += 1;
            }
        }
        self.report.dropped += old
            .iter()
            .filter(|p| p.status != PimdirStatus::Clean && !carried_over.contains(&p.handle))
            .count();

        // NOTE: never a handle this batch also upserts, or the storage's
        // apply order would decide. Rekeyed tells a storage sharing items
        // across sources that a renumbering is not a mass delete.
        for placement in &old {
            if written.contains(&placement.handle) {
                continue;
            }
            let reason = match carried_over.contains(&placement.handle) {
                true => PimdirDropReason::Rekeyed,
                false => PimdirDropReason::Deleted,
            };
            writes.push(PimdirWriteOp::DropPlacement {
                collection: self.collection.clone(),
                handle: placement.handle.clone(),
                reason,
            });
        }

        writes.push(PimdirWriteOp::SetCheckpoint {
            collection: self.collection.clone(),
            checkpoint: self.checkpoint.take().expect("an enumerated checkpoint"),
        });

        writes
    }

    /// The key a rebuilt member takes, and the old placement is found under.
    ///
    /// The hint while unclaimed, else the minted key an old copy already
    /// carries (the handle it was minted from is what the change took away),
    /// else a mint of the member's own, for a copy with no old row to carry.
    fn key_of(
        hint: &PimdirLinkId,
        handle: &PimdirHandle,
        claimed: &BTreeSet<PimdirLinkId>,
        old_by_link: &BTreeMap<PimdirLinkId, PimdirPlacement>,
    ) -> PimdirLinkId {
        if !claimed.contains(hint) {
            return hint.clone();
        }

        let minted = old_by_link
            .values()
            .filter(|old| old.link_id.as_ref() == Some(&hint.minted(&old.handle)))
            .min_by(|a, b| a.handle.cmp(&b.handle))
            .and_then(|old| old.link_id.clone());

        minted.unwrap_or_else(|| {
            hint.claim(handle, |key| {
                claimed.contains(key) || old_by_link.contains_key(key)
            })
        })
    }

    /// Carries an old placement onto the new handle.
    ///
    /// The cache survives and the flag delta re-derives against the new
    /// base. A revision the old base does not hold is a remote edit
    /// (SYNC §8): a placement holding a local edit is carried conflicted
    /// at it with its base untouched, a tombstone is revived as a sync
    /// would, and anything else is carried as the pull a sync would make,
    /// body dropped and base at the fetched revision.
    fn carry(
        &self,
        old: PimdirPlacement,
        item: &PimdirRemoteItem,
        resolved: Option<&Resolved>,
    ) -> PimdirPlacement {
        let old_base = old.base.as_ref();
        let old_base_flags = old_base.map(|b| b.flags.clone()).unwrap_or_default();
        let flags = PimdirFlags::merge(&old_base_flags, &old.flags, &item.flags);

        let observed = item
            .revision
            .clone()
            .or_else(|| resolved.and_then(|r| r.revision.clone()));
        let base_revision = old_base.and_then(|b| b.revision.clone());
        let remote_edited = observed.is_some() && observed != base_revision;
        let local_edit = old.object.is_some() && old_base.is_none_or(|b| b.object != old.object);

        let summary = resolved
            .and_then(|r| r.summary.clone())
            .or_else(|| old.summary.clone());
        let sort_key = resolved
            .map(|r| r.sort_key.clone())
            .unwrap_or_else(|| old.sort_key.clone());

        let mut carried = PimdirPlacement {
            collection: self.collection.clone(),
            handle: item.handle.clone(),
            link_id: old.link_id.clone(),
            object: old.object.clone(),
            level: old.level,
            summary,
            sort_key,
            flags,
            status: PimdirStatus::Clean,
            conflict_revision: None,
            conflict_object: None,
            base: Some(PimdirBase {
                flags: item.flags.clone(),
                revision: base_revision,
                object: old_base.and_then(|b| b.object.clone()),
            }),
            origin: old.origin.clone(),
        };

        let flags_pending = carried.flags != item.flags;
        match old.status {
            PimdirStatus::Conflict => {
                carried.status = PimdirStatus::Conflict;
                carried.conflict_revision = observed;
                // NOTE: the diverging body describes the revision recorded
                // beside it, so a newer one drops it and the upgrade pass
                // asks anew.
                if carried.conflict_revision == old.conflict_revision {
                    carried.conflict_object = old.conflict_object.clone();
                }
            }
            // NOTE: a delta never relists the member, so the revive a sync
            // would make on the next listing is made here (SYNC §5).
            PimdirStatus::Tombstone if remote_edited => {
                carried.object = None;
                carried.level = PimdirLevel::Probed;
                carried.flags = item.flags.clone();
                carried.origin = None;
                if let Some(base) = &mut carried.base {
                    base.revision = observed;
                    base.object = None;
                }
            }
            PimdirStatus::Tombstone => {
                carried.status = PimdirStatus::Tombstone;
            }
            _ if remote_edited && local_edit => {
                carried.status = PimdirStatus::Conflict;
                carried.conflict_revision = observed;
            }
            _ if remote_edited => {
                carried.object = None;
                carried.level = PimdirLevel::Probed;
                carried.status = match flags_pending {
                    true => PimdirStatus::Dirty,
                    false => PimdirStatus::Clean,
                };
                if let Some(base) = &mut carried.base {
                    base.revision = observed;
                    base.object = None;
                }
            }
            _ if local_edit && old.status == PimdirStatus::Dirty => {
                carried.status = PimdirStatus::Dirty;
            }
            _ => {
                carried.status = match flags_pending {
                    true => PimdirStatus::Dirty,
                    false => PimdirStatus::Clean,
                };
            }
        }

        carried
    }

    /// A fresh placement for a new member with no old counterpart.
    ///
    /// Carries the summary the meta fetch resolved, keyed under what
    /// [`key_of`](Self::key_of) settled for it.
    fn fresh(
        &self,
        item: &PimdirRemoteItem,
        resolved: Option<&Resolved>,
        key: Option<PimdirLinkId>,
    ) -> PimdirPlacement {
        PimdirPlacement {
            collection: self.collection.clone(),
            handle: item.handle.clone(),
            link_id: key,
            object: None,
            level: if resolved.is_some() {
                PimdirLevel::Meta
            } else {
                PimdirLevel::Probed
            },
            summary: resolved.and_then(|r| r.summary.clone()),
            sort_key: resolved.map(|r| r.sort_key.clone()).unwrap_or_default(),
            flags: item.flags.clone(),
            status: PimdirStatus::Clean,
            conflict_revision: None,
            conflict_object: None,
            base: Some(PimdirBase {
                flags: item.flags.clone(),
                revision: item.revision.clone(),
                object: None,
            }),
            origin: None,
        }
    }
}

impl PimdirCoroutine for PimdirRekey {
    type Yield = PimdirYield;
    type Return = Result<PimdirRekeyReport, PimdirArgError>;

    fn resume(
        &mut self,
        arg: Option<PimdirArg>,
    ) -> PimdirCoroutineState<Self::Yield, Self::Return> {
        match (&self.state, arg) {
            (State::Start, None) => {
                debug!("load local state from storage");
                self.state = State::Loading;
                PimdirCoroutineState::Yielded(PimdirYield::WantsLoad {
                    collection: self.collection.clone(),
                    scope: PimdirLoadScope::All,
                })
            }

            (State::Loading, Some(PimdirArg::Load(loaded))) => {
                self.old = loaded.placements;

                debug!("enumerate the new handle space in full");
                trace!("loaded {} old placements", self.old.len());
                self.state = State::Enumerating;
                PimdirCoroutineState::Yielded(PimdirYield::WantsEnumerate {
                    collection: self.collection.clone(),
                    cursor: None,
                })
            }

            (State::Enumerating, Some(PimdirArg::Enumerate(snapshot))) => {
                self.items = snapshot.items;
                self.checkpoint = Some(snapshot.checkpoint);

                if self.old.iter().any(|p| p.link_id.is_some()) {
                    self.unresolved = self.items.iter().map(|i| i.handle.clone()).collect();
                } else {
                    debug!("no link ids to match, rebuild the spine");
                }
                self.step()
            }

            (State::Fetching, Some(PimdirArg::Fetch(fetched))) => {
                for item in fetched {
                    self.resolved.insert(
                        item.handle,
                        Resolved {
                            link_id: item.link_id,
                            summary: item.summary,
                            sort_key: item.sort_key,
                            revision: item.revision,
                        },
                    );
                }
                self.step()
            }

            (State::Writing, Some(PimdirArg::Write)) => {
                debug!(
                    "rekey done: {} carried, {} pulled, {} pending dropped",
                    self.report.rekeyed, self.report.pulled, self.report.dropped,
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

/// What a meta fetch resolved for one new handle.
struct Resolved {
    link_id: PimdirLinkId,
    summary: Option<PimdirSummary>,
    sort_key: PimdirSortKey,
    revision: Option<String>,
}

/// What the coroutine is doing while it waits for the caller.
enum State {
    Start,
    Loading,
    Enumerating,
    Fetching,
    Writing,
    Done,
}

#[cfg(test)]
mod tests;
