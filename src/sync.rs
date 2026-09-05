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
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirPlacement, PimdirSortKey,
        PimdirStatus,
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
    /// Keep both: pull the remote and stage the local body as a new member.
    KeepBoth,
}

/// What becomes of a local delete the source will not take (SYNC §5).
///
/// A forbidden flag or content change stays dirty and re-derives, but a
/// forbidden delete is either undone or held, and holding it hides a
/// member the source still holds. Which is right depends on whether the
/// source is bound beside others, which a consumer knows and the engine
/// does not.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PimdirDeletePolicy {
    /// Let the consumer decide from the binding count (the default).
    ///
    /// The engine reads it as [`Revert`](Self::Revert); a consumer that
    /// knows how many sources bind the collection resolves it to
    /// [`Keep`](Self::Keep) when there is more than one, a revert reading
    /// as a resurrection there.
    #[default]
    Auto,
    /// Undo it: the member comes back with what it had cached.
    ///
    /// The right reading for a source the replica does not own: an
    /// incremental enumeration never lists an untouched member again, so
    /// a held tombstone hides that member for good.
    Revert,
    /// Hold it: the tombstone stays pending until a later run may push it.
    ///
    /// The right reading when the refusal may lift, and the one a source
    /// bound beside others wants: reverting says the source still holds
    /// the member, which the [hub](crate::hub) reads as alive and mirrors
    /// back.
    Keep,
}

/// Tuning for one sync run: the push direction and the enumerate depth.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PimdirSyncOptions {
    /// The master push switch: when false the source is read-only.
    pub push: bool,
    /// Per-kind refinement of `push`, consulted only when `push` is true.
    pub rights: PimdirPushRights,
    /// What becomes of a local delete this source will not take.
    pub delete: PimdirDeletePolicy,
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
            delete: PimdirDeletePolicy::default(),
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
    local: BTreeMap<PimdirHandle, PimdirPlacement>,
    checkpoint: Option<PimdirCheckpoint>,
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
            local: BTreeMap::new(),
            checkpoint: None,
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
                    return self.resurrect(local);
                }

                self.drop(&handle, PimdirDropReason::Deleted);
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
                if local.status != PimdirStatus::Created {
                    // NOTE: a base-less body is a create-collision whose
                    // remote side went, so the local body survives as a
                    // create; a base-less row holding none is a probe, and
                    // a probe the enumeration no longer lists is gone.
                    if local.object.is_some() {
                        return self.resurrect(local);
                    }
                    self.drop(&handle, PimdirDropReason::Deleted);
                    self.report.pulled += 1;
                    self.emit(PimdirSyncEvent::Vanished(handle.clone()));
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

    /// Stages a body whose remote side went as a fresh create (SYNC §5).
    ///
    /// New content beats a delete: the placement is rewritten `Created`
    /// with no base, no origin and no divergence, and appended when the
    /// source takes adds.
    fn resurrect(&mut self, local: PimdirPlacement) -> Option<PimdirChangeKind> {
        let handle = local.handle.clone();
        let mut resurrected = local;
        resurrected.status = PimdirStatus::Created;
        resurrected.conflict_revision = None;
        resurrected.conflict_object = None;
        resurrected.base = None;
        resurrected.origin = None;
        self.upsert(resurrected.clone());

        if !(self.opts.push && self.opts.rights.add) {
            return None;
        }

        let add = PimdirChangeKind::Add {
            handle: handle.clone(),
            link_id: resurrected.link_id.clone(),
            flags: resurrected.flags.clone(),
            origin: None,
            object: resurrected.object.clone(),
        };
        self.pending.insert(handle, Pending::Create(resurrected));
        Some(add)
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
            if item.revision.is_some() && item.revision != local.conflict_revision {
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
            PimdirConflictPolicy::KeepBoth => {
                self.stage_conflict_dup(local, item);
                ContentOutcome::Rewritten(self.pull_content(local, item))
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

    /// Stages the local body as a fresh `Created` member for `KeepBoth`.
    ///
    /// The duplicate is a second copy of the identity under a key minted
    /// from its provisional handle, which names the placement, body and
    /// remote revision it forked, so a replay stages the same row.
    fn stage_conflict_dup(&mut self, local: &PimdirPlacement, item: &PimdirRemoteItem) {
        let object = local.object.clone().expect("a staged edited body");

        let mut handle = local.handle.0.clone();
        handle.push('\u{1}');
        handle.push_str(object.as_str());
        handle.push('\u{1}');
        handle.push_str(item.revision.as_deref().unwrap_or_default());
        let handle = PimdirHandle(handle);

        // NOTE: a second copy of one identity, minted like any other
        // (STORAGE §9), so lookup_objects never pairs it with the original.
        let link = local.link_id.as_ref().map(|hint| hint.minted(&handle));

        let dup = PimdirPlacement {
            collection: self.collection.clone(),
            handle,
            link_id: link,
            object: Some(object),
            level: PimdirLevel::Full,
            summary: local.summary.clone(),
            sort_key: local.sort_key.clone(),
            flags: local.flags.clone(),
            status: PimdirStatus::Created,
            conflict_revision: None,
            conflict_object: None,
            base: None,
            origin: None,
        };
        self.upsert(dup);
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

    /// Settles a local delete the source refused, per [`PimdirDeletePolicy`].
    fn refuse_delete(&mut self, local: PimdirPlacement) -> Option<PimdirChangeKind> {
        match self.opts.delete {
            PimdirDeletePolicy::Auto | PimdirDeletePolicy::Revert => {
                debug!("reverting a delete {} will not take", local.handle.as_str());
                let mut reverted = local;
                // NOTE: only the delete is undone, a divergence or a staged
                // edit is still owed.
                reverted.status = match (&reverted.conflict_revision, reverted.staged_edit()) {
                    (Some(_), _) => PimdirStatus::Conflict,
                    (None, Some(_)) => PimdirStatus::Dirty,
                    (None, None) => PimdirStatus::Clean,
                };
                // NOTE: left behind, the destination would turn the next
                // plain delete of the member into a move.
                reverted.origin = None;
                self.upsert(reverted);
            }
            PimdirDeletePolicy::Keep => {
                trace!("holding a delete {} will not take", local.handle.as_str());
            }
        }

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
mod tests;
