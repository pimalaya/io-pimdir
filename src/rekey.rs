//! # Rekey coroutine
//!
//! Rebuilds a collection after a handle-space change (an IMAP UIDVALIDITY
//! bump), carrying local state over to the new handles by link id.
//!
//! A plain full sync would read every old handle as deleted upstream and
//! drop cached bodies and pending changes with them. The rebuild instead
//! enumerates the new spine, resolves its link ids at the meta tier, and
//! carries each old placement onto the new handle of the same item.
//!
//! The cache survives without a refetch, flag deltas re-derive against
//! the new base, tombstones keep their pending remove and staged edits
//! their body. An edit whose item found no new home survives as a
//! pending create; other unmatched pending state is dropped and counted.
//!
//! Pending creates are local staging, not spine, and stay untouched. The
//! carried base adopts the new observed revision, so a carried edit
//! pushes last-writer-wins on its first sync: the old revision chain is
//! gone with the old handles.
//!
//! Identity keys the match, so two copies of one identity stay two items:
//! a source reports the shared hint, never the minted key, so the first
//! copy in handle order takes the hint and the next is carried onto its
//! minted key. A row carried nowhere is dropped as the deletion it is.

use core::mem;

use alloc::{
    collections::{BTreeMap, BTreeSet},
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
    report: PimdirRekeyReport,
    state: State,
}

impl PimdirRekey {
    /// Creates a coroutine rebuilding `collection` onto its new handles.
    pub fn new(collection: impl Into<PimdirCollectionId>) -> Self {
        let collection = collection.into();
        debug!("rekey collection {}", collection.as_str());

        Self {
            collection,
            old: Vec::new(),
            items: Vec::new(),
            checkpoint: None,
            report: PimdirRekeyReport::default(),
            state: State::Start,
        }
    }

    /// Builds the write batch, carrying old placements over by link id.
    ///
    /// Drops the old spine and upserts one placement per new member,
    /// carried when an old placement resolves to the same item and fresh
    /// otherwise.
    fn rebuild(
        &mut self,
        links: BTreeMap<PimdirHandle, (PimdirLinkId, Option<PimdirSummary>, PimdirSortKey)>,
    ) -> Vec<PimdirWriteOp> {
        let mut writes = Vec::new();

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
            let key = resolved
                .map(|(hint, _, _)| Self::key_of(hint, &item.handle, &claimed, &old_by_link));
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
        // apply order would decide. Superseded tells a storage sharing
        // items across sources that a renumbering is not a mass delete.
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
    /// The cache survives, the flag delta re-derives against the new base
    /// and pending statuses stay pending.
    fn carry(
        &self,
        old: PimdirPlacement,
        item: &PimdirRemoteItem,
        resolved: Option<&(PimdirLinkId, Option<PimdirSummary>, PimdirSortKey)>,
    ) -> PimdirPlacement {
        let old_base_flags = old
            .base
            .as_ref()
            .map(|b| b.flags.clone())
            .unwrap_or_default();
        let flags = PimdirFlags::merge(&old_base_flags, &old.flags, &item.flags);

        let content_edit = old.status == PimdirStatus::Dirty
            && old.object.is_some()
            && old.base.as_ref().is_none_or(|b| b.object != old.object);
        let status = match old.status {
            PimdirStatus::Tombstone => PimdirStatus::Tombstone,
            PimdirStatus::Conflict => PimdirStatus::Conflict,
            _ if content_edit => PimdirStatus::Dirty,
            _ if flags != item.flags => PimdirStatus::Dirty,
            _ => PimdirStatus::Clean,
        };
        let conflict_revision = if status == PimdirStatus::Conflict {
            item.revision.clone()
        } else {
            None
        };

        // NOTE: the diverging body describes the revision recorded beside
        // it, so a newer one drops it and the upgrade pass asks anew.
        let conflict_object = match conflict_revision == old.conflict_revision {
            true => old.conflict_object.clone(),
            false => None,
        };

        PimdirPlacement {
            collection: self.collection.clone(),
            handle: item.handle.clone(),
            link_id: old.link_id.clone(),
            object: old.object.clone(),
            level: old.level,
            sort_key: resolved
                .map(|(_, _, key)| key.clone())
                .unwrap_or_else(|| old.sort_key.clone()),
            summary: resolved
                .and_then(|(_, summary, _)| summary.clone())
                .or_else(|| old.summary.clone()),
            flags,
            status,
            conflict_revision,
            conflict_object,
            base: Some(PimdirBase {
                flags: item.flags.clone(),
                revision: item.revision.clone(),
                object: old.base.as_ref().and_then(|b| b.object.clone()),
            }),
            origin: old.origin,
        }
    }

    /// A fresh placement for a new member with no old counterpart.
    ///
    /// Carries the summary the meta fetch resolved, keyed under what
    /// [`key_of`](Self::key_of) settled for it.
    fn fresh(
        &self,
        item: &PimdirRemoteItem,
        resolved: Option<&(PimdirLinkId, Option<PimdirSummary>, PimdirSortKey)>,
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
            summary: resolved.and_then(|(_, summary, _)| summary.clone()),
            sort_key: resolved.map(|(_, _, key)| key.clone()).unwrap_or_default(),
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
                self.state = State::PendingLoad;
                PimdirCoroutineState::Yielded(PimdirYield::WantsLoad {
                    collection: self.collection.clone(),
                    scope: PimdirLoadScope::All,
                })
            }

            (State::PendingLoad, Some(PimdirArg::Load(loaded))) => {
                self.old = loaded.placements;

                debug!("enumerate the new handle space in full");
                trace!("loaded {} old placements", self.old.len());
                self.state = State::PendingEnumerate;
                PimdirCoroutineState::Yielded(PimdirYield::WantsEnumerate {
                    collection: self.collection.clone(),
                    cursor: None,
                })
            }

            (State::PendingEnumerate, Some(PimdirArg::Enumerate(snapshot))) => {
                self.items = snapshot.items;
                self.checkpoint = Some(snapshot.checkpoint);

                if !self.old.iter().any(|p| p.link_id.is_some()) {
                    debug!("no link ids to match, rebuild the spine");
                    self.state = State::PendingWrite;
                    let writes = self.rebuild(BTreeMap::new());
                    return PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(writes));
                }

                let handles: Vec<PimdirHandle> =
                    self.items.iter().map(|i| i.handle.clone()).collect();
                debug!("resolve {} new link ids at meta tier", handles.len());
                self.state = State::PendingFetch;
                PimdirCoroutineState::Yielded(PimdirYield::WantsFetch {
                    collection: self.collection.clone(),
                    handles,
                    tier: PimdirTier::Meta,
                })
            }

            (State::PendingFetch, Some(PimdirArg::Fetch(fetched))) => {
                let links: BTreeMap<
                    PimdirHandle,
                    (PimdirLinkId, Option<PimdirSummary>, PimdirSortKey),
                > = fetched
                    .into_iter()
                    .map(|f| (f.handle, (f.link_id, f.summary, f.sort_key)))
                    .collect();

                trace!("resolved {} link ids", links.len());
                self.state = State::PendingWrite;
                let writes = self.rebuild(links);
                PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(writes))
            }

            (State::PendingWrite, Some(PimdirArg::Write)) => {
                debug!(
                    "rekey done: {} carried, {} pulled, {} pending dropped",
                    self.report.rekeyed, self.report.pulled, self.report.dropped,
                );
                self.state = State::Done;
                PimdirCoroutineState::Complete(Ok(self.report))
            }

            (_, Some(_)) => PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)),
            (_, None) => PimdirCoroutineState::Complete(Err(PimdirArgError::MissingArg)),
        }
    }
}

enum State {
    Start,
    PendingLoad,
    PendingEnumerate,
    PendingFetch,
    PendingWrite,
    Done,
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use crate::{
        load::PimdirLoaded,
        object::PimdirHash,
        placement::PimdirOrigin,
        rekey::*,
        remote::{PimdirFetchedItem, PimdirRemoteSnapshot},
    };

    /// An old-spine placement, synced clean at base `flags`.
    fn synced(handle: &str, link: &str, flags: &[&str]) -> PimdirPlacement {
        PimdirPlacement {
            sort_key: Default::default(),
            collection: "inbox".into(),
            handle: PimdirHandle::from(handle),
            link_id: Some(PimdirLinkId::from(link)),
            object: None,
            level: PimdirLevel::Meta,
            summary: Some(crate::summary::stub("row")),
            flags: PimdirFlags::from_iter(flags.iter().copied()),
            status: PimdirStatus::Clean,
            conflict_revision: None,
            conflict_object: None,
            base: Some(PimdirBase {
                flags: PimdirFlags::from_iter(flags.iter().copied()),
                revision: None,
                object: None,
            }),
            origin: None,
        }
    }

    fn item(handle: &str, flags: &[&str]) -> PimdirRemoteItem {
        PimdirRemoteItem {
            handle: PimdirHandle::from(handle),
            flags: PimdirFlags::from_iter(flags.iter().copied()),
            revision: None,
        }
    }

    fn fetched(handle: &str, link: &str) -> PimdirFetchedItem {
        PimdirFetchedItem {
            sort_key: Default::default(),
            handle: PimdirHandle::from(handle),
            link_id: PimdirLinkId::from(link),
            summary: Some(crate::summary::stub("fresh row")),
            body: None,
            revision: None,
        }
    }

    /// Runs a rekey over an old spine, a new spine and its meta replies.
    fn run(
        old: Vec<PimdirPlacement>,
        items: Vec<PimdirRemoteItem>,
        metas: Vec<PimdirFetchedItem>,
    ) -> (Vec<PimdirWriteOp>, PimdirRekeyReport) {
        crate::testlog::init();
        let mut rekey = PimdirRekey::new("inbox");
        let _ = rekey.resume(None);
        let _ = rekey.resume(Some(PimdirArg::Load(PimdirLoaded {
            placements: old,
            checkpoint: None,
        })));

        let snapshot = PimdirRemoteSnapshot {
            items,
            vanished: Vec::new(),
            complete: true,
            checkpoint: PimdirCheckpoint(b"v2".to_vec()),
        };
        let writes = match rekey.resume(Some(PimdirArg::Enumerate(snapshot))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsFetch { tier, .. }) => {
                assert_eq!(tier, PimdirTier::Meta);
                match rekey.resume(Some(PimdirArg::Fetch(metas))) {
                    PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(w)) => w,
                    state => panic!("expected WantsWrite, got {state:?}"),
                }
            }
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(w)) => w,
            state => panic!("expected fetch or write, got {state:?}"),
        };

        let report = match rekey.resume(Some(PimdirArg::Write)) {
            PimdirCoroutineState::Complete(Ok(report)) => report,
            state => panic!("expected Complete(Ok), got {state:?}"),
        };
        (writes, report)
    }

    fn upserted<'a>(writes: &'a [PimdirWriteOp], handle: &str) -> Option<&'a PimdirPlacement> {
        writes.iter().find_map(|w| match w {
            PimdirWriteOp::UpsertPlacement(p) if p.handle.as_str() == handle => Some(p),
            _ => None,
        })
    }

    /// The handles the batch both drops and writes.
    ///
    /// The one way a rekey can depend on the order a storage applies it.
    fn dropped_and_upserted(writes: &[PimdirWriteOp]) -> Vec<&str> {
        writes
            .iter()
            .filter_map(|w| match w {
                PimdirWriteOp::DropPlacement { handle, .. } => Some(handle.as_str()),
                _ => None,
            })
            .filter(|handle| upserted(writes, handle).is_some())
            .collect()
    }

    /// The common case: a server renumbering into the same handle range.
    #[test]
    fn a_reused_handle_is_not_dropped_by_the_batch_that_writes_it() {
        let old = synced("1", "a", &[]);
        let (writes, report) = run(vec![old], vec![item("1", &[])], vec![fetched("1", "a")]);

        assert_eq!(report.rekeyed, 1, "the item is carried over");
        assert!(
            upserted(&writes, "1").is_some(),
            "the new spine holds it: {writes:?}",
        );
        assert_eq!(
            dropped_and_upserted(&writes),
            Vec::<&str>::new(),
            "the batch decides by apply order: {writes:?}",
        );
    }

    /// Same hazard without reuse: the edit resurrects under its old handle.
    #[test]
    fn a_resurrected_edit_is_not_dropped_by_the_batch_that_writes_it() {
        let mut old = synced("1", "a", &[]);
        old.status = PimdirStatus::Dirty;
        old.object = Some(PimdirHash::from("h2"));
        old.level = PimdirLevel::Full;
        old.base = Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: None,
            object: Some(PimdirHash::from("h1")),
        });

        let (writes, _report) = run(
            vec![old],
            vec![item("101", &[])],
            vec![fetched("101", "other")],
        );

        let resurrected = upserted(&writes, "1").expect("the edit survives as a create");
        assert_eq!(resurrected.status, PimdirStatus::Created);
        assert_eq!(
            dropped_and_upserted(&writes),
            Vec::<&str>::new(),
            "the local edit survives only if the drop is applied first: {writes:?}",
        );
    }

    #[test]
    fn a_pending_flag_delta_survives_the_bump() {
        let mut old = synced("1", "msg-a", &["seen"]);
        old.flags = PimdirFlags::from_iter(["seen", "flagged"]);
        old.status = PimdirStatus::Dirty;

        let (writes, report) = run(
            vec![old],
            vec![item("101", &["seen"])],
            vec![fetched("101", "msg-a")],
        );

        assert_eq!(report.rekeyed, 1);
        assert_eq!(report.dropped, 0);
        assert!(
            writes.iter().any(
                |w| matches!(w, PimdirWriteOp::DropPlacement { handle, .. } if handle.as_str() == "1")
            ),
            "the old handle is dropped: {writes:?}",
        );
        let carried = upserted(&writes, "101").expect("a carried placement");
        assert_eq!(
            carried.status,
            PimdirStatus::Dirty,
            "the delta stays pending"
        );
        assert!(carried.flags.contains("flagged"));
        let base = carried.base.as_ref().expect("a base");
        assert!(base.flags.contains("seen") && !base.flags.contains("flagged"));
    }

    #[test]
    fn a_tombstone_survives_with_its_destination() {
        let mut old = synced("1", "msg-a", &["seen"]);
        old.status = PimdirStatus::Tombstone;
        old.origin = Some(PimdirOrigin {
            collection: "archive".into(),
            handle: PimdirHandle::from("1"),
        });

        let (writes, report) = run(
            vec![old],
            vec![item("101", &["seen"])],
            vec![fetched("101", "msg-a")],
        );

        assert_eq!(report.rekeyed, 1);
        let carried = upserted(&writes, "101").expect("a carried placement");
        assert_eq!(carried.status, PimdirStatus::Tombstone);
        assert_eq!(
            carried.origin.as_ref().expect("a move target").collection,
            "archive".into(),
        );
    }

    #[test]
    fn a_staged_edit_survives_with_its_body() {
        let mut old = synced("1", "msg-a", &[]);
        old.object = Some(PimdirHash::from("h2"));
        old.level = PimdirLevel::Full;
        old.status = PimdirStatus::Dirty;

        let (writes, report) = run(
            vec![old],
            vec![item("101", &[])],
            vec![fetched("101", "msg-a")],
        );

        assert_eq!(report.rekeyed, 1);
        let carried = upserted(&writes, "101").expect("a carried placement");
        assert_eq!(carried.status, PimdirStatus::Dirty);
        assert_eq!(
            carried.object,
            Some(PimdirHash::from("h2")),
            "the body survives"
        );
        assert_eq!(carried.level, PimdirLevel::Full, "the cache survives");
    }

    #[test]
    fn a_clean_cache_carries_over_without_pending_state() {
        let mut old = synced("1", "msg-a", &["seen"]);
        old.object = Some(PimdirHash::from("h1"));
        old.base.as_mut().expect("a base").object = Some(PimdirHash::from("h1"));
        old.level = PimdirLevel::Full;

        let (writes, report) = run(
            vec![old],
            vec![item("101", &["seen"])],
            vec![fetched("101", "msg-a")],
        );

        assert_eq!(report.rekeyed, 1);
        let carried = upserted(&writes, "101").expect("a carried placement");
        assert_eq!(carried.status, PimdirStatus::Clean);
        assert_eq!(carried.object, Some(PimdirHash::from("h1")));
        assert_eq!(carried.level, PimdirLevel::Full);
        let base = carried.base.as_ref().expect("a base");
        assert_eq!(base.object, Some(PimdirHash::from("h1")));
    }

    #[test]
    fn an_unmatched_staged_edit_resurrects_as_a_pending_create() {
        let mut old = synced("1", "msg-a", &[]);
        old.object = Some(PimdirHash::from("h2"));
        old.level = PimdirLevel::Full;
        old.status = PimdirStatus::Dirty;

        let (writes, report) = run(vec![old], vec![], vec![]);

        assert_eq!(report.rekeyed, 1, "carried as a pending create");
        assert_eq!(report.dropped, 0);
        let resurrected = upserted(&writes, "1").expect("a resurrected placement");
        assert_eq!(resurrected.status, PimdirStatus::Created);
        assert!(resurrected.base.is_none());
        assert_eq!(resurrected.object, Some(PimdirHash::from("h2")));
    }

    /// A probed-only placement has no link id to match on.
    #[test]
    fn unmatched_pending_state_is_dropped_and_counted() {
        let mut old = synced("1", "msg-a", &[]);
        old.link_id = None;
        old.flags = PimdirFlags::from_iter(["flagged"]);
        old.status = PimdirStatus::Dirty;

        let (writes, report) = run(vec![old], vec![item("101", &[])], vec![]);

        assert_eq!(report.rekeyed, 0);
        assert_eq!(report.pulled, 1);
        assert_eq!(report.dropped, 1, "the pending edit is lost, and said so");
        let fresh = upserted(&writes, "101").expect("a fresh placement");
        assert_eq!(fresh.status, PimdirStatus::Clean);
    }

    #[test]
    fn no_link_ids_skips_the_meta_fetch() {
        let mut old = synced("1", "msg-a", &[]);
        old.link_id = None;

        let mut rekey = PimdirRekey::new("inbox");
        let _ = rekey.resume(None);
        let _ = rekey.resume(Some(PimdirArg::Load(PimdirLoaded {
            placements: vec![old],
            checkpoint: None,
        })));

        let snapshot = PimdirRemoteSnapshot {
            items: vec![item("101", &[])],
            vanished: Vec::new(),
            complete: true,
            checkpoint: PimdirCheckpoint(b"v2".to_vec()),
        };
        match rekey.resume(Some(PimdirArg::Enumerate(snapshot))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(_)) => {}
            state => panic!("expected WantsWrite without a fetch, got {state:?}"),
        }
    }

    #[test]
    fn pending_creates_are_left_untouched() {
        let mut placeholder = synced("tmp-1", "msg-b", &[]);
        placeholder.status = PimdirStatus::Created;
        placeholder.base = None;

        let (writes, report) = run(vec![placeholder], vec![], vec![]);

        assert_eq!(report.rekeyed + report.pulled + report.dropped, 0);
        assert!(
            !writes.iter().any(|w| matches!(
                w,
                PimdirWriteOp::DropPlacement { handle, .. } if handle.as_str() == "tmp-1"
            )),
            "the placeholder is not spine, it stays: {writes:?}",
        );
    }

    #[test]
    fn missing_arg_errors() {
        let mut rekey = PimdirRekey::new("inbox");
        let _ = rekey.resume(None);
        match rekey.resume(None) {
            PimdirCoroutineState::Complete(Err(PimdirArgError::MissingArg)) => {}
            state => panic!("expected MissingArg, got {state:?}"),
        }
    }

    /// An empty report would pass for a run that did nothing.
    #[test]
    fn a_completed_rekey_does_not_resume() {
        let mut rekey = PimdirRekey::new("inbox");
        let _ = rekey.resume(None);
        let _ = rekey.resume(Some(PimdirArg::Load(PimdirLoaded::default())));
        let _ = rekey.resume(Some(PimdirArg::Enumerate(PimdirRemoteSnapshot {
            items: Vec::new(),
            vanished: Vec::new(),
            complete: true,
            checkpoint: PimdirCheckpoint(b"v2".to_vec()),
        })));
        let _ = rekey.resume(Some(PimdirArg::Write));

        match rekey.resume(Some(PimdirArg::Write)) {
            PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => {}
            state => panic!("expected UnexpectedArg, got {state:?}"),
        }
    }

    #[test]
    fn unexpected_arg_errors() {
        let mut rekey = PimdirRekey::new("inbox");
        let _ = rekey.resume(None);
        match rekey.resume(Some(PimdirArg::Write)) {
            PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => {}
            state => panic!("expected UnexpectedArg, got {state:?}"),
        }
    }

    /// A minted key is a key: the rebuild reads nothing into its shape.
    #[test]
    fn a_minted_key_is_carried_over_a_handle_space_change() {
        let mut minted = synced("8", "dup:m1#8", &[]);
        minted.flags = PimdirFlags::from_iter(["seen"]);
        minted.status = PimdirStatus::Dirty;

        let (writes, report) = run(
            vec![synced("7", "m1", &[]), minted],
            vec![item("v2-0", &[]), item("v2-1", &[])],
            vec![fetched("v2-0", "m1"), fetched("v2-1", "dup:m1#8")],
        );

        assert_eq!(report.rekeyed, 2);
        assert_eq!(report.dropped, 0, "no pending state was lost");
        let carried = |handle: &str| {
            writes
                .iter()
                .find_map(|op| match op {
                    PimdirWriteOp::UpsertPlacement(p) if p.handle.as_str() == handle => Some(p),
                    _ => None,
                })
                .expect("the carried placement")
                .clone()
        };

        assert_eq!(carried("v2-0").link_id, Some(PimdirLinkId::from("m1")));
        let copy = carried("v2-1");
        assert_eq!(copy.link_id, Some(PimdirLinkId::from("dup:m1#8")));
        assert_eq!(
            copy.status,
            PimdirStatus::Dirty,
            "and its pending push survives the rebuild like any other",
        );
    }

    /// Both copies resolve to the shared hint, never to the minted key.
    #[test]
    fn two_copies_of_one_hint_are_carried_apart() {
        let mut first = synced("7", "m1", &[]);
        first.object = Some(PimdirHash::from("h1"));
        first.level = PimdirLevel::Full;
        let mut second = synced("8", "dup:m1#8", &[]);
        second.object = Some(PimdirHash::from("h2"));
        second.level = PimdirLevel::Full;

        let (writes, report) = run(
            vec![first, second],
            vec![item("v2-0", &[]), item("v2-1", &[])],
            vec![fetched("v2-0", "m1"), fetched("v2-1", "m1")],
        );

        assert_eq!(report.rekeyed, 2, "both copies are carried, not merged");
        assert_eq!(report.pulled, 0);
        let carried = |handle: &str| upserted(&writes, handle).expect("a carried placement");
        assert_eq!(carried("v2-0").link_id, Some(PimdirLinkId::from("m1")));
        assert_eq!(carried("v2-0").object, Some(PimdirHash::from("h1")));
        assert_eq!(
            carried("v2-1").link_id,
            Some(PimdirLinkId::from("dup:m1#8")),
            "the second copy keeps the key it was minted under",
        );
        assert_eq!(
            carried("v2-1").object,
            Some(PimdirHash::from("h2")),
            "with its own body, which is the copy nobody would see again",
        );
    }

    /// So a store rebuilt from scratch converges on the same keys.
    #[test]
    fn a_new_copy_of_one_hint_is_minted_from_its_own_handle() {
        let (writes, _report) = run(
            vec![synced("7", "m1", &[])],
            vec![item("v2-0", &[]), item("v2-1", &[])],
            vec![fetched("v2-0", "m1"), fetched("v2-1", "m1")],
        );

        let minted = upserted(&writes, "v2-1").expect("the second copy");
        assert_eq!(minted.link_id, Some(PimdirLinkId::from("dup:m1#v2-1")));
    }

    #[test]
    fn a_rebuild_leaves_a_pending_create_its_key() {
        let mut placeholder = synced("tmp-1", "m1", &[]);
        placeholder.status = PimdirStatus::Created;
        placeholder.base = None;

        let (writes, _report) = run(
            vec![placeholder],
            vec![item("v2-0", &[])],
            vec![fetched("v2-0", "m1")],
        );

        let member = upserted(&writes, "v2-0").expect("the rebuilt member");
        assert_eq!(member.link_id, Some(PimdirLinkId::from("dup:m1#v2-0")));
    }

    /// A storage sharing items across sources has to tell the two apart.
    #[test]
    fn a_rebuild_says_which_rows_are_gone_and_which_moved() {
        let (writes, _report) = run(
            vec![synced("7", "m1", &[]), synced("8", "m2", &[])],
            vec![item("v2-0", &[])],
            vec![fetched("v2-0", "m1")],
        );

        let dropped: Vec<(&str, PimdirDropReason)> = writes
            .iter()
            .filter_map(|op| match op {
                PimdirWriteOp::DropPlacement { handle, reason, .. } => {
                    Some((handle.as_str(), *reason))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            dropped,
            vec![
                ("7", PimdirDropReason::Rekeyed),
                ("8", PimdirDropReason::Deleted),
            ],
        );
    }
}
