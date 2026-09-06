//! # Sync coroutine
//!
//! I/O-free coroutine reconciling one collection with its remote
//! (SYNC §5): it loads local state, enumerates the remote delta, then
//! three-way merges local, base and remote per placement.
//!
//! The merge compares per-placement identities (the flag set, a content
//! revision), never raw bytes. Flags merge element-wise and never
//! conflict, only divergent content edits do. An edit beats a delete in
//! both directions, and a push is confirmed before local state changes.
//!
//! Mutable-content backends report a content revision and can conflict.
//! Immutable ones report none, so they only ever merge flags and
//! membership, through the same merge shape.

use core::mem;

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
    vec::Vec,
};

use log::{debug, trace};

use crate::{
    change::{PimdirChange, PimdirChangeKind, PimdirDropReason, PimdirWriteOp},
    collection::{PimdirCheckpoint, PimdirCollectionId},
    coroutine::*,
    load::PimdirLoadScope,
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirSortKey, PimdirStatus,
    },
    remote::{PimdirPushOutcome, PimdirPushResult, PimdirRemoteItem, PimdirRemoteSnapshot},
    sync::join::{Candidate, Join, Merge},
};

mod join;

/// Which push kinds a writable source may derive.
///
/// Each kind is independent, so a source can accept flag changes but
/// refuse deletes. All permitted by default, and only consulted when
/// [`PimdirSyncOptions::push`] is true: a false `push` is read-only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PimdirPushRights {
    /// May push a flag-set change.
    pub flags: bool,
    /// May push an in-place content update.
    pub content: bool,
    /// May push a membership add (a create: copy, move target or append).
    pub add: bool,
    /// May push a membership remove (a delete or move source).
    pub remove: bool,
}

impl PimdirPushRights {
    /// Every push kind permitted (the default).
    pub const fn all() -> Self {
        Self {
            flags: true,
            content: true,
            add: true,
            remove: true,
        }
    }

    /// No push kind permitted.
    pub const fn none() -> Self {
        Self {
            flags: false,
            content: false,
            add: false,
            remove: false,
        }
    }
}

impl Default for PimdirPushRights {
    fn default() -> Self {
        Self::all()
    }
}

/// How a sync resolves a content conflict, content diverged on both sides.
///
/// Only mutable-content backends can conflict, immutable content reports
/// no revision. A base-less create-collision is always kept as a
/// conflict: with no shared ancestor there is nothing to automate.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PimdirConflictPolicy {
    /// Leave the placement conflicted for the consumer to edit (the default).
    #[default]
    Manual,
    /// Keep the local edit, overwriting the remote's current version.
    PreferLocal,
    /// Keep the remote edit, dropping the local one and pulling the remote.
    PreferRemote,
}

/// Tuning for one sync run: the push direction and the enumerate depth.
///
/// A local delete the source's rights forbid follows no option (SYNC §5):
/// it is held when the item has another binding, whose source pushes the
/// delete, and reverted when this binding is its last, which
/// [`beside_other_sources`](PimdirSync::beside_other_sources) tells the
/// coroutine from what the store knows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PimdirSyncOptions {
    /// The master push switch: when false the source is read-only.
    pub push: bool,
    /// Per-kind refinement of `push`, consulted only when `push` is true.
    pub rights: PimdirPushRights,
    /// How a content conflict is resolved.
    pub conflict: PimdirConflictPolicy,
    /// Whether to ignore the checkpoint and enumerate the whole remote.
    ///
    /// The recovery path for a replica that drifted: the merge reconciles
    /// the complete spine, re-adding missing members and dropping
    /// phantoms.
    pub full: bool,
}

impl Default for PimdirSyncOptions {
    fn default() -> Self {
        Self {
            push: true,
            rights: PimdirPushRights::all(),
            conflict: PimdirConflictPolicy::Manual,
            full: false,
        }
    }
}

/// A per-item outcome of a sync (SYNC §5): what the remote changed
/// locally, a divergence, and an accepted add. A pushed local change
/// reports nothing, the consumer having made it.
///
/// Emitted in order as the merge touches each handle; the counters on
/// [`PimdirSyncReport`] summarise them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirSyncEvent {
    /// A new member appeared locally, pulled from the remote.
    Added(PimdirHandle),
    /// A placement's flag set changed, pulled from the remote.
    FlagsChanged(PimdirHandle),
    /// A placement's body was dropped after a remote content change.
    ContentChanged(PimdirHandle),
    /// A member was removed after a remote delete.
    Vanished(PimdirHandle),
    /// A placement's content diverged on both sides and is left conflicted.
    Conflicted(PimdirHandle),
    /// A local create (copy, move target or append) the remote accepted.
    Created(PimdirHandle),
}

/// What a sync did.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirSyncReport {
    /// Placements changed by pulling the remote.
    pub pulled: usize,
    /// Changes the remote accepted.
    pub pushed: usize,
    /// Placements left in conflict (content diverged on both sides).
    pub conflicts: usize,
    /// Pushes the remote rejected on optimistic concurrency.
    pub rejected: usize,
    /// Placements whose stale body was dropped after a remote content change.
    pub refreshed: usize,
    /// The per-item events this sync emitted, in order.
    pub events: Vec<PimdirSyncEvent>,
}

/// I/O-free SYNC coroutine.
pub struct PimdirSync {
    collection: PimdirCollectionId,
    opts: PimdirSyncOptions,
    /// Whether the collection is bound by other sources too, which decides
    /// a refused delete (SYNC §5): held beside others, reverted alone.
    beside_others: bool,
    local: BTreeMap<PimdirHandle, PimdirPlacement>,
    checkpoint: Option<PimdirCheckpoint>,
    /// Whether the collection holds a probe of this source, at load or in
    /// the enumeration: a create is not derived while it does (SYNC §5).
    holds_probes: bool,
    /// The merge in progress, from the enumerate to the join's last candidate.
    merging: Option<Merge>,
    writes: Vec<PimdirWriteOp>,
    /// The derived changes no chunk has taken yet, in derivation order.
    pushes: Vec<PimdirChange>,
    /// The checkpoint the enumerate reported, held for the last write.
    ///
    /// No intermediate write carries it: it lands once every chunk is
    /// recorded.
    next_checkpoint: Option<PimdirCheckpoint>,
    /// What each derived push does to its row once accepted, by handle.
    ///
    /// One handle yields at most one change, and the flag axis refreshes
    /// the entry whenever it writes the handle, so an accepted push
    /// rebases the row the merge last wrote (SYNC §5).
    pending: BTreeMap<PimdirHandle, Pending>,
    /// The entries of the chunk awaiting its outcome.
    ///
    /// Only these are settled or forgotten when the outcomes land: a
    /// later chunk's are still waiting for their own.
    in_flight: BTreeMap<PimdirHandle, Pending>,
    report: PimdirSyncReport,
    state: State,
}

impl PimdirSync {
    /// How many changes one push chunk holds.
    ///
    /// Each chunk's outcomes are recorded before the next is pushed, so
    /// an interrupted run replays at most this many. It bounds a crash
    /// window, not throughput, hence an engine constant.
    pub const PUSH_CHUNK: usize = 64;

    /// How many storage writes one batch holds before the merge hands it over.
    ///
    /// Bounds memory rather than a crash window: a lost batch costs a free
    /// re-merge. A floor, not a ceiling, since it never cuts through the
    /// writes of one candidate.
    pub const WRITE_CHUNK: usize = 1024;

    /// Creates a coroutine that reconciles `collection`.
    pub fn new(collection: impl Into<PimdirCollectionId>, opts: PimdirSyncOptions) -> Self {
        let collection = collection.into();
        debug!(
            "sync collection {} (push={})",
            collection.as_str(),
            opts.push
        );

        Self {
            collection,
            opts,
            beside_others: false,
            local: BTreeMap::new(),
            checkpoint: None,
            holds_probes: false,
            merging: None,
            writes: Vec::new(),
            pushes: Vec::new(),
            next_checkpoint: None,
            pending: BTreeMap::new(),
            in_flight: BTreeMap::new(),
            report: PimdirSyncReport::default(),
            state: State::Start,
        }
    }

    /// Tells the coroutine whether other sources bind the collection, so
    /// a delete the rights forbid is held rather than reverted (SYNC §5):
    /// reverted beside another source it would read as a resurrection
    /// there. The std client answers it from the store's bindings.
    pub fn beside_other_sources(mut self, beside: bool) -> Self {
        self.beside_others = beside;
        self
    }

    /// Yields the next push chunk, or the write recording the last one.
    fn step(
        &mut self,
    ) -> PimdirCoroutineState<PimdirYield, Result<PimdirSyncReport, PimdirArgError>> {
        if self.pushes.is_empty() {
            debug!("write {} storage ops", self.writes.len());
            self.state = State::Writing;
            let batch = self.batch();
            return PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(batch));
        }

        let size = self.pushes.len().min(Self::PUSH_CHUNK);
        let changes: Vec<PimdirChange> = self.pushes.drain(..size).collect();
        self.in_flight = changes
            .iter()
            .filter_map(|change| self.pending.remove_entry(change.handle()))
            .collect();

        debug!(
            "push {} local changes to remote, {} left after them",
            changes.len(),
            self.pushes.len(),
        );
        trace!("changes: {changes:?}");
        self.state = State::Pushing;
        PimdirCoroutineState::Yielded(PimdirYield::WantsPush {
            collection: self.collection.clone(),
            changes,
        })
    }

    /// Takes the accumulated writes, plus the checkpoint after the last chunk.
    ///
    /// The checkpoint stays the pre-push one, so the next delta re-lists
    /// the engine's own echo. An intermediate chunk must not carry it, or
    /// a crashed run would resume past its unrecorded pushes.
    fn batch(&mut self) -> Vec<PimdirWriteOp> {
        let mut batch = mem::take(&mut self.writes);

        if self.pushes.is_empty()
            && let Some(checkpoint) = self.next_checkpoint.take()
        {
            batch.push(PimdirWriteOp::SetCheckpoint {
                collection: self.collection.clone(),
                checkpoint,
            });
        }

        batch
    }

    /// Opens the three-way merge over what the enumerate reported.
    ///
    /// The local placements are moved into the join: nothing else reads
    /// them, so the merge owns each one instead of cloning it per
    /// candidate.
    fn open_merge(&mut self, snapshot: PimdirRemoteSnapshot) {
        let PimdirRemoteSnapshot {
            mut items,
            vanished,
            complete,
            checkpoint,
        } = snapshot;

        // NOTE: the join walks both sides in handle order and pairs each
        // handle once, so the snapshot is sorted and deduplicated first.
        if !items.is_sorted_by(|a, b| a.handle <= b.handle) {
            debug!("enumeration is not ordered by handle, sorting it");
            items.sort_by(|a, b| a.handle.cmp(&b.handle));
        }
        items.dedup_by(|a, b| a.handle == b.handle);

        let vanished: BTreeSet<PimdirHandle> = vanished.into_iter().collect();
        let local = mem::take(&mut self.local);

        // NOTE: a probe may be a pending create's own arrival, so no
        // create is pushed while one exists (SYNC §5): the ones loaded,
        // and the members this enumeration lists that nothing binds yet.
        self.holds_probes = local.values().any(is_probe)
            || items.iter().any(|item| !local.contains_key(&item.handle));

        self.merging = Some(Merge {
            join: Join::new(local, items),
            vanished,
            complete,
            checkpoint,
        });
    }

    /// Merges candidates until the write batch is full or the join runs out.
    fn merge_step(
        &mut self,
    ) -> PimdirCoroutineState<PimdirYield, Result<PimdirSyncReport, PimdirArgError>> {
        while let Some(candidate) = self.next_candidate() {
            if let Some(kind) = self.merge(candidate) {
                self.pushes.push(PimdirChange::new(&self.collection, kind));
            }

            // NOTE: cut between candidates, never inside one: the writes
            // one candidate derives are consistent only together.
            if self.writes.len() >= Self::WRITE_CHUNK {
                self.state = State::Merging;
                let batch = self.batch();
                return PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(batch));
            }
        }

        let merge = self.merging.take().expect("a merge in progress");
        self.next_checkpoint = Some(merge.checkpoint);
        debug!(
            "reconciled: {} pulled, {} conflicts, {} changes to push",
            self.report.pulled,
            self.report.conflicts,
            self.pushes.len(),
        );

        self.step()
    }

    /// The next handle to merge, or `None` once the join is exhausted.
    ///
    /// A complete snapshot merges every handle either side holds. A delta
    /// merges the changed or vanished ones, plus every locally non-clean
    /// handle, whose pending push it would otherwise never revisit.
    fn next_candidate(&mut self) -> Option<Candidate> {
        let merge = self.merging.as_mut()?;

        loop {
            let candidate = merge.join.next()?;

            if merge.complete {
                return Some(candidate);
            }
            if let Some(candidate) = merge.narrow(candidate) {
                return Some(candidate);
            }
        }
    }

    /// Three-way merges one candidate, returning a push if the local side won.
    ///
    /// Membership arms come first: an edit beats a delete either way, a
    /// delete is held until confirmed, and a create waits for its assigned
    /// handle. Present on both, content reconciles before flags.
    fn merge(&mut self, candidate: Candidate) -> Option<PimdirChangeKind> {
        let Candidate {
            handle,
            local,
            remote: remote_item,
        } = candidate;

        let based = local.as_ref().map(|p| p.base.is_some()).unwrap_or(false);

        let local_tombstone = local
            .as_ref()
            .map(|p| p.status == PimdirStatus::Tombstone)
            .unwrap_or(false);
        let local_present = local.is_some() && !local_tombstone;
        let remote_present = remote_item.is_some();

        match (local_present, based, remote_present) {
            (false, true, true) if local_tombstone => {
                let local = local.expect("local present");
                let item = remote_item.as_ref().expect("remote present");

                let base_revision = local.base.as_ref().and_then(|b| b.revision.clone());
                if item.revision.is_some() && item.revision != base_revision {
                    // NOTE: a tombstone holding a local edit too is the
                    // both-changed case and follows the policy (SYNC §5);
                    // one holding none is revived and pulled.
                    if local.staged_edit().is_some() {
                        let mut live = local;
                        live.status = PimdirStatus::Dirty;
                        live.origin = None;
                        return match self.resolve_conflict(&live, item) {
                            ContentOutcome::Push(change) => {
                                self.reconcile_flags(&live, item, PushFlags::Withhold);
                                Some(change)
                            }
                            ContentOutcome::Rewritten(rewritten) => {
                                self.reconcile_flags(&rewritten, item, PushFlags::Derive)
                            }
                            ContentOutcome::Untouched => {
                                self.reconcile_flags(&live, item, PushFlags::Derive)
                            }
                        };
                    }
                    self.revive(&local, item);
                    self.report.pulled += 1;
                    self.emit(PimdirSyncEvent::Added(handle.clone()));
                    return None;
                }

                // NOTE: a staged edit rides ahead of the move, which derives
                // again once the base holds the pushed content.
                if local.staged_edit().is_some() && local.origin.is_some() {
                    if !(self.opts.push && self.opts.rights.content) {
                        return self.refuse_delete(local);
                    }
                    let object = local.object.clone().expect("a staged edited body");
                    self.pending.insert(handle.clone(), Pending::Content(local));
                    return Some(PimdirChangeKind::Update {
                        handle,
                        object,
                        if_match: base_revision,
                    });
                }

                if !(self.opts.push && self.opts.rights.remove) {
                    return self.refuse_delete(local);
                }

                self.pending.insert(handle.clone(), Pending::Remove);
                let to = local.origin.as_ref().map(|o| o.collection.clone());
                Some(PimdirChangeKind::Remove {
                    handle,
                    to,
                    link_id: local.link_id.clone(),
                    if_match: base_revision,
                })
            }
            (false, _, false) if local_tombstone => {
                self.drop(&handle, PimdirDropReason::Deleted);
                None
            }
            (true, true, false) => {
                let local = local.expect("local present");

                let edited = matches!(local.status, PimdirStatus::Dirty | PimdirStatus::Conflict)
                    && local.object.is_some()
                    && local
                        .base
                        .as_ref()
                        .is_some_and(|b| local.object != b.object);
                if edited {
                    self.resurrect(local);
                } else {
                    self.drop(&handle, PimdirDropReason::Deleted);
                }
                self.report.pulled += 1;
                self.emit(PimdirSyncEvent::Vanished(handle.clone()));
                None
            }
            (false, false, true) => {
                self.pull_add(&remote_item.expect("remote present"));
                self.report.pulled += 1;
                self.emit(PimdirSyncEvent::Added(handle.clone()));
                None
            }
            // NOTE: the flag merge runs even under a content push or
            // conflict, since a delta lists a flag change only once; the
            // content push merely withholds the flag push.
            (true, _, true) => {
                let local = local.expect("local present");
                let item = remote_item.as_ref().expect("remote present");

                match self.reconcile_content(&local, item) {
                    ContentOutcome::Push(change) => {
                        self.reconcile_flags(&local, item, PushFlags::Withhold);
                        Some(change)
                    }
                    ContentOutcome::Rewritten(rewritten) => {
                        self.reconcile_flags(&rewritten, item, PushFlags::Derive)
                    }
                    ContentOutcome::Untouched => {
                        self.reconcile_flags(&local, item, PushFlags::Derive)
                    }
                }
            }
            (true, false, false) => {
                let local = local.expect("local present");
                // NOTE: a pending create the item's own conflict projects
                // as `Conflict` (SYNC §3) owes nothing until an edit
                // settles it; it is neither gone nor a create to push.
                if local.status == PimdirStatus::Conflict && local.conflict_revision.is_none() {
                    return None;
                }
                if local.status != PimdirStatus::Created {
                    // NOTE: a base-less body is a create-collision whose
                    // remote side went, so the local body survives as a
                    // create; a base-less row holding none is a probe, and
                    // a probe the enumeration no longer lists is gone.
                    if local.object.is_some() {
                        self.resurrect(local);
                    } else {
                        self.drop(&handle, PimdirDropReason::Deleted);
                    }
                    self.report.pulled += 1;
                    self.emit(PimdirSyncEvent::Vanished(handle.clone()));
                    return None;
                }
                if self.holds_probes {
                    trace!(
                        "holding the create {} while the collection holds probes",
                        handle.as_str()
                    );
                    return None;
                }
                let pushable = local.object.is_some() || local.origin.is_some();
                if self.opts.push && self.opts.rights.add && pushable {
                    let add = PimdirChangeKind::Add {
                        handle: handle.clone(),
                        link_id: local.link_id.clone(),
                        flags: local.flags.clone(),
                        origin: local.origin.clone(),
                        object: local.object.clone(),
                    };
                    self.pending.insert(handle, Pending::Create(local));
                    return Some(add);
                }
                None
            }
            _ => None,
        }
    }

    /// Re-stages a body whose remote side went as a pending create (SYNC §5).
    ///
    /// New content beats a delete: the binding moves to the provisional
    /// handle its key derives, licensed by a `Superseded` drop of the one
    /// the member had, and is rewritten `Created` with no base, no origin
    /// and no divergence. The next run adds it; this one reports the
    /// member vanished.
    fn resurrect(&mut self, local: PimdirPlacement) {
        let mut resurrected = local;
        let provisional = resurrected
            .link_id
            .as_ref()
            .map(PimdirLinkId::provisional)
            .unwrap_or_else(|| resurrected.handle.clone());
        if provisional != resurrected.handle {
            self.drop(&resurrected.handle, PimdirDropReason::Superseded);
            resurrected.handle = provisional;
        }
        resurrected.status = PimdirStatus::Created;
        resurrected.conflict_revision = None;
        resurrected.conflict_object = None;
        resurrected.base = None;
        resurrected.origin = None;
        self.upsert(resurrected);
    }

    /// Reconciles the content of a placement present on both sides.
    ///
    /// Both axes read positive signals only: a dirty placement whose body
    /// its base does not hold, a reported revision differing from the
    /// base. Immutable content produces neither and falls through.
    fn reconcile_content(
        &mut self,
        local: &PimdirPlacement,
        item: &PimdirRemoteItem,
    ) -> ContentOutcome {
        let Some(base) = &local.base else {
            // NOTE: a base-less body the remote also holds is a
            // create-collision.
            if local.object.is_some() && item.revision.is_some() {
                return self.mark_conflict(local, item);
            }
            return ContentOutcome::Untouched;
        };

        if local.status == PimdirStatus::Conflict {
            // NOTE: a `Conflict` the item carries rather than the binding
            // (SYNC §3) records no revision here and is settled by an edit
            // or a remove, never by this axis.
            if local.conflict_revision.is_some()
                && item.revision.is_some()
                && item.revision != local.conflict_revision
            {
                let mut updated = local.clone();
                updated.conflict_revision = item.revision.clone();
                // NOTE: the stored diverging body described the old
                // revision, so the upgrade pass fetches it anew.
                updated.conflict_object = None;
                self.upsert(updated.clone());
                return ContentOutcome::Rewritten(updated);
            }
            return ContentOutcome::Untouched;
        }

        let local_changed = local.status == PimdirStatus::Dirty
            && local.object.is_some()
            && local.object != base.object;
        let remote_changed = item.revision.is_some() && item.revision != base.revision;

        match (local_changed, remote_changed) {
            (false, false) => ContentOutcome::Untouched,
            (false, true) => ContentOutcome::Rewritten(self.pull_content(local, item)),
            (true, false) => {
                if !(self.opts.push && self.opts.rights.content) {
                    return ContentOutcome::Untouched;
                }
                self.push_content(local, base.revision.clone())
            }
            (true, true) => self.resolve_conflict(local, item),
        }
    }

    /// Derives the `Update` pushing a staged body, gated on `if_match`.
    fn push_content(
        &mut self,
        local: &PimdirPlacement,
        if_match: Option<String>,
    ) -> ContentOutcome {
        let object = local.object.clone().expect("a staged edited body");
        self.pending
            .insert(local.handle.clone(), Pending::Content(local.clone()));
        ContentOutcome::Push(PimdirChangeKind::Update {
            handle: local.handle.clone(),
            object,
            if_match,
        })
    }

    /// Resolves a content conflict by the configured [`PimdirConflictPolicy`].
    fn resolve_conflict(
        &mut self,
        local: &PimdirPlacement,
        item: &PimdirRemoteItem,
    ) -> ContentOutcome {
        match self.opts.conflict {
            PimdirConflictPolicy::Manual => self.mark_conflict(local, item),
            PimdirConflictPolicy::PreferRemote => {
                ContentOutcome::Rewritten(self.pull_content(local, item))
            }
            PimdirConflictPolicy::PreferLocal => {
                // NOTE: the precondition is the observed remote revision,
                // not the stale base.
                if !(self.opts.push && self.opts.rights.content) {
                    return self.mark_conflict(local, item);
                }
                self.push_content(local, item.revision.clone())
            }
        }
    }

    /// Marks a placement conflicted, carrying the observed remote revision.
    ///
    /// The diverging body is marked wanted rather than taken, the engine
    /// fetching nothing itself: a conflict holding no conflict object is
    /// the request, and the upgrade pass is what answers it.
    fn mark_conflict(
        &mut self,
        local: &PimdirPlacement,
        item: &PimdirRemoteItem,
    ) -> ContentOutcome {
        let mut conflicted = local.clone();
        conflicted.status = PimdirStatus::Conflict;
        conflicted.conflict_revision = item.revision.clone();
        conflicted.conflict_object = None;
        self.upsert(conflicted.clone());
        self.report.conflicts += 1;
        self.emit(PimdirSyncEvent::Conflicted(local.handle.clone()));
        ContentOutcome::Rewritten(conflicted)
    }

    /// Reconciles the flag sets of a placement present on both sides.
    ///
    /// Flags merge element-wise ([`PimdirFlags::merge`]) and never
    /// conflict: remote-won flags are pulled, local-won ones pushed. A
    /// base-less placement, a probe, adopts the remote set, and one
    /// already holding it is left alone.
    fn reconcile_flags(
        &mut self,
        local: &PimdirPlacement,
        remote: &PimdirRemoteItem,
        allow: PushFlags,
    ) -> Option<PimdirChangeKind> {
        let base_flags = local.base.as_ref().map(|b| b.flags.clone());

        let Some(base_flags) = base_flags else {
            if local.flags == remote.flags {
                return None;
            }
            self.pull_flags(local, &remote.flags);
            self.report.pulled += 1;
            self.emit(PimdirSyncEvent::FlagsChanged(local.handle.clone()));
            return None;
        };

        let merged = PimdirFlags::merge(&base_flags, &local.flags, &remote.flags);
        let pull = merged != local.flags;
        let push = merged != remote.flags;

        match (pull, push) {
            // NOTE: a dirty placement whose flag edit turned out a no-op is
            // cleaned here, unless a staged content edit keeps it dirty.
            (false, false) => {
                let settled = local.status == PimdirStatus::Dirty && !content_pending(local);
                if local.flags != base_flags || settled {
                    self.rebase(local, &merged);
                }
                None
            }
            (true, false) => {
                self.pull_flags(local, &merged);
                self.report.pulled += 1;
                self.emit(PimdirSyncEvent::FlagsChanged(local.handle.clone()));
                None
            }
            (pull, true) => {
                let mut updated = local.clone();
                updated.flags = merged.clone();
                if pull {
                    self.upsert(updated.clone());
                    self.report.pulled += 1;
                    self.emit(PimdirSyncEvent::FlagsChanged(local.handle.clone()));
                }

                if allow == PushFlags::Withhold || !(self.opts.push && self.opts.rights.flags) {
                    return None;
                }
                self.pending
                    .insert(local.handle.clone(), Pending::Flags(updated));
                Some(PimdirChangeKind::SetFlags {
                    handle: local.handle.clone(),
                    flags: merged,
                })
            }
        }
    }

    /// Settles a local delete the source's rights refuse (SYNC §5): held
    /// when another source binds the collection and pushes the delete,
    /// reverted when this source is alone, since a held tombstone would
    /// hide a member an incremental enumeration never lists again.
    fn refuse_delete(&mut self, local: PimdirPlacement) -> Option<PimdirChangeKind> {
        if self.beside_others {
            trace!("holding a delete {} will not take", local.handle.as_str());
            return None;
        }

        debug!("reverting a delete {} will not take", local.handle.as_str());
        let mut reverted = local;
        // NOTE: only the delete is undone, a divergence or a staged edit
        // is still owed.
        reverted.status = match (&reverted.conflict_revision, reverted.staged_edit()) {
            (Some(_), _) => PimdirStatus::Conflict,
            (None, Some(_)) => PimdirStatus::Dirty,
            (None, None) => PimdirStatus::Clean,
        };
        // NOTE: left behind, the destination would turn the next plain
        // delete of the member into a move.
        reverted.origin = None;
        self.upsert(reverted);

        None
    }

    /// Applies an accepted push to its row (SYNC §5).
    fn settle(&mut self, pending: Pending, result: &PimdirPushResult) {
        match pending {
            Pending::Flags(placement) => {
                let flags = placement.flags.clone();
                self.rebase(&placement, &flags);
            }
            Pending::Content(placement) => {
                self.rebase_content(&placement, result.revision.clone());
            }
            Pending::Remove => self.drop(&result.handle, PimdirDropReason::Deleted),
            Pending::Create(placeholder) => {
                let created = result
                    .assigned
                    .clone()
                    .unwrap_or_else(|| result.handle.clone());
                match result.assigned.clone() {
                    Some(assigned) => {
                        self.rekey_create(placeholder, assigned, result.revision.clone())
                    }
                    // NOTE: no assigned handle (no UIDPLUS), so the next
                    // enumerate re-adds it.
                    None => self.drop(&placeholder.handle, PimdirDropReason::Superseded),
                }
                self.emit(PimdirSyncEvent::Created(created));
            }
        }
    }

    /// Records a per-item event for the report.
    fn emit(&mut self, event: PimdirSyncEvent) {
        self.report.events.push(event);
    }

    /// Writes a placement, refreshing the content push stashed for it.
    ///
    /// The flag axis writes a handle the content axis already claimed, and
    /// the accepted push must rebase what the merge last wrote for it, or
    /// the pulled flag is lost until an enumeration relists the item.
    fn upsert(&mut self, placement: PimdirPlacement) {
        if let Some(Pending::Content(stashed)) = self.pending.get_mut(&placement.handle) {
            *stashed = placement.clone();
        }
        self.writes.push(PimdirWriteOp::UpsertPlacement(placement));
    }

    fn drop(&mut self, handle: &PimdirHandle, reason: PimdirDropReason) {
        self.writes.push(PimdirWriteOp::DropPlacement {
            collection: self.collection.clone(),
            handle: handle.clone(),
            reason,
        });
    }

    fn pull_add(&mut self, item: &PimdirRemoteItem) {
        let placement = PimdirPlacement {
            collection: self.collection.clone(),
            handle: item.handle.clone(),
            link_id: None,
            object: None,
            level: PimdirLevel::Probed,
            summary: None,
            sort_key: PimdirSortKey::default(),
            flags: item.flags.clone(),
            status: PimdirStatus::Clean,
            conflict_revision: None,
            conflict_object: None,
            base: None,
            origin: None,
        };
        self.upsert(placement);
    }

    /// Revives a tombstone the remote edited past its base: the identity
    /// and summary stay, the body goes, and the base adopts what the
    /// remote reports, so the next upgrade refetches and nothing pushes.
    fn revive(&mut self, local: &PimdirPlacement, item: &PimdirRemoteItem) {
        let mut revived = local.clone();
        revived.object = None;
        revived.level = PimdirLevel::Probed;
        revived.flags = item.flags.clone();
        revived.status = PimdirStatus::Clean;
        revived.conflict_revision = None;
        revived.conflict_object = None;
        revived.origin = None;
        revived.base = Some(PimdirBase {
            flags: item.flags.clone(),
            revision: item.revision.clone(),
            object: None,
        });
        self.upsert(revived);
    }

    /// Pulls a remote content change: stale body dropped, revision rebased.
    ///
    /// The level falls back to probed, keeping the stale summary as a
    /// display fallback until a meta upgrade refetches it. Flags and
    /// status are left for the flag reconciliation.
    fn pull_content(
        &mut self,
        local: &PimdirPlacement,
        item: &PimdirRemoteItem,
    ) -> PimdirPlacement {
        let mut updated = local.clone();
        updated.object = None;
        updated.level = PimdirLevel::Probed;
        if let Some(base) = &mut updated.base {
            base.revision = item.revision.clone();
            base.object = None;
        }

        self.upsert(updated.clone());
        self.report.refreshed += 1;
        self.emit(PimdirSyncEvent::ContentChanged(local.handle.clone()));
        updated
    }

    /// Adopts `flags` as both the current and the base flag set.
    ///
    /// Only the flag axis converged, so a placement the content axis
    /// still owes a push for keeps its status, an unresolved conflict and
    /// a staged edit alike; everything else lands clean.
    fn pull_flags(&mut self, local: &PimdirPlacement, flags: &PimdirFlags) {
        let mut updated = local.clone();
        updated.flags = flags.clone();
        if updated.status != PimdirStatus::Conflict && !content_pending(&updated) {
            updated.status = PimdirStatus::Clean;
        }
        updated.base = Some(PimdirBase {
            flags: flags.clone(),
            revision: local.base.as_ref().and_then(|b| b.revision.clone()),
            object: local.base.as_ref().and_then(|b| b.object.clone()),
        });
        self.upsert(updated);
    }

    /// Rebases the flag axis onto `flags`, keeping the current flag set.
    ///
    /// Only the flag axis settles here, so only a placement with nothing
    /// else pending lands clean: an unresolved conflict and a staged
    /// content edit keep their status, or the edit is never pushed again.
    fn rebase(&mut self, local: &PimdirPlacement, flags: &PimdirFlags) {
        let mut updated = local.clone();
        if updated.status != PimdirStatus::Conflict && !content_pending(&updated) {
            updated.status = PimdirStatus::Clean;
        }
        updated.base = Some(PimdirBase {
            flags: flags.clone(),
            revision: local.base.as_ref().and_then(|b| b.revision.clone()),
            object: local.base.as_ref().and_then(|b| b.object.clone()),
        });
        self.upsert(updated);
    }

    /// Rebases an accepted content push onto the pushed body and revision.
    ///
    /// The base flags are left as they were, so a flag edit that rode
    /// along stays derivable and pushes on the next sync.
    fn rebase_content(&mut self, local: &PimdirPlacement, revision: Option<String>) {
        let base_flags = local
            .base
            .as_ref()
            .map(|b| b.flags.clone())
            .unwrap_or_default();

        let mut updated = local.clone();
        // NOTE: a tombstone stays one, its edit pushed ahead of the
        // pending move.
        updated.status = if local.status == PimdirStatus::Tombstone {
            PimdirStatus::Tombstone
        } else if local.flags == base_flags {
            PimdirStatus::Clean
        } else {
            PimdirStatus::Dirty
        };
        updated.base = Some(PimdirBase {
            flags: base_flags,
            revision,
            object: local.object.clone(),
        });
        self.upsert(updated);
    }

    /// Rekeys an accepted create under the server-assigned `handle`.
    ///
    /// The provisional placeholder is dropped and the placement upserted
    /// clean and based, so the next enumerate finds it in sync.
    fn rekey_create(
        &mut self,
        placeholder: PimdirPlacement,
        handle: PimdirHandle,
        revision: Option<String>,
    ) {
        self.drop(&placeholder.handle, PimdirDropReason::Superseded);
        let mut placed = placeholder;
        placed.handle = handle;
        placed.status = PimdirStatus::Clean;
        placed.origin = None;
        placed.base = Some(PimdirBase {
            flags: placed.flags.clone(),
            revision,
            object: placed.object.clone(),
        });
        self.upsert(placed);
    }
}

impl PimdirCoroutine for PimdirSync {
    type Yield = PimdirYield;
    type Return = Result<PimdirSyncReport, PimdirArgError>;

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
                self.local = loaded
                    .placements
                    .into_iter()
                    .map(|p| (p.handle.clone(), p))
                    .collect();
                self.checkpoint = if self.opts.full {
                    None
                } else {
                    loaded.checkpoint
                };

                debug!("enumerate remote from checkpoint");
                trace!("loaded {} local items", self.local.len());
                self.state = State::Enumerating;
                PimdirCoroutineState::Yielded(PimdirYield::WantsEnumerate {
                    collection: self.collection.clone(),
                    cursor: self.checkpoint.clone(),
                })
            }

            (State::Enumerating, Some(PimdirArg::Enumerate(snapshot))) => {
                trace!(
                    "enumerated {} items, {} vanished, complete={}",
                    snapshot.items.len(),
                    snapshot.vanished.len(),
                    snapshot.complete,
                );
                self.open_merge(snapshot);
                self.merge_step()
            }

            (State::Merging, Some(PimdirArg::Write)) => self.merge_step(),

            (State::Pushing, Some(PimdirArg::Push(results))) => {
                for result in &results {
                    // NOTE: matched by handle and settled once: a result
                    // naming a handle nobody pushed, or naming one twice,
                    // moves nothing and counts for nothing.
                    let Some(pending) = self.in_flight.remove(&result.handle) else {
                        continue;
                    };
                    match result.outcome {
                        PimdirPushOutcome::Accepted => {
                            self.settle(pending, result);
                            self.report.pushed += 1;
                        }
                        PimdirPushOutcome::Rejected => self.report.rejected += 1,
                    }
                }
                // NOTE: unreported handles are forgotten too, but only this
                // chunk's: later chunks have not been pushed yet.
                self.in_flight.clear();

                debug!("pushed, write {} storage ops", self.writes.len());
                trace!("push results: {results:?}");
                self.state = State::Writing;
                let batch = self.batch();
                PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(batch))
            }

            (State::Writing, Some(PimdirArg::Write)) => {
                if !self.pushes.is_empty() {
                    return self.step();
                }

                debug!(
                    "sync done: {} pulled, {} pushed, {} refreshed, {} conflicts, {} rejected",
                    self.report.pulled,
                    self.report.pushed,
                    self.report.refreshed,
                    self.report.conflicts,
                    self.report.rejected,
                );
                self.state = State::Done;
                PimdirCoroutineState::Complete(Ok(mem::take(&mut self.report)))
            }

            (State::Done, _) | (_, Some(_)) => {
                PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg))
            }
            (_, None) => PimdirCoroutineState::Complete(Err(PimdirArgError::MissingArg)),
        }
    }
}

/// Whether a placement is a probe: a handle enumerated and named by no
/// fetch yet (SYNC §3), which a create staged without an identity is not.
fn is_probe(placement: &PimdirPlacement) -> bool {
    placement.link_id.is_none()
        && placement.base.is_none()
        && placement.status != PimdirStatus::Created
}

/// Whether the content axis still owes a push for this placement.
///
/// A dirty placement pointing at a body its base does not hold. The flag
/// axis must not land such a row clean, or no later run derives a push
/// for it.
fn content_pending(placement: &PimdirPlacement) -> bool {
    placement.status == PimdirStatus::Dirty && placement.staged_edit().is_some()
}

/// Whether the flag axis may derive a push of its own.
///
/// A push result is matched by handle, so one handle yields at most one
/// change: when the content axis claimed it, the flag axis still merges
/// and writes, it just leaves its own push for the next sync.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PushFlags {
    Derive,
    Withhold,
}

/// What the content axis decided for a placement present on both sides.
// NOTE: produced and consumed per candidate on the merge's hot path, so
// boxing the large variant would add an allocation per item.
#[allow(clippy::large_enum_variant)]
enum ContentOutcome {
    /// No content signal on either side: the flag merge runs as loaded.
    Untouched,
    /// Rewritten by a pull or a conflict mark: flags merge on this copy.
    Rewritten(PimdirPlacement),
    /// The local content won: the change to push.
    Push(PimdirChangeKind),
}

/// A derived push awaiting its outcome, and what acceptance does to its row.
///
/// A rejected or unreported push leaves the row as the merge wrote it,
/// so the next run derives the change again.
enum Pending {
    /// A flag push: the placement to rebase onto the pushed set.
    Flags(PimdirPlacement),
    /// A content push: the placement the merge last wrote, to rebase onto
    /// the pushed body and the reported revision.
    Content(PimdirPlacement),
    /// A delete: the tombstone is dropped once confirmed.
    Remove,
    /// A create: the placeholder is superseded by the assigned handle.
    Create(PimdirPlacement),
}

/// What the coroutine is doing while it waits for the caller.
enum State {
    Start,
    Loading,
    Enumerating,
    Merging,
    Pushing,
    Writing,
    Done,
}

#[cfg(test)]
mod tests {
    use alloc::{format, vec, vec::Vec};

    use crate::{
        load::PimdirLoaded,
        object::PimdirHash,
        placement::{PimdirLinkId, PimdirOrigin},
        remote::{PimdirPushOutcome, PimdirPushResult, PimdirRemoteSnapshot},
        sync::*,
    };

    /// A pending create staged in "inbox", its body sourced from "sent".
    fn created(handle: &str) -> PimdirPlacement {
        PimdirPlacement {
            sort_key: Default::default(),
            collection: "inbox".into(),
            handle: PimdirHandle::from(handle),
            link_id: None,
            object: None,
            level: PimdirLevel::Probed,
            summary: None,
            flags: PimdirFlags::default(),
            status: PimdirStatus::Created,
            conflict_revision: None,
            conflict_object: None,
            base: None,
            origin: Some(PimdirOrigin {
                collection: "sent".into(),
                handle: PimdirHandle::from("9"),
            }),
        }
    }

    fn synced(handle: &str, flags: &[&str]) -> PimdirPlacement {
        PimdirPlacement {
            sort_key: Default::default(),
            collection: "inbox".into(),
            handle: PimdirHandle::from(handle),
            link_id: Some(PimdirLinkId::from(handle)),
            object: None,
            level: PimdirLevel::Probed,
            summary: None,
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

    fn remote(handle: &str, flags: &[&str]) -> PimdirRemoteItem {
        PimdirRemoteItem {
            handle: PimdirHandle::from(handle),
            flags: PimdirFlags::from_iter(flags.iter().copied()),
            revision: None,
        }
    }

    fn full(items: Vec<PimdirRemoteItem>) -> PimdirRemoteSnapshot {
        PimdirRemoteSnapshot {
            items,
            vanished: Vec::new(),
            complete: true,
            checkpoint: PimdirCheckpoint(b"c1".to_vec()),
        }
    }

    fn delta(items: Vec<PimdirRemoteItem>, vanished: Vec<PimdirHandle>) -> PimdirRemoteSnapshot {
        PimdirRemoteSnapshot {
            items,
            vanished,
            complete: false,
            checkpoint: PimdirCheckpoint(b"c1".to_vec()),
        }
    }

    fn run(
        sync: &mut PimdirSync,
        local: Vec<PimdirPlacement>,
        items: Vec<PimdirRemoteItem>,
    ) -> (
        Option<Vec<PimdirChange>>,
        Vec<PimdirWriteOp>,
        PimdirSyncReport,
    ) {
        run_snapshot(sync, local, full(items))
    }

    fn run_snapshot(
        sync: &mut PimdirSync,
        local: Vec<PimdirPlacement>,
        snapshot: PimdirRemoteSnapshot,
    ) -> (
        Option<Vec<PimdirChange>>,
        Vec<PimdirWriteOp>,
        PimdirSyncReport,
    ) {
        crate::testlog::init();
        let _ = sync.resume(None);
        let _ = sync.resume(Some(PimdirArg::Load(PimdirLoaded {
            placements: local,
            checkpoint: None,
        })));

        let mut pushes = None;
        let writes = match sync.resume(Some(PimdirArg::Enumerate(snapshot))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsPush { changes, .. }) => {
                let results = changes
                    .iter()
                    .map(|change| PimdirPushResult {
                        handle: change.handle().clone(),
                        outcome: PimdirPushOutcome::Accepted,
                        assigned: None,
                        revision: None,
                    })
                    .collect();
                pushes = Some(changes);
                match sync.resume(Some(PimdirArg::Push(results))) {
                    PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(w)) => w,
                    state => panic!("expected WantsWrite, got {state:?}"),
                }
            }
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(w)) => w,
            state => panic!("expected push or write, got {state:?}"),
        };

        let report = match sync.resume(Some(PimdirArg::Write)) {
            PimdirCoroutineState::Complete(Ok(report)) => report,
            state => panic!("expected Complete(Ok), got {state:?}"),
        };

        (pushes, writes, report)
    }

    #[test]
    fn remote_add_pulls_probed() {
        crate::testlog::init();
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) = run(&mut sync, vec![], vec![remote("1", &["seen"])]);

        assert!(pushes.is_none());
        assert_eq!(report.pulled, 1);
        let PimdirWriteOp::UpsertPlacement(p) = &writes[0] else {
            panic!("expected UpsertPlacement, got {:?}", writes[0]);
        };
        assert_eq!(p.level, PimdirLevel::Probed);
        assert!(p.flags.contains("seen"));
    }

    /// Unknown markers hold no opinion (spec §13): the remote set is pulled.
    #[test]
    fn an_unknown_local_set_adopts_the_remote_one_and_pushes_nothing() {
        let mut local = synced("1", &[]);
        local.flags = PimdirFlags::Unknown;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

        assert!(pushes.is_none(), "nothing to push from an unknown set");
        assert_eq!(report.pulled, 1);
        let PimdirWriteOp::UpsertPlacement(p) = &writes[0] else {
            panic!("expected UpsertPlacement, got {:?}", writes[0]);
        };
        assert!(p.flags.contains("seen"));
    }

    #[test]
    fn local_flag_change_pushes() {
        let mut local = synced("1", &[]);
        local.flags = PimdirFlags::from_iter(["seen"]);
        local.status = PimdirStatus::Dirty;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, _writes, report) = run(&mut sync, vec![local], vec![remote("1", &[])]);

        let pushes = pushes.expect("a push");
        assert!(matches!(pushes[0].kind, PimdirChangeKind::SetFlags { .. }));
        assert_eq!(report.pushed, 1);
    }

    /// Runs a sync through its push with the given results.
    ///
    /// Returns the writes the engine then stages, and the report.
    fn run_push(
        sync: &mut PimdirSync,
        local: Vec<PimdirPlacement>,
        items: Vec<PimdirRemoteItem>,
        results: Vec<PimdirPushResult>,
    ) -> (Vec<PimdirWriteOp>, PimdirSyncReport) {
        crate::testlog::init();
        let _ = sync.resume(None);
        let _ = sync.resume(Some(PimdirArg::Load(PimdirLoaded {
            placements: local,
            checkpoint: None,
        })));
        match sync.resume(Some(PimdirArg::Enumerate(full(items)))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsPush { .. }) => {}
            state => panic!("expected WantsPush, got {state:?}"),
        }
        let writes = match sync.resume(Some(PimdirArg::Push(results))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(writes)) => writes,
            state => panic!("expected WantsWrite, got {state:?}"),
        };
        let report = match sync.resume(Some(PimdirArg::Write)) {
            PimdirCoroutineState::Complete(Ok(report)) => report,
            state => panic!("expected Complete(Ok), got {state:?}"),
        };
        (writes, report)
    }

    /// What a run asked for, in the order it asked.
    struct Run {
        /// Each push chunk, in order.
        chunks: Vec<Vec<PimdirChange>>,
        /// Each write batch, in order.
        batches: Vec<Vec<PimdirWriteOp>>,
        /// The yields as they came, so a test can pin the interleaving.
        order: Vec<&'static str>,
        report: PimdirSyncReport,
    }

    impl Run {
        /// Every write of the run, batch boundaries flattened away.
        fn writes(&self) -> Vec<PimdirWriteOp> {
            self.batches.iter().flatten().cloned().collect()
        }

        /// The index of the batch holding a write, if any.
        fn batch_of(&self, mut held: impl FnMut(&PimdirWriteOp) -> bool) -> Option<usize> {
            self.batches
                .iter()
                .position(|batch| batch.iter().any(&mut held))
        }
    }

    /// Runs a sync to completion against an accepting remote, keeping yields.
    ///
    /// Unlike [`run`] it assumes nothing about how many pushes and
    /// writes a run takes, which is what the chunked paths are about.
    fn run_batches(
        sync: &mut PimdirSync,
        local: Vec<PimdirPlacement>,
        snapshot: PimdirRemoteSnapshot,
    ) -> Run {
        crate::testlog::init();
        let _ = sync.resume(None);
        let _ = sync.resume(Some(PimdirArg::Load(PimdirLoaded {
            placements: local,
            checkpoint: None,
        })));

        let mut run = Run {
            chunks: Vec::new(),
            batches: Vec::new(),
            order: Vec::new(),
            report: PimdirSyncReport::default(),
        };
        let mut arg = Some(PimdirArg::Enumerate(snapshot));

        loop {
            match sync.resume(arg.take()) {
                PimdirCoroutineState::Yielded(PimdirYield::WantsPush { changes, .. }) => {
                    run.order.push("push");
                    let results = changes
                        .iter()
                        .map(|change| PimdirPushResult {
                            handle: change.handle().clone(),
                            outcome: PimdirPushOutcome::Accepted,
                            assigned: None,
                            revision: None,
                        })
                        .collect();
                    run.chunks.push(changes);
                    arg = Some(PimdirArg::Push(results));
                }
                PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(writes)) => {
                    run.order.push("write");
                    run.batches.push(writes);
                    arg = Some(PimdirArg::Write);
                }
                PimdirCoroutineState::Complete(Ok(report)) => {
                    run.report = report;
                    return run;
                }
                state => panic!("expected push or write, got {state:?}"),
            }
        }
    }

    /// The last placement an UpsertPlacement op writes for `handle`, if any.
    ///
    /// The last, since a batch applies in order and the row a store ends up
    /// holding is the last write naming it.
    fn upserted<'a>(writes: &'a [PimdirWriteOp], handle: &str) -> Option<&'a PimdirPlacement> {
        writes.iter().rev().find_map(|w| match w {
            PimdirWriteOp::UpsertPlacement(p) if p.handle.as_str() == handle => Some(p),
            _ => None,
        })
    }

    #[test]
    fn rejected_flag_push_keeps_dirty() {
        let mut local = synced("1", &[]);
        local.flags = PimdirFlags::from_iter(["flagged"]);
        local.status = PimdirStatus::Dirty;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let results = vec![PimdirPushResult {
            handle: PimdirHandle::from("1"),
            outcome: PimdirPushOutcome::Rejected,
            assigned: None,
            revision: None,
        }];
        let (writes, report) = run_push(&mut sync, vec![local], vec![remote("1", &[])], results);

        assert!(
            upserted(&writes, "1").is_none(),
            "a rejected flag push must not rebase the placement: {writes:?}",
        );
        assert_eq!(report.rejected, 1);
    }

    #[test]
    fn accepted_flag_push_rebases_clean() {
        let mut local = synced("1", &[]);
        local.flags = PimdirFlags::from_iter(["flagged"]);
        local.status = PimdirStatus::Dirty;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let results = vec![PimdirPushResult {
            handle: PimdirHandle::from("1"),
            outcome: PimdirPushOutcome::Accepted,
            assigned: None,
            revision: None,
        }];
        let (writes, _report) = run_push(&mut sync, vec![local], vec![remote("1", &[])], results);

        let rebased = upserted(&writes, "1").expect("an accepted flag push rebases the placement");
        assert_eq!(rebased.status, PimdirStatus::Clean);
        assert!(
            rebased
                .base
                .as_ref()
                .expect("a base")
                .flags
                .contains("flagged")
        );
    }

    /// A dirty flag placement, ready to derive one push.
    fn pending(handle: &str) -> PimdirPlacement {
        let mut placement = synced(handle, &[]);
        placement.flags = PimdirFlags::from_iter(["flagged"]);
        placement.status = PimdirStatus::Dirty;
        placement
    }

    fn accepted(handle: &str) -> PimdirPushResult {
        PimdirPushResult {
            handle: PimdirHandle::from(handle),
            outcome: PimdirPushOutcome::Accepted,
            assigned: None,
            revision: None,
        }
    }

    fn rejected(handle: &str) -> PimdirPushResult {
        PimdirPushResult {
            outcome: PimdirPushOutcome::Rejected,
            ..accepted(handle)
        }
    }

    /// A handle nobody reported on is retried, never assumed accepted.
    #[test]
    fn an_unreported_push_stays_pending() {
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (writes, report) = run_push(
            &mut sync,
            vec![pending("1"), pending("2")],
            vec![remote("1", &[]), remote("2", &[])],
            vec![accepted("1")],
        );

        assert_eq!(
            upserted(&writes, "1")
                .expect("the reported push rebases")
                .status,
            PimdirStatus::Clean,
        );
        assert!(
            upserted(&writes, "2").is_none(),
            "an unreported push must stay dirty for the next run: {writes:?}",
        );
        assert_eq!(report.pushed, 1);
        assert_eq!(report.rejected, 0, "silence is not a rejection");
    }

    /// Results match by handle: order, strangers and duplicates do not matter.
    #[test]
    fn a_result_set_is_matched_by_handle_not_by_shape() {
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let results = vec![
            accepted("2"),
            accepted("nobody-pushed-this"),
            rejected("nobody-pushed-this-either"),
            accepted("1"),
            accepted("2"),
            rejected("2"),
        ];
        let (writes, report) = run_push(
            &mut sync,
            vec![pending("1"), pending("2")],
            vec![remote("1", &[]), remote("2", &[])],
            results,
        );

        for handle in ["1", "2"] {
            let rebased = upserted(&writes, handle);
            assert!(rebased.is_some_and(|p| p.status == PimdirStatus::Clean));
        }
        assert!(upserted(&writes, "nobody-pushed-this").is_none());
        assert_eq!(
            report.pushed, 2,
            "a duplicate result and an unknown handle cannot inflate the count",
        );
        assert_eq!(
            report.rejected, 0,
            "a rejection counts for a pushed handle only, and once",
        );
        assert_eq!(
            report.events.len(),
            0,
            "a pushed change reports no event: {:?}",
            report.events,
        );
    }

    #[test]
    fn partial_push_accepts_one_rejects_other() {
        let mut one = synced("1", &[]);
        one.flags = PimdirFlags::from_iter(["flagged"]);
        one.status = PimdirStatus::Dirty;
        let mut two = synced("2", &[]);
        two.flags = PimdirFlags::from_iter(["flagged"]);
        two.status = PimdirStatus::Dirty;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let results = vec![
            PimdirPushResult {
                handle: PimdirHandle::from("1"),
                outcome: PimdirPushOutcome::Accepted,
                assigned: None,
                revision: None,
            },
            PimdirPushResult {
                handle: PimdirHandle::from("2"),
                outcome: PimdirPushOutcome::Rejected,
                assigned: None,
                revision: None,
            },
        ];
        let (writes, report) = run_push(
            &mut sync,
            vec![one, two],
            vec![remote("1", &[]), remote("2", &[])],
            results,
        );

        assert_eq!(
            upserted(&writes, "1").expect("accepted rebases").status,
            PimdirStatus::Clean,
        );
        assert!(
            upserted(&writes, "2").is_none(),
            "rejected handle must stay dirty: {writes:?}",
        );
        assert_eq!(report.pushed, 1, "only the accepted change counts");
        assert_eq!(report.rejected, 1);
    }

    #[test]
    fn rejected_push_retries_on_next_sync() {
        let mut local = synced("1", &[]);
        local.flags = PimdirFlags::from_iter(["flagged"]);
        local.status = PimdirStatus::Dirty;

        let mut first = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let results = vec![PimdirPushResult {
            handle: PimdirHandle::from("1"),
            outcome: PimdirPushOutcome::Rejected,
            assigned: None,
            revision: None,
        }];
        let (writes, _report) = run_push(
            &mut first,
            vec![local.clone()],
            vec![remote("1", &[])],
            results,
        );
        assert!(upserted(&writes, "1").is_none(), "rejected push left dirty");

        let mut second = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, _writes, report) = run(&mut second, vec![local], vec![remote("1", &[])]);
        let pushes = pushes.expect("the dirty change is pushed again");
        assert!(matches!(pushes[0].kind, PimdirChangeKind::SetFlags { .. }));
        assert_eq!(report.pushed, 1);
    }

    /// Each chunk is recorded before the next is pushed, bounding the replay.
    #[test]
    fn a_chunk_is_recorded_before_the_next_one_is_pushed() {
        let extra = 3;
        let count = PimdirSync::PUSH_CHUNK + extra;

        let mut local = Vec::new();
        let mut items = Vec::new();
        for index in 0..count {
            let handle = format!("{index:03}");
            let mut placement = synced(&handle, &[]);
            placement.flags = PimdirFlags::from_iter(["seen"]);
            placement.status = PimdirStatus::Dirty;
            local.push(placement);
            items.push(remote(&handle, &[]));
        }

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let run = run_batches(&mut sync, local, full(items));

        assert_eq!(run.order, ["push", "write", "push", "write"]);
        assert_eq!(run.chunks[0].len(), PimdirSync::PUSH_CHUNK);
        assert_eq!(run.chunks[1].len(), extra);

        for change in &run.chunks[0] {
            let rebased = upserted(&run.batches[0], change.handle().as_str());
            assert!(rebased.is_some_and(|p| p.status == PimdirStatus::Clean));
        }
        for change in &run.chunks[1] {
            let handle = change.handle().as_str();
            assert!(upserted(&run.batches[0], handle).is_none());
            let rebased = upserted(&run.batches[1], handle);
            assert!(rebased.is_some_and(|p| p.status == PimdirStatus::Clean));
        }

        assert!(
            run.batch_of(|op| matches!(op, PimdirWriteOp::SetCheckpoint { .. })) == Some(1),
            "the checkpoint must land in the closing batch",
        );

        assert_eq!(run.report.pushed, count);
    }

    /// An unordered enumeration derives exactly what the sorted one derives.
    #[test]
    fn an_unordered_enumeration_merges_like_an_ordered_one() {
        let local = || {
            let mut dirty = synced("5", &[]);
            dirty.flags = PimdirFlags::from_iter(["flagged"]);
            dirty.status = PimdirStatus::Dirty;
            vec![synced("1", &[]), synced("2", &[]), synced("3", &[]), dirty]
        };
        let items = || {
            vec![
                remote("1", &["seen"]),
                remote("3", &[]),
                remote("4", &[]),
                remote("5", &[]),
            ]
        };

        let mut ordered = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let ordered = run_batches(&mut ordered, local(), full(items()));

        let mut shuffled = items();
        shuffled.reverse();
        let mut unordered = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let unordered = run_batches(&mut unordered, local(), full(shuffled));

        assert_eq!(unordered.chunks, ordered.chunks, "different pushes");
        assert_eq!(unordered.writes(), ordered.writes(), "different writes");
        assert_eq!(unordered.report, ordered.report, "different report");
        assert_eq!(
            ordered.report.pulled, 3,
            "one pull each of add, flags, drop"
        );
        assert_eq!(ordered.report.pushed, 1);
    }

    /// A handle listed twice pairs with its one placement, pulling no phantom.
    #[test]
    fn a_handle_listed_twice_is_merged_once() {
        let snapshot = full(vec![remote("1", &["seen"]), remote("1", &["seen"])]);
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let run = run_batches(&mut sync, vec![synced("1", &[])], snapshot);

        assert_eq!(
            run.report.events,
            [PimdirSyncEvent::FlagsChanged("1".into())]
        );
        assert_eq!(
            run.writes().len(),
            2,
            "one upsert and the checkpoint: {:?}",
            run.writes(),
        );
    }

    /// The merge hands a full write batch over rather than holding every write.
    #[test]
    fn a_full_write_batch_is_handed_over_mid_merge() {
        let extra = 76;
        let count = PimdirSync::WRITE_CHUNK + extra;
        let items = (0..count)
            .map(|index| remote(&format!("{index:05}"), &[]))
            .collect();

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let run = run_batches(&mut sync, vec![], full(items));

        assert_eq!(run.order, ["write", "write"], "one batch, or three");
        assert_eq!(run.batches[0].len(), PimdirSync::WRITE_CHUNK);
        assert_eq!(
            run.batches[1].len(),
            extra + 1,
            "the rest, plus the checkpoint",
        );
        assert!(
            run.batch_of(|op| matches!(op, PimdirWriteOp::SetCheckpoint { .. })) == Some(1),
            "a mid-merge batch must not checkpoint what it has not merged",
        );
        assert_eq!(run.report.pulled, count);
    }

    /// A batch boundary falls between candidates, never inside one.
    ///
    /// A vanished member holding a local edit is re-staged as a create: a
    /// drop of its handle and the create under the provisional one, and
    /// losing either would lose the edit or leave two rows.
    #[test]
    fn a_batch_never_cuts_through_one_candidate() {
        let fillers = PimdirSync::WRITE_CHUNK - 1;
        let items = (0..fillers)
            .map(|index| remote(&format!("{index:05}"), &[]))
            .collect();

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let run = run_batches(&mut sync, vec![edited("zz")], full(items));

        let staged = run
            .batch_of(|op| matches!(op, PimdirWriteOp::UpsertPlacement(p) if p.status == PimdirStatus::Created))
            .expect("the re-staged create");
        let dropped = run
            .batch_of(|op| matches!(op, PimdirWriteOp::DropPlacement { handle, .. } if handle.as_str() == "zz"))
            .expect("the vanished handle's drop");

        assert!(
            run.batches[0].len() > PimdirSync::WRITE_CHUNK,
            "the boundary must fall on the candidate, not before it",
        );
        assert_eq!(
            staged, dropped,
            "both writes of one candidate must land together: {:?}",
            run.order,
        );
    }

    #[test]
    fn remote_flag_change_pulls() {
        let local = synced("1", &[]);
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

        assert!(pushes.is_none());
        assert_eq!(report.pulled, 1);
        let PimdirWriteOp::UpsertPlacement(p) = &writes[0] else {
            panic!("expected UpsertPlacement, got {:?}", writes[0]);
        };
        assert!(p.flags.contains("seen"));
        assert_eq!(p.status, PimdirStatus::Clean);
    }

    /// Each side wins its own flag: the union is pushed, no conflict.
    #[test]
    fn divergent_flags_merge_element_wise() {
        let mut local = synced("1", &[]);
        local.flags = PimdirFlags::from_iter(["flagged"]);
        local.status = PimdirStatus::Dirty;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

        match &pushes.expect("a push")[0].kind {
            PimdirChangeKind::SetFlags { flags, .. } => {
                assert!(flags.contains("flagged") && flags.contains("seen"));
            }
            other => panic!("expected a SetFlags push, got {other:?}"),
        }
        assert_eq!(report.conflicts, 0);
        assert_eq!(report.pulled, 1, "the remote-won flag is folded in");

        let rebased = writes
            .iter()
            .rev()
            .find_map(|w| match w {
                PimdirWriteOp::UpsertPlacement(p) if p.handle.as_str() == "1" => Some(p),
                _ => None,
            })
            .expect("a rebased placement");
        assert_eq!(rebased.status, PimdirStatus::Clean);
        assert!(rebased.flags.contains("flagged") && rebased.flags.contains("seen"));
        let base = rebased.base.as_ref().expect("a base");
        assert!(base.flags.contains("flagged") && base.flags.contains("seen"));
    }

    /// The local removal and the remote addition both win.
    #[test]
    fn flag_removal_merges_against_concurrent_addition() {
        let mut local = synced("1", &["seen"]);
        local.flags = PimdirFlags::default();
        local.status = PimdirStatus::Dirty;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, _writes, report) = run(
            &mut sync,
            vec![local],
            vec![remote("1", &["seen", "important"])],
        );

        match &pushes.expect("a push")[0].kind {
            PimdirChangeKind::SetFlags { flags, .. } => {
                assert!(flags.contains("important"), "the remote addition wins");
                assert!(!flags.contains("seen"), "the local removal wins");
            }
            other => panic!("expected a SetFlags push, got {other:?}"),
        }
        assert_eq!(report.conflicts, 0);
    }

    #[test]
    fn read_only_keeps_local_dirty() {
        let mut local = synced("1", &[]);
        local.flags = PimdirFlags::from_iter(["seen"]);
        local.status = PimdirStatus::Dirty;

        let opts = PimdirSyncOptions {
            push: false,
            rights: PimdirPushRights::all(),
            conflict: PimdirConflictPolicy::Manual,
            full: false,
        };
        let mut sync = PimdirSync::new("inbox", opts);
        let (pushes, _writes, report) = run(&mut sync, vec![local], vec![remote("1", &[])]);

        assert!(pushes.is_none(), "read-only source must not push");
        assert_eq!(report.pushed, 0);
    }

    #[test]
    fn delta_vanished_drops() {
        let local = synced("1", &["seen"]);
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let snapshot = delta(vec![], vec![PimdirHandle::from("1")]);
        let (pushes, writes, report) = run_snapshot(&mut sync, vec![local], snapshot);

        assert!(pushes.is_none());
        assert_eq!(report.pulled, 1);
        assert!(
            matches!(&writes[0], PimdirWriteOp::DropPlacement { handle, .. } if handle.as_str() == "1"),
            "vanished placement dropped, got {:?}",
            writes[0],
        );
    }

    #[test]
    fn delta_pull_add() {
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let snapshot = delta(vec![remote("9", &["seen"])], vec![]);
        let (pushes, writes, report) = run_snapshot(&mut sync, vec![], snapshot);

        assert!(pushes.is_none());
        assert_eq!(report.pulled, 1);
        let PimdirWriteOp::UpsertPlacement(p) = &writes[0] else {
            panic!("expected UpsertPlacement, got {:?}", writes[0]);
        };
        assert_eq!(p.handle.as_str(), "9");
        assert_eq!(p.level, PimdirLevel::Probed);
    }

    #[test]
    fn delta_leaves_unlisted_untouched() {
        let one = synced("1", &[]);
        let two = synced("2", &[]);
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let snapshot = delta(vec![remote("2", &["seen"])], vec![]);
        let (pushes, writes, report) = run_snapshot(&mut sync, vec![one, two], snapshot);

        assert!(pushes.is_none());
        assert_eq!(report.pulled, 1);
        assert_eq!(writes.len(), 2, "only the changed placement and checkpoint");
        let PimdirWriteOp::UpsertPlacement(p) = &writes[0] else {
            panic!("expected UpsertPlacement, got {:?}", writes[0]);
        };
        assert_eq!(p.handle.as_str(), "2");
        assert!(p.flags.contains("seen"));
    }

    /// An unlisted dirty handle derives its pending push against its own base.
    #[test]
    fn delta_pushes_unlisted_local_dirty() {
        let mut local = synced("1", &[]);
        local.flags = PimdirFlags::from_iter(["seen"]);
        local.status = PimdirStatus::Dirty;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let snapshot = delta(vec![], vec![]);
        let (pushes, _writes, report) = run_snapshot(&mut sync, vec![local], snapshot);

        let pushes = pushes.expect("a push");
        assert!(matches!(pushes[0].kind, PimdirChangeKind::SetFlags { .. }));
        assert_eq!(report.pushed, 1);
    }

    #[test]
    fn unchanged_flags_is_noop() {
        let local = synced("1", &["seen"]);
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

        assert!(pushes.is_none());
        assert_eq!(report, PimdirSyncReport::default(), "a no-op sync");
        assert!(
            upserted(&writes, "1").is_none(),
            "an unchanged placement is not rewritten: {writes:?}",
        );
    }

    #[test]
    fn concurrent_same_flags_rebases_without_push() {
        let mut local = synced("1", &[]);
        local.flags = PimdirFlags::from_iter(["flagged"]);
        local.status = PimdirStatus::Dirty;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) = run(&mut sync, vec![local], vec![remote("1", &["flagged"])]);

        assert!(pushes.is_none(), "no push when both reached the same flags");
        assert_eq!(report.conflicts, 0);
        let rebased = upserted(&writes, "1").expect("a converging rebase");
        assert_eq!(rebased.status, PimdirStatus::Clean);
        assert!(
            rebased
                .base
                .as_ref()
                .expect("a base")
                .flags
                .contains("flagged")
        );
    }

    #[test]
    fn no_base_present_converges_on_remote() {
        let mut local = synced("1", &["flagged"]);
        local.base = None;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

        assert!(pushes.is_none());
        assert_eq!(report.pulled, 1);
        let pulled = upserted(&writes, "1").expect("a converged placement");
        assert_eq!(pulled.status, PimdirStatus::Clean);
        assert!(pulled.flags.contains("seen"));
        assert!(!pulled.flags.contains("flagged"), "remote flags win");
    }

    /// Pulled flags never launder a conflict away, or the staged edit is lost.
    #[test]
    fn flag_pull_on_a_conflicted_placement_keeps_the_conflict() {
        let mut placement = edited("1");
        placement.status = PimdirStatus::Conflict;
        placement.conflict_revision = Some("r2".into());

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let mut item = remote_rev("1", "r2");
        item.flags = PimdirFlags::from_iter(["seen"]);
        let (pushes, writes, report) = run(&mut sync, vec![placement], vec![item]);

        assert!(pushes.is_none());
        assert_eq!(report.pulled, 1);
        let pulled = upserted(&writes, "1").expect("a flag pull");
        assert_eq!(
            pulled.status,
            PimdirStatus::Conflict,
            "the conflict survives"
        );
        assert!(pulled.flags.contains("seen"));
        assert_eq!(
            pulled.object,
            Some(PimdirHash::from("h2")),
            "the edit survives"
        );
    }

    #[test]
    fn read_only_still_pulls_remote_changes() {
        let local = synced("1", &[]);
        let opts = PimdirSyncOptions {
            push: false,
            rights: PimdirPushRights::all(),
            conflict: PimdirConflictPolicy::Manual,
            full: false,
        };
        let mut sync = PimdirSync::new("inbox", opts);
        let (pushes, writes, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

        assert!(pushes.is_none());
        assert_eq!(report.pulled, 1);
        assert!(
            upserted(&writes, "1")
                .expect("a pull")
                .flags
                .contains("seen")
        );
    }

    #[test]
    fn accepted_delete_drops_tombstone() {
        let mut local = synced("1", &["seen"]);
        local.status = PimdirStatus::Tombstone;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let results = vec![PimdirPushResult {
            handle: PimdirHandle::from("1"),
            outcome: PimdirPushOutcome::Accepted,
            assigned: None,
            revision: None,
        }];
        let (writes, report) = run_push(
            &mut sync,
            vec![local],
            vec![remote("1", &["seen"])],
            results,
        );

        assert_eq!(report.pushed, 1);
        assert!(
            writes.iter().any(
                |w| matches!(w, PimdirWriteOp::DropPlacement { handle, .. } if handle.as_str() == "1")
            ),
            "an accepted delete drops the tombstone: {writes:?}",
        );
    }

    #[test]
    fn rejected_delete_keeps_tombstone() {
        let mut local = synced("1", &["seen"]);
        local.status = PimdirStatus::Tombstone;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let results = vec![PimdirPushResult {
            handle: PimdirHandle::from("1"),
            outcome: PimdirPushOutcome::Rejected,
            assigned: None,
            revision: None,
        }];
        let (writes, report) = run_push(
            &mut sync,
            vec![local],
            vec![remote("1", &["seen"])],
            results,
        );

        assert_eq!(report.rejected, 1);
        assert!(
            !writes.iter().any(
                |w| matches!(w, PimdirWriteOp::DropPlacement { handle, .. } if handle.as_str() == "1")
            ),
            "a rejected delete must not drop the tombstone: {writes:?}",
        );
    }

    #[test]
    fn local_delete_gone_remote_just_drops() {
        let mut local = synced("1", &["seen"]);
        local.status = PimdirStatus::Tombstone;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) = run(&mut sync, vec![local], vec![]);

        assert!(pushes.is_none());
        assert_eq!(report.pushed, 0);
        assert!(
            writes.iter().any(
                |w| matches!(w, PimdirWriteOp::DropPlacement { handle, .. } if handle.as_str() == "1")
            ),
            "the tombstone is dropped without a push: {writes:?}",
        );
    }

    #[test]
    fn remote_delete_in_full_drops() {
        let local = synced("1", &["seen"]);
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) = run(&mut sync, vec![local], vec![]);

        assert!(pushes.is_none());
        assert_eq!(report.pulled, 1);
        assert!(
            writes.iter().any(
                |w| matches!(w, PimdirWriteOp::DropPlacement { handle, .. } if handle.as_str() == "1")
            ),
            "the vanished placement is dropped: {writes:?}",
        );
    }

    /// A create-collision conflict whose remote side went keeps its body,
    /// re-staged as a pending create the next run adds (SYNC §5).
    #[test]
    fn a_base_less_body_absent_from_the_remote_resurrects_as_a_create() {
        let mut placement = synced("1", &[]);
        placement.base = None;
        placement.object = Some(PimdirHash::from("h1"));
        placement.level = PimdirLevel::Full;
        placement.status = PimdirStatus::Conflict;
        placement.conflict_revision = Some("r9".into());

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) = run(&mut sync, vec![placement], vec![]);

        assert!(pushes.is_none(), "the next run adds it");
        assert_eq!(report.events, [PimdirSyncEvent::Vanished("1".into())]);
        assert!(
            writes.iter().any(
                |w| matches!(w, PimdirWriteOp::DropPlacement { handle, reason, .. } if handle.as_str() == "1" && *reason == PimdirDropReason::Superseded)
            ),
            "the vanished handle is superseded by the provisional one: {writes:?}",
        );
        let resurrected = upserted(&writes, "\u{1}1").expect("a re-staged create");
        assert_eq!(resurrected.status, PimdirStatus::Created);
        assert_eq!(resurrected.conflict_revision, None);
        assert_eq!(resurrected.object, Some(PimdirHash::from("h1")));
    }

    /// A probe the enumeration no longer lists is gone like any member.
    #[test]
    fn a_probe_absent_from_a_complete_enumeration_is_dropped() {
        let mut probe = synced("1", &["flagged"]);
        probe.link_id = None;
        probe.base = None;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) = run(&mut sync, vec![probe], vec![]);

        assert!(pushes.is_none());
        assert_eq!(report.pulled, 1);
        assert_eq!(report.events, [PimdirSyncEvent::Vanished("1".into())]);
        assert!(
            writes.iter().any(
                |w| matches!(w, PimdirWriteOp::DropPlacement { handle, reason, .. } if handle.as_str() == "1" && *reason == PimdirDropReason::Deleted)
            ),
            "the probe is dropped: {writes:?}",
        );
    }

    /// The Add carries the origin (copy, not re-upload) and the flag set.
    #[test]
    fn created_placement_pushes_add() {
        let mut local = created("tmp-1");
        local.flags = PimdirFlags::from_iter(["seen"]);
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, _writes, report) = run(&mut sync, vec![local], vec![]);

        match &pushes.expect("a push")[0].kind {
            PimdirChangeKind::Add { origin, flags, .. } => {
                assert!(origin.is_some());
                assert!(flags.contains("seen"), "the flag set rides the add");
            }
            other => panic!("expected an Add push, got {other:?}"),
        }
        assert_eq!(report.pushed, 1);
    }

    /// The placeholder is dropped and the placement rekeyed clean and based.
    #[test]
    fn accepted_create_rekeys_to_assigned() {
        let mut local = created("tmp-1");
        local.object = Some(PimdirHash::from("h-1"));
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let results = vec![PimdirPushResult {
            handle: PimdirHandle::from("tmp-1"),
            outcome: PimdirPushOutcome::Accepted,
            assigned: Some(PimdirHandle::from("42")),
            revision: Some("r1".into()),
        }];
        let (writes, _report) = run_push(&mut sync, vec![local], vec![], results);

        assert!(
            writes.iter().any(
                |w| matches!(w, PimdirWriteOp::DropPlacement { handle, .. } if handle.as_str() == "tmp-1")
            ),
            "the placeholder is dropped: {writes:?}",
        );
        let real =
            upserted(&writes, "42").expect("the placement is rekeyed to the assigned handle");
        assert_eq!(real.status, PimdirStatus::Clean);
        assert!(real.origin.is_none());
        let base = real.base.as_ref().expect("a base");
        assert_eq!(base.revision.as_deref(), Some("r1"));
        assert_eq!(base.object, Some(PimdirHash::from("h-1")));
    }

    #[test]
    fn rejected_create_keeps_placeholder() {
        let local = created("tmp-1");
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let results = vec![PimdirPushResult {
            handle: PimdirHandle::from("tmp-1"),
            outcome: PimdirPushOutcome::Rejected,
            assigned: None,
            revision: None,
        }];
        let (writes, report) = run_push(&mut sync, vec![local], vec![], results);

        assert_eq!(report.rejected, 1);
        assert!(
            !writes
                .iter()
                .any(|w| matches!(w, PimdirWriteOp::DropPlacement { .. })),
            "a rejected create must not drop the placeholder: {writes:?}",
        );
        assert!(upserted(&writes, "tmp-1").is_none());
    }

    /// A tombstone carrying an origin is a move: the Remove names the target.
    #[test]
    fn move_pushes_remove_with_target() {
        let mut local = synced("1", &["seen"]);
        local.status = PimdirStatus::Tombstone;
        local.origin = Some(PimdirOrigin {
            collection: "archive".into(),
            handle: PimdirHandle::from("1"),
        });

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, _writes, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

        match &pushes.expect("a push")[0].kind {
            PimdirChangeKind::Remove { to: Some(to), .. } => assert_eq!(to.as_str(), "archive"),
            other => panic!("expected a move Remove, got {other:?}"),
        }
        assert_eq!(report.pushed, 1);
    }

    /// Without an assigned handle (no UIDPLUS) the next enumerate re-adds it.
    #[test]
    fn accepted_create_without_assigned_drops_placeholder() {
        let local = created("tmp-1");
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let results = vec![PimdirPushResult {
            handle: PimdirHandle::from("tmp-1"),
            outcome: PimdirPushOutcome::Accepted,
            assigned: None,
            revision: None,
        }];
        let (writes, _report) = run_push(&mut sync, vec![local], vec![], results);

        assert!(
            writes.iter().any(
                |w| matches!(w, PimdirWriteOp::DropPlacement { handle, .. } if handle.as_str() == "tmp-1")
            ),
            "the placeholder is dropped once the copy lands: {writes:?}",
        );
        assert!(upserted(&writes, "tmp-1").is_none());
    }

    #[test]
    fn full_sync_ignores_checkpoint() {
        let mut sync = PimdirSync::new(
            "inbox",
            PimdirSyncOptions {
                push: true,
                rights: PimdirPushRights::all(),
                conflict: PimdirConflictPolicy::Manual,
                full: true,
            },
        );
        let _ = sync.resume(None);
        let loaded = PimdirLoaded {
            placements: Vec::new(),
            checkpoint: Some(PimdirCheckpoint(b"cp".to_vec())),
        };
        match sync.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsEnumerate { cursor, .. }) => {
                assert!(cursor.is_none(), "a full sync must ignore the checkpoint");
            }
            state => panic!("expected WantsEnumerate, got {state:?}"),
        }
    }

    /// A synced placement with a staged edit: body "h2", base "h1" at "r1".
    fn edited(handle: &str) -> PimdirPlacement {
        let mut placement = synced(handle, &[]);
        placement.status = PimdirStatus::Dirty;
        placement.object = Some(PimdirHash::from("h2"));
        placement.level = PimdirLevel::Full;
        let base = placement.base.as_mut().expect("a base");
        base.revision = Some("r1".into());
        base.object = Some(PimdirHash::from("h1"));
        placement
    }

    /// A remote item at the given content revision.
    fn remote_rev(handle: &str, revision: &str) -> PimdirRemoteItem {
        let mut item = remote(handle, &[]);
        item.revision = Some(revision.into());
        item
    }

    /// A delta lists a flag change once, so the content axis must not eat it.
    ///
    /// The accepted push rebases the row the flag merge wrote, never the one
    /// read before it (SYNC §5): the last write of the handle holds the flag.
    #[test]
    fn a_content_push_still_pulls_a_remote_flag_change() {
        let mut local = edited("1");
        let mut item = remote_rev("1", "r1");
        item.flags = PimdirFlags::from_iter(["seen"]);
        local.base.as_mut().expect("a base").revision = Some("r1".into());

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, _report) = run(&mut sync, vec![local], vec![item]);

        match &pushes.expect("a push")[0].kind {
            PimdirChangeKind::Update { .. } => {}
            other => panic!("expected an Update push, got {other:?}"),
        }
        let rebased = upserted(&writes, "1").expect("the rebased placement");
        assert!(
            rebased.flags.contains("seen"),
            "the remote flag lands with the content push, not a run later",
        );
        assert_eq!(rebased.status, PimdirStatus::Clean);
        let base = rebased.base.as_ref().expect("a base");
        assert!(base.flags.contains("seen"), "the pulled flag is agreed on");
        assert_eq!(base.object, Some(PimdirHash::from("h2")));
    }

    /// The Update is gated on the base revision, which then adopts the result.
    #[test]
    fn local_content_edit_pushes_update_and_rebases() {
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let _ = sync.resume(None);
        let _ = sync.resume(Some(PimdirArg::Load(PimdirLoaded {
            placements: vec![edited("1")],
            checkpoint: None,
        })));

        let pushes = match sync.resume(Some(PimdirArg::Enumerate(full(vec![remote_rev(
            "1", "r1",
        )])))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsPush { changes, .. }) => changes,
            state => panic!("expected WantsPush, got {state:?}"),
        };
        match &pushes[0].kind {
            PimdirChangeKind::Update {
                handle,
                object,
                if_match,
            } => {
                assert_eq!(handle.as_str(), "1");
                assert_eq!(object, &PimdirHash::from("h2"));
                assert_eq!(if_match.as_deref(), Some("r1"));
            }
            other => panic!("expected an Update push, got {other:?}"),
        }

        let results = vec![PimdirPushResult {
            handle: PimdirHandle::from("1"),
            outcome: PimdirPushOutcome::Accepted,
            assigned: None,
            revision: Some("r2".into()),
        }];
        let writes = match sync.resume(Some(PimdirArg::Push(results))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(writes)) => writes,
            state => panic!("expected WantsWrite, got {state:?}"),
        };

        let rebased = upserted(&writes, "1").expect("a rebased placement");
        assert_eq!(rebased.status, PimdirStatus::Clean);
        let base = rebased.base.as_ref().expect("a base");
        assert_eq!(base.revision.as_deref(), Some("r2"));
        assert_eq!(base.object, Some(PimdirHash::from("h2")));
    }

    /// The content rebase keeps the base flags, so the flag push derives later.
    #[test]
    fn content_rebase_defers_a_riding_flag_edit() {
        let mut placement = edited("1");
        placement.flags = PimdirFlags::from_iter(["seen"]);

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let results = vec![PimdirPushResult {
            handle: PimdirHandle::from("1"),
            outcome: PimdirPushOutcome::Accepted,
            assigned: None,
            revision: Some("r2".into()),
        }];
        let (writes, _report) = run_push(
            &mut sync,
            vec![placement],
            vec![remote_rev("1", "r1")],
            results,
        );

        let rebased = upserted(&writes, "1").expect("a rebased placement");
        assert_eq!(
            rebased.status,
            PimdirStatus::Dirty,
            "the flag edit stays pending"
        );
        let base = rebased.base.as_ref().expect("a base");
        assert!(!base.flags.contains("seen"), "base flags stay as synced");
        assert_eq!(base.object, Some(PimdirHash::from("h2")));
    }

    #[test]
    fn remote_content_change_refreshes_the_stale_body() {
        let mut placement = synced("1", &[]);
        placement.object = Some(PimdirHash::from("h1"));
        placement.level = PimdirLevel::Full;
        let base = placement.base.as_mut().expect("a base");
        base.revision = Some("r1".into());
        base.object = Some(PimdirHash::from("h1"));

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) = run(&mut sync, vec![placement], vec![remote_rev("1", "r2")]);

        assert!(pushes.is_none(), "a refresh pushes nothing");
        assert_eq!(report.refreshed, 1);
        let refreshed = upserted(&writes, "1").expect("a refreshed placement");
        assert_eq!(refreshed.object, None, "the stale body is dropped");
        assert_eq!(
            refreshed.level,
            PimdirLevel::Probed,
            "the summary is stale too"
        );
        let base = refreshed.base.as_ref().expect("a base");
        assert_eq!(base.revision.as_deref(), Some("r2"));
        assert_eq!(base.object, None);
    }

    /// The mark carries the observed revision; the upgrade fetches the body.
    #[test]
    fn divergent_content_edits_conflict() {
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) =
            run(&mut sync, vec![edited("1")], vec![remote_rev("1", "r2")]);

        assert!(pushes.is_none());
        assert_eq!(report.conflicts, 1);
        assert_eq!(report.refreshed, 0);
        let conflicted = upserted(&writes, "1").expect("a conflicted placement");
        assert_eq!(conflicted.status, PimdirStatus::Conflict);
        assert_eq!(conflicted.conflict_revision.as_deref(), Some("r2"));
        assert_eq!(
            conflicted.conflict_object, None,
            "the diverging body is wanted, not taken"
        );
        assert_eq!(
            conflicted.object,
            Some(PimdirHash::from("h2")),
            "the edit survives"
        );
    }

    /// With no shared ancestor there is nothing to merge, so it conflicts.
    ///
    /// Converging on flags alone would strand the two bodies apart and
    /// loop every sync; the consumer's resolution re-establishes a base.
    #[test]
    fn base_less_body_present_on_both_conflicts() {
        let mut placement = synced("1", &[]);
        placement.base = None;
        placement.object = Some(PimdirHash::from("h1"));
        placement.level = PimdirLevel::Full;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) = run(&mut sync, vec![placement], vec![remote_rev("1", "r9")]);

        assert!(pushes.is_none(), "an unresolved conflict pushes nothing");
        assert_eq!(report.conflicts, 1);
        let conflicted = upserted(&writes, "1").expect("a conflicted placement");
        assert_eq!(conflicted.status, PimdirStatus::Conflict);
        assert_eq!(conflicted.conflict_revision.as_deref(), Some("r9"));
        assert_eq!(
            conflicted.object,
            Some(PimdirHash::from("h1")),
            "the body survives for the resolution"
        );
    }

    /// No body and no base is a probe: agreeing on the flags, it is left alone.
    #[test]
    fn base_less_body_less_present_on_both_stays_flag_only() {
        let mut placement = synced("1", &["seen"]);
        placement.base = None;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) =
            run(&mut sync, vec![placement], vec![remote("1", &["seen"])]);

        assert!(pushes.is_none());
        assert_eq!(report, PimdirSyncReport::default(), "no conflict, no pull");
        assert!(
            upserted(&writes, "1").is_none(),
            "nothing to write: {writes:?}"
        );
    }

    #[test]
    fn an_unresolved_conflict_tracks_the_latest_remote_revision() {
        let mut placement = edited("1");
        placement.status = PimdirStatus::Conflict;
        placement.conflict_revision = Some("r2".into());

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) = run(&mut sync, vec![placement], vec![remote_rev("1", "r3")]);

        assert!(pushes.is_none());
        assert_eq!(report.conflicts, 0, "no recount");
        let tracked = upserted(&writes, "1").expect("an updated placement");
        assert_eq!(tracked.status, PimdirStatus::Conflict);
        assert_eq!(tracked.conflict_revision.as_deref(), Some("r3"));
        assert_eq!(
            tracked.object,
            Some(PimdirHash::from("h2")),
            "the edit survives"
        );
    }

    /// The stored body described the old revision, a resolver must not see it.
    #[test]
    fn a_conflict_whose_remote_moved_drops_its_stored_body() {
        let mut placement = edited("1");
        placement.status = PimdirStatus::Conflict;
        placement.conflict_revision = Some("r2".into());
        placement.conflict_object = Some(PimdirHash::from("h-r2"));

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (_pushes, writes, _report) =
            run(&mut sync, vec![placement], vec![remote_rev("1", "r3")]);

        let tracked = upserted(&writes, "1").expect("an updated placement");
        assert_eq!(tracked.conflict_revision.as_deref(), Some("r3"));
        assert_eq!(
            tracked.conflict_object, None,
            "the body of the revision that moved is asked for anew"
        );
    }

    /// The kept ancestor pushes, gated on the revision it was decided against.
    #[test]
    fn a_resolution_keeping_the_ancestor_pushes_it() {
        let mut placement = edited("1");
        placement.object = Some(PimdirHash::from("h-base"));
        let base = placement.base.as_mut().expect("a base");
        base.revision = Some("r2".into());
        base.object = Some(PimdirHash::from("h-remote"));

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, _writes, report) =
            run(&mut sync, vec![placement], vec![remote_rev("1", "r2")]);

        assert_eq!(report.conflicts, 0);
        match &pushes.expect("a push")[0].kind {
            PimdirChangeKind::Update {
                object, if_match, ..
            } => {
                assert_eq!(object, &PimdirHash::from("h-base"));
                assert_eq!(if_match.as_deref(), Some("r2"));
            }
            other => panic!("expected an Update push, got {other:?}"),
        }
    }

    #[test]
    fn a_resolution_taking_the_remote_body_settles_clean() {
        let mut placement = edited("1");
        placement.object = Some(PimdirHash::from("h-remote"));
        let base = placement.base.as_mut().expect("a base");
        base.revision = Some("r2".into());
        base.object = Some(PimdirHash::from("h-remote"));

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) = run(&mut sync, vec![placement], vec![remote_rev("1", "r2")]);

        assert!(pushes.is_none(), "the remote holds the decision already");
        assert_eq!(report.conflicts, 0);
        let settled = upserted(&writes, "1").expect("a settled placement");
        assert_eq!(settled.status, PimdirStatus::Clean);
        assert_eq!(settled.object, Some(PimdirHash::from("h-remote")));
    }

    /// No revision means no content signal, so neither side is ever written.
    #[test]
    fn an_immutable_backend_records_no_conflict_at_all() {
        let mut placement = edited("1");
        let base = placement.base.as_mut().expect("a base");
        base.revision = None;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (_pushes, writes, report) = run(&mut sync, vec![placement], vec![remote("1", &[])]);

        assert_eq!(report.conflicts, 0);
        for write in &writes {
            let PimdirWriteOp::UpsertPlacement(placement) = write else {
                continue;
            };
            assert_ne!(placement.status, PimdirStatus::Conflict);
            assert_eq!(placement.conflict_revision, None);
            assert_eq!(placement.conflict_object, None);
        }
    }

    /// A delta lists a flag change once, so the conflict mark must not eat it.
    #[test]
    fn a_content_conflict_still_pulls_the_remote_flag_change() {
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let mut item = remote_rev("1", "r2");
        item.flags = PimdirFlags::from_iter(["seen"]);
        let (pushes, writes, report) = run(&mut sync, vec![edited("1")], vec![item]);

        assert!(pushes.is_none());
        assert_eq!(report.conflicts, 1);
        let conflicted = writes
            .iter()
            .rev()
            .find_map(|w| match w {
                PimdirWriteOp::UpsertPlacement(p) if p.handle.as_str() == "1" => Some(p),
                _ => None,
            })
            .expect("a conflict write");
        assert_eq!(conflicted.status, PimdirStatus::Conflict);
        assert_eq!(conflicted.conflict_revision.as_deref(), Some("r2"));
        assert!(conflicted.flags.contains("seen"), "the flag change lands");
        assert_eq!(
            conflicted.object,
            Some(PimdirHash::from("h2")),
            "the edit survives"
        );
    }

    /// The synthesized remote state carries the observed conflict revision.
    #[test]
    fn unlisted_conflict_keeps_its_observed_remote_revision() {
        let mut placement = edited("1");
        placement.status = PimdirStatus::Conflict;
        placement.conflict_revision = Some("r2".into());

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let snapshot = delta(vec![], vec![]);
        let (pushes, writes, report) = run_snapshot(&mut sync, vec![placement], snapshot);

        assert!(pushes.is_none());
        assert_eq!(report.conflicts, 0, "no recount");
        assert!(
            upserted(&writes, "1").is_none(),
            "the conflict tracking must not regress to the base revision: {writes:?}",
        );
    }

    /// A remote edit over a plain tombstone revives it and pulls (SYNC §5).
    #[test]
    fn remote_content_change_beats_a_local_delete() {
        let mut placement = synced("1", &[]);
        placement.base = Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: Some("r1".into()),
            object: Some(PimdirHash::from("h1")),
        });
        placement.object = Some(PimdirHash::from("h1"));
        placement.status = PimdirStatus::Tombstone;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) = run(&mut sync, vec![placement], vec![remote_rev("1", "r2")]);

        assert!(pushes.is_none(), "the delete is not pushed");
        assert_eq!(report.pulled, 1);
        let resurrected = upserted(&writes, "1").expect("a resurrected placement");
        assert_eq!(resurrected.status, PimdirStatus::Clean);
        let base = resurrected.base.as_ref().expect("a base");
        assert_eq!(base.revision.as_deref(), Some("r2"));
    }

    /// The edit rides into the target's create, where an Update would race it.
    #[test]
    fn a_tombstone_carrying_a_staged_edit_still_removes() {
        let mut placement = edited("1");
        placement.status = PimdirStatus::Tombstone;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, _writes, _report) =
            run(&mut sync, vec![placement], vec![remote_rev("1", "r1")]);

        match &pushes.expect("a push")[0].kind {
            PimdirChangeKind::Remove { to, if_match, .. } => {
                assert_eq!(*to, None, "a plain remove, not a server-side move");
                assert_eq!(if_match.as_deref(), Some("r1"));
            }
            other => panic!("expected a Remove, got {other:?}"),
        }
    }

    /// A storage plumbing a tombstone origin through still derives the target.
    #[test]
    fn a_tombstone_origin_derives_a_move_remove() {
        let mut placement = synced("1", &[]);
        placement.status = PimdirStatus::Tombstone;
        placement.origin = Some(PimdirOrigin {
            collection: "archive".into(),
            handle: PimdirHandle::from("1"),
        });

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, _writes, _report) = run(&mut sync, vec![placement], vec![remote("1", &[])]);

        match &pushes.expect("a push")[0].kind {
            PimdirChangeKind::Remove { to: Some(to), .. } => assert_eq!(to.as_str(), "archive"),
            other => panic!("expected a move Remove, got {other:?}"),
        }
    }

    #[test]
    fn remove_carries_the_base_revision_as_precondition() {
        let mut placement = edited("1");
        placement.status = PimdirStatus::Tombstone;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, _writes, _report) =
            run(&mut sync, vec![placement], vec![remote_rev("1", "r1")]);

        match &pushes.expect("a push")[0].kind {
            PimdirChangeKind::Remove { if_match, .. } => {
                assert_eq!(if_match.as_deref(), Some("r1"))
            }
            other => panic!("expected a Remove push, got {other:?}"),
        }
    }

    /// A vanished member holding a local edit is re-staged as a pending
    /// create under the provisional handle, which the next run adds (SYNC §5).
    #[test]
    fn remote_delete_with_staged_edit_resurrects_as_create() {
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) = run(&mut sync, vec![edited("1")], vec![]);

        assert!(pushes.is_none(), "the next run adds it");
        assert_eq!(report.events, [PimdirSyncEvent::Vanished("1".into())]);
        let resurrected = upserted(&writes, "\u{1}1").expect("a re-staged create");
        assert_eq!(resurrected.status, PimdirStatus::Created);
        assert!(resurrected.base.is_none());
        assert!(resurrected.origin.is_none(), "an append, not a copy");
        assert_eq!(resurrected.object, Some(PimdirHash::from("h2")));

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, _writes, _report) = run(&mut sync, vec![resurrected.clone()], vec![]);
        match &pushes.expect("the add")[0].kind {
            PimdirChangeKind::Add { object, origin, .. } => {
                assert_eq!(object, &Some(PimdirHash::from("h2")), "the edited body");
                assert!(origin.is_none(), "an append, not a copy");
            }
            other => panic!("expected an Add push, got {other:?}"),
        }
    }

    /// The remote side is gone, so the conflict is moot and the edit survives.
    #[test]
    fn remote_delete_of_a_conflicted_placement_resurrects_the_edit() {
        let mut placement = edited("1");
        placement.status = PimdirStatus::Conflict;
        placement.conflict_revision = Some("r2".into());

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, _report) = run(&mut sync, vec![placement], vec![]);

        assert!(pushes.is_none(), "the next run adds it");
        let resurrected = upserted(&writes, "\u{1}1").expect("a re-staged create");
        assert_eq!(resurrected.status, PimdirStatus::Created);
        assert_eq!(resurrected.conflict_revision, None, "the conflict is moot");
        assert_eq!(
            resurrected.object,
            Some(PimdirHash::from("h2")),
            "the edit survives"
        );
    }

    /// No push on a read-only source, but the pending create keeps the edit.
    #[test]
    fn read_only_remote_delete_with_staged_edit_keeps_the_edit() {
        let opts = PimdirSyncOptions {
            push: false,
            rights: PimdirPushRights::all(),
            conflict: PimdirConflictPolicy::Manual,
            full: false,
        };
        let mut sync = PimdirSync::new("inbox", opts);
        let (pushes, writes, _report) = run(&mut sync, vec![edited("1")], vec![]);

        assert!(pushes.is_none());
        let resurrected = upserted(&writes, "\u{1}1").expect("a re-staged create");
        assert_eq!(resurrected.status, PimdirStatus::Created);
        assert_eq!(resurrected.object, Some(PimdirHash::from("h2")));
    }

    #[test]
    fn read_only_keeps_a_content_edit_dirty() {
        let mut sync = PimdirSync::new(
            "inbox",
            PimdirSyncOptions {
                push: false,
                rights: PimdirPushRights::all(),
                conflict: PimdirConflictPolicy::Manual,
                full: false,
            },
        );
        let (pushes, writes, _report) =
            run(&mut sync, vec![edited("1")], vec![remote_rev("1", "r1")]);

        assert!(pushes.is_none());
        assert!(
            upserted(&writes, "1").is_none(),
            "the placement is left as is"
        );
    }

    /// The member stays with its cached body rather than being refetched later.
    #[test]
    fn read_only_delete_is_reverted_rather_than_applied() {
        let mut local = synced("1", &["seen"]);
        local.status = PimdirStatus::Tombstone;

        let opts = PimdirSyncOptions {
            push: false,
            rights: PimdirPushRights::all(),
            conflict: PimdirConflictPolicy::Manual,
            full: false,
        };
        let mut sync = PimdirSync::new("inbox", opts);
        let (pushes, writes, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

        assert!(pushes.is_none(), "read-only source must not push");
        assert_eq!(report.pushed, 0);
        assert!(
            !writes
                .iter()
                .any(|w| matches!(w, PimdirWriteOp::DropPlacement { .. })),
            "the member is not dropped: {writes:?}",
        );
        let reverted = upserted(&writes, "1").expect("the reverted placement");
        assert_eq!(reverted.status, PimdirStatus::Clean);
    }

    /// A delta never re-lists an untouched member, so the revert cannot wait.
    #[test]
    fn a_read_only_delete_survives_a_delta_enumerate() {
        let mut local = synced("1", &["seen"]);
        local.status = PimdirStatus::Tombstone;

        let opts = PimdirSyncOptions {
            push: false,
            ..Default::default()
        };
        let mut sync = PimdirSync::new("inbox", opts);
        let (_pushes, writes, _report) =
            run_snapshot(&mut sync, vec![local], delta(vec![], vec![]));

        let reverted = upserted(&writes, "1").expect("the reverted placement");
        assert_eq!(reverted.status, PimdirStatus::Clean);
    }

    /// A delta may report a handle removed before the replica ever knew it.
    #[test]
    fn unknown_vanished_handle_is_ignored() {
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let snapshot = delta(vec![], vec![PimdirHandle::from("ghost")]);
        let (pushes, writes, report) = run_snapshot(&mut sync, vec![], snapshot);

        assert!(pushes.is_none());
        assert_eq!(report, PimdirSyncReport::default());
        assert_eq!(writes.len(), 1, "only the checkpoint write");
        assert!(matches!(&writes[0], PimdirWriteOp::SetCheckpoint { .. }));
    }

    #[test]
    fn noop_flag_edit_rebases_clean() {
        let mut local = synced("1", &["seen"]);
        local.status = PimdirStatus::Dirty;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

        assert!(pushes.is_none(), "nothing to push");
        assert_eq!(report, PimdirSyncReport::default());
        let cleaned = upserted(&writes, "1").expect("a cleaning rebase");
        assert_eq!(cleaned.status, PimdirStatus::Clean);
    }

    #[test]
    fn missing_arg_errors() {
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let _ = sync.resume(None);
        match sync.resume(None) {
            PimdirCoroutineState::Complete(Err(PimdirArgError::MissingArg)) => {}
            state => panic!("expected MissingArg, got {state:?}"),
        }
    }

    /// An empty report reads like a run that did nothing, so resuming errors.
    #[test]
    fn a_completed_sync_does_not_resume() {
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let _ = run(&mut sync, vec![], vec![]);
        match sync.resume(Some(PimdirArg::Write)) {
            PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => {}
            state => panic!("expected UnexpectedArg, got {state:?}"),
        }
        match sync.resume(None) {
            PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => {}
            state => panic!("expected UnexpectedArg, got {state:?}"),
        }
    }

    /// A probe the enumeration lists again as the store holds it is no pull.
    #[test]
    fn a_relisted_probe_with_the_same_flags_derives_nothing() {
        let mut probe = synced("1", &["seen"]);
        probe.link_id = None;
        probe.base = None;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) = run(&mut sync, vec![probe], vec![remote("1", &["seen"])]);

        assert!(pushes.is_none());
        assert_eq!(report, PimdirSyncReport::default(), "nothing pulled");
        assert!(
            upserted(&writes, "1").is_none(),
            "the probe is not rewritten: {writes:?}",
        );
    }

    /// A source alone reverts a delete its rights refuse; the std client says
    /// when it is not alone.
    #[test]
    fn a_refused_delete_is_reverted_for_a_lone_source() {
        let mut local = synced("1", &["seen"]);
        local.status = PimdirStatus::Tombstone;
        let mut sync = PimdirSync::new("inbox", with_rights(true, true, true, false));
        let (_pushes, writes, _report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

        assert_eq!(
            upserted(&writes, "1")
                .expect("the reverted tombstone")
                .status,
            PimdirStatus::Clean,
        );
    }

    #[test]
    fn unexpected_arg_errors() {
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let _ = sync.resume(None);
        match sync.resume(Some(PimdirArg::Write)) {
            PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => {}
            state => panic!("expected UnexpectedArg, got {state:?}"),
        }
    }

    /// A writable sync (`push = true`) with the given per-kind rights.
    fn with_rights(flags: bool, content: bool, add: bool, remove: bool) -> PimdirSyncOptions {
        PimdirSyncOptions {
            rights: PimdirPushRights {
                flags,
                content,
                add,
                remove,
            },
            ..Default::default()
        }
    }

    #[test]
    fn forbidding_flags_keeps_dirty_without_push() {
        let mut local = synced("1", &[]);
        local.flags = PimdirFlags::from_iter(["seen"]);
        local.status = PimdirStatus::Dirty;

        let mut sync = PimdirSync::new("inbox", with_rights(false, true, true, true));
        let (pushes, _writes, report) = run(&mut sync, vec![local], vec![remote("1", &[])]);

        assert!(pushes.is_none(), "a forbidden flag push must not fire");
        assert_eq!(report.pushed, 0);
    }

    #[test]
    fn forbidding_remove_reverts_the_tombstone_by_default() {
        let mut local = synced("1", &["seen"]);
        local.status = PimdirStatus::Tombstone;

        let mut sync = PimdirSync::new("inbox", with_rights(true, true, true, false));
        let (pushes, writes, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);

        assert!(pushes.is_none(), "a forbidden remove must not push");
        assert!(
            !writes.iter().any(|w| matches!(
                w,
                PimdirWriteOp::DropPlacement { handle, .. } if handle.as_str() == "1"
            )),
            "a delete the source refuses is never applied to the replica: {writes:?}",
        );
        assert_eq!(
            upserted(&writes, "1")
                .expect("the tombstone is reverted")
                .status,
            PimdirStatus::Clean,
            "the default policy mirrors the source, as it does for a read-only one",
        );
        assert_eq!(report.pushed, 0);
    }

    /// Beside another source, rights `none()` and `push = false` agree on what
    /// a refused delete does: the tombstone is held, the other source pushing it.
    #[test]
    fn a_refused_delete_beside_other_sources_holds_the_tombstone_either_way() {
        let mut local = synced("1", &["seen"]);
        local.status = PimdirStatus::Tombstone;

        let forbidden = PimdirSyncOptions {
            rights: PimdirPushRights {
                remove: false,
                ..PimdirPushRights::all()
            },
            ..Default::default()
        };
        let read_only = PimdirSyncOptions {
            push: false,
            ..Default::default()
        };

        for opts in [forbidden, read_only] {
            let mut sync = PimdirSync::new("inbox", opts).beside_other_sources(true);
            let (pushes, writes, _report) =
                run(&mut sync, vec![local.clone()], vec![remote("1", &["seen"])]);

            assert!(pushes.is_none(), "a refused delete must not push");
            assert!(
                upserted(&writes, "1").is_none(),
                "the tombstone is held as it is, for a later run: {writes:?}",
            );
        }
    }

    #[test]
    fn flags_allowed_remove_forbidden_pushes_only_flags() {
        let mut dirty = synced("1", &[]);
        dirty.flags = PimdirFlags::from_iter(["seen"]);
        dirty.status = PimdirStatus::Dirty;
        let mut tomb = synced("2", &[]);
        tomb.status = PimdirStatus::Tombstone;

        let mut sync = PimdirSync::new("inbox", with_rights(true, true, true, false));
        let (pushes, _writes, _report) = run(
            &mut sync,
            vec![dirty, tomb],
            vec![remote("1", &[]), remote("2", &[])],
        );

        let pushes = pushes.expect("the permitted flag push still fires");
        assert!(
            pushes
                .iter()
                .all(|c| matches!(c.kind, PimdirChangeKind::SetFlags { .. })),
            "only the flag change may be pushed, not the delete: {pushes:?}",
        );
    }

    #[test]
    fn forbidding_add_keeps_created_pending() {
        let mut sync = PimdirSync::new("inbox", with_rights(true, true, false, true));
        let (pushes, _writes, report) = run(&mut sync, vec![created("tmp")], vec![]);

        assert!(pushes.is_none(), "a forbidden add must not push the create");
        assert_eq!(report.pushed, 0);
    }

    #[test]
    fn event_added_on_remote_add() {
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (_p, _w, report) = run(&mut sync, vec![], vec![remote("1", &["seen"])]);
        assert_eq!(
            report.events,
            vec![PimdirSyncEvent::Added(PimdirHandle::from("1"))]
        );
    }

    #[test]
    fn event_flags_changed_on_remote_flag_pull() {
        let local = synced("1", &[]);
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (_p, _w, report) = run(&mut sync, vec![local], vec![remote("1", &["seen"])]);
        assert_eq!(
            report.events,
            vec![PimdirSyncEvent::FlagsChanged(PimdirHandle::from("1"))]
        );
    }

    #[test]
    fn event_vanished_on_delta_vanish() {
        let local = synced("1", &["seen"]);
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let snapshot = delta(vec![], vec![PimdirHandle::from("1")]);
        let (_p, _w, report) = run_snapshot(&mut sync, vec![local], snapshot);
        assert_eq!(
            report.events,
            vec![PimdirSyncEvent::Vanished(PimdirHandle::from("1"))]
        );
    }

    /// The consumer made the change, so nothing is reported back to it.
    #[test]
    fn an_accepted_flag_push_reports_no_event() {
        let mut local = synced("1", &[]);
        local.flags = PimdirFlags::from_iter(["seen"]);
        local.status = PimdirStatus::Dirty;
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (_p, _w, report) = run(&mut sync, vec![local], vec![remote("1", &[])]);
        assert!(report.events.is_empty(), "{:?}", report.events);
        assert_eq!(report.pushed, 1);
    }

    #[test]
    fn event_created_on_accepted_create() {
        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let results = vec![PimdirPushResult {
            handle: PimdirHandle::from("tmp"),
            outcome: PimdirPushOutcome::Accepted,
            assigned: Some(PimdirHandle::from("99")),
            revision: None,
        }];
        let (_w, report) = run_push(&mut sync, vec![created("tmp")], vec![], results);
        assert_eq!(
            report.events,
            vec![PimdirSyncEvent::Created(PimdirHandle::from("99"))]
        );
    }

    /// Sync options with a conflict policy, everything else default.
    fn with_conflict(policy: PimdirConflictPolicy) -> PimdirSyncOptions {
        PimdirSyncOptions {
            conflict: policy,
            ..Default::default()
        }
    }

    #[test]
    fn prefer_remote_drops_the_local_edit() {
        let mut sync = PimdirSync::new("inbox", with_conflict(PimdirConflictPolicy::PreferRemote));
        let (pushes, writes, report) =
            run(&mut sync, vec![edited("1")], vec![remote_rev("1", "r2")]);

        assert!(pushes.is_none(), "prefer-remote pulls, never pushes");
        assert_eq!(report.conflicts, 0);
        assert_eq!(report.refreshed, 1, "the remote content is pulled");
        let pulled = upserted(&writes, "1").expect("a pulled placement");
        assert_eq!(pulled.object, None, "the local edit is dropped");
        assert_eq!(pulled.level, PimdirLevel::Probed);
    }

    #[test]
    fn prefer_local_overwrites_the_remote() {
        let mut sync = PimdirSync::new("inbox", with_conflict(PimdirConflictPolicy::PreferLocal));
        let (pushes, _writes, report) =
            run(&mut sync, vec![edited("1")], vec![remote_rev("1", "r2")]);

        let pushes = pushes.expect("prefer-local pushes the edit");
        match &pushes[0].kind {
            PimdirChangeKind::Update {
                object, if_match, ..
            } => {
                assert_eq!(object, &PimdirHash::from("h2"));
                assert_eq!(
                    if_match.as_deref(),
                    Some("r2"),
                    "overwrites the current remote revision, not the stale base",
                );
            }
            other => panic!("expected an Update push, got {other:?}"),
        }
        assert_eq!(report.conflicts, 0);
    }

    #[test]
    fn prefer_local_falls_back_to_conflict_when_it_cannot_push() {
        let opts = PimdirSyncOptions {
            conflict: PimdirConflictPolicy::PreferLocal,
            rights: PimdirPushRights {
                content: false,
                ..PimdirPushRights::all()
            },
            ..Default::default()
        };
        let mut sync = PimdirSync::new("inbox", opts);
        let (pushes, _writes, report) =
            run(&mut sync, vec![edited("1")], vec![remote_rev("1", "r2")]);

        assert!(pushes.is_none());
        assert_eq!(report.conflicts, 1, "no push right, so it stays a conflict");
    }

    /// A pending create waits while the collection holds a probe of its
    /// source: the probe may be its own arrival (SYNC §5).
    #[test]
    fn a_create_waits_while_the_collection_holds_probes() {
        let mut create = synced("\u{1}m1", &[]);
        create.link_id = Some(PimdirLinkId::from("m1"));
        create.object = Some(PimdirHash::from("h1"));
        create.level = PimdirLevel::Full;
        create.status = PimdirStatus::Created;
        create.base = None;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, _writes, _report) =
            run(&mut sync, vec![create.clone()], vec![remote("7", &[])]);
        assert!(pushes.is_none(), "the listed member is a probe until named");

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, _writes, _report) = run(&mut sync, vec![create], vec![]);
        assert!(
            matches!(
                &pushes.expect("the add")[0].kind,
                PimdirChangeKind::Add { .. }
            ),
            "with no probe left the create is pushed",
        );
    }

    /// A tombstone holding a local edit met by a remote edit is the
    /// both-changed case, not a blind revive (SYNC §5).
    #[test]
    fn a_tombstoned_local_edit_over_a_remote_edit_follows_the_policy() {
        let mut local = edited("1");
        local.status = PimdirStatus::Tombstone;

        let mut sync = PimdirSync::new("inbox", PimdirSyncOptions::default());
        let (pushes, writes, report) = run(&mut sync, vec![local], vec![remote_rev("1", "r2")]);

        assert!(pushes.is_none());
        assert_eq!(report.conflicts, 1);
        let conflicted = upserted(&writes, "1").expect("the conflicted placement");
        assert_eq!(conflicted.status, PimdirStatus::Conflict);
        assert_eq!(conflicted.staged_edit(), Some(&PimdirHash::from("h2")));
    }

    /// A reverted delete undoes the delete alone: the rest is still owed.
    #[test]
    fn a_reverted_delete_keeps_what_it_did_not_undo() {
        let read_only = PimdirSyncOptions {
            push: false,
            ..Default::default()
        };

        let mut edited_tomb = edited("1");
        edited_tomb.status = PimdirStatus::Tombstone;
        let mut sync = PimdirSync::new("inbox", read_only);
        let (_pushes, writes, _report) =
            run(&mut sync, vec![edited_tomb], vec![remote_rev("1", "r1")]);
        let reverted = upserted(&writes, "1").expect("the reverted tombstone");
        assert_eq!(reverted.status, PimdirStatus::Dirty);
        assert_eq!(reverted.staged_edit(), Some(&PimdirHash::from("h2")));

        let mut conflicted_tomb = edited("2");
        conflicted_tomb.status = PimdirStatus::Tombstone;
        conflicted_tomb.conflict_revision = Some("r2".into());
        let mut sync = PimdirSync::new("inbox", read_only);
        let (_pushes, writes, _report) = run(
            &mut sync,
            vec![conflicted_tomb],
            vec![remote_rev("2", "r1")],
        );
        let reverted = upserted(&writes, "2").expect("the reverted tombstone");
        assert_eq!(
            reverted.status,
            PimdirStatus::Conflict,
            "reverting the delete does not decide the divergence",
        );
    }

    /// Left behind, the destination would relocate the next plain delete.
    #[test]
    fn a_reverted_move_drops_the_destination_it_was_going_to() {
        let mut moved = synced("1", &[]);
        moved.status = PimdirStatus::Tombstone;
        moved.origin = Some(PimdirOrigin {
            collection: "archive".into(),
            handle: PimdirHandle::from("1"),
        });

        let mut sync = PimdirSync::new("inbox", with_rights(true, true, true, false));
        let (_pushes, writes, _report) = run(&mut sync, vec![moved], vec![remote("1", &[])]);

        let reverted = upserted(&writes, "1").expect("the reverted tombstone");
        assert_eq!(reverted.status, PimdirStatus::Clean);
        assert_eq!(reverted.origin, None);
    }

    /// A source refusing content pushes must not land the placement clean.
    #[test]
    fn a_flag_rebase_leaves_a_staged_edit_pending() {
        let mut local = edited("1");
        local.flags = PimdirFlags::from_iter(["seen"]);

        let mut sync = PimdirSync::new("inbox", with_rights(true, false, true, true));
        let (pushes, writes, _report) = run(&mut sync, vec![local], vec![remote_rev("1", "r1")]);

        assert!(
            pushes
                .expect("the permitted flag push")
                .iter()
                .all(|c| matches!(c.kind, PimdirChangeKind::SetFlags { .. })),
            "the forbidden content push is withheld",
        );
        let rebased = upserted(&writes, "1").expect("the rebased placement");
        assert_eq!(rebased.status, PimdirStatus::Dirty);
        assert_eq!(rebased.staged_edit(), Some(&PimdirHash::from("h2")));
    }
}
