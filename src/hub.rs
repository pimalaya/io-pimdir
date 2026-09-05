//! # Multi-source hub
//!
//! Shared content with a last-synced base per source, so one logical
//! item can live on several sources (two servers, a server and a phone).
//!
//! A storage wrapping the hub [`project`]s a per-source placement and
//! [`absorb`]s the engine's writes back. A projection carries the shared
//! content against the source's own base, so a change another source
//! folded in reads as dirty here and the ordinary reconcile pushes it.
//!
//! Propagation thus falls out of the per-source merge with no
//! cross-merge: adds, flags and deletes travel the same way, and a
//! cross-source content conflict resolves by [`PimdirHubConflict`].
//!
//! [`project`]: PimdirHub::project
//! [`absorb`]: PimdirHub::absorb

use alloc::{collections::BTreeMap, string::String, vec::Vec};

use crate::{
    change::{PimdirDropReason, PimdirWriteOp},
    collection::PimdirCollectionId,
    object::PimdirHash,
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirOrigin,
        PimdirPlacement, PimdirSortKey, PimdirStatus,
    },
    summary::PimdirSummary,
};

crate::pimdir_id! {
    /// A source of a shared item: one authoritative replica (`left`, `phone`).
    PimdirSourceId, Ord, PartialOrd,
}

/// One source's binding of a shared item: handle, bases and conflict state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirBinding {
    /// The item's handle on this source.
    pub handle: PimdirHandle,
    /// The last state synced with this source; `None` until first reconciled.
    pub base: Option<PimdirBase>,
    /// Whether this source's own sync left the placement in conflict.
    ///
    /// Distinct from [`PimdirHubItem::conflicted`], the cross-source
    /// conflict; a two-source store needs both. Set by a row carrying a
    /// divergence and cleared by one carrying none, so an edit resolves it.
    pub conflicted: bool,
    /// The remote revision observed when the conflict was recorded.
    ///
    /// `None` when not conflicted, or when the remote reports no revision.
    pub conflict_revision: Option<String>,
    /// The remote body at that revision, for the resolver to read locally.
    ///
    /// `None` until the upgrade pass supplies it, and dropped whenever
    /// the revision beside it moves.
    pub conflict_object: Option<PimdirHash>,
    /// The shared body this source last reconciled against.
    ///
    /// The base of the cross-source axis, since [`base`](Self::base) moves
    /// only on a sync and would make a source disagree with a body it
    /// folded in itself. `None` until first folded; the sync base stands in.
    pub shared_object: Option<PimdirHash>,
}

/// How the hub resolves a cross-source content conflict.
///
/// Flags never conflict, they merge element-wise. Only mutable-content
/// backends conflict; immutable ones mint a new link id per body.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PimdirHubConflict {
    /// Flag the item conflicted, keeping the shared body and the diverging one.
    #[default]
    Manual,
    /// Last-writer-wins: adopt the incoming body, overwriting the shared one.
    PreferIncoming,
    /// Keep the already-shared body, dropping the incoming one.
    PreferExisting,
}

/// A logical item shared across sources: content plus a binding per source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirHubItem {
    /// The current flag set, shared by every source.
    pub flags: PimdirFlags,
    /// The current body, shared by every source; `None` until hydrated.
    pub object: Option<PimdirHash>,
    /// The current summary, shared by every source; `None` until fetched.
    pub summary: Option<PimdirSummary>,
    /// The current sort key, shared by every source; empty until derived.
    pub sort_key: PimdirSortKey,
    /// The highest level any source reached.
    ///
    /// The item's own only while it holds a body, see
    /// [`stored_level`](Self::stored_level).
    pub level: PimdirLevel,
    /// Whether a source removed the item, propagating the delete to the rest.
    ///
    /// Never copied to a source lacking it. A later live upsert clears
    /// it, edit and add beating delete across sources.
    pub deleted: bool,
    /// Whether a `Manual` cross-source conflict is unresolved.
    pub conflicted: bool,
    /// The diverging body a `Manual` conflict recorded; `None` otherwise.
    pub conflict_object: Option<PimdirHash>,
    /// Per-source bindings, keyed by source id.
    pub sources: BTreeMap<PimdirSourceId, PimdirBinding>,
}

impl PimdirHubItem {
    /// The level the item can honestly claim: `Full` only with a stored body.
    ///
    /// [`level`](Self::level) is the high-water mark across sources; read
    /// as the fact it would strand an item whose body a content change
    /// dropped, an upgrade skipping whatever reads as `Full`.
    pub fn stored_level(&self) -> PimdirLevel {
        match self.object {
            Some(_) => self.level,
            None => self.level.min(PimdirLevel::Meta),
        }
    }

    /// The shared half of a projection into `collection` under `handle`.
    ///
    /// Status, base and conflict fields are the binding's to settle.
    fn project(
        &self,
        collection: &PimdirCollectionId,
        link: &PimdirLinkId,
        handle: PimdirHandle,
    ) -> PimdirPlacement {
        PimdirPlacement {
            collection: collection.clone(),
            handle,
            link_id: Some(link.clone()),
            object: self.object.clone(),
            level: self.stored_level(),
            summary: self.summary.clone(),
            sort_key: self.sort_key.clone(),
            flags: self.flags.clone(),
            status: PimdirStatus::Clean,
            conflict_revision: None,
            conflict_object: None,
            base: None,
            origin: None,
        }
    }
}

/// The multi-source hub: logical items keyed by link id.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirHub {
    /// The shared items, keyed by their cross-source link id.
    pub items: BTreeMap<PimdirLinkId, PimdirHubItem>,
    /// How a cross-source content conflict is resolved.
    pub conflict: PimdirHubConflict,
}

impl PimdirHub {
    /// Projects the per-source placements a source's `load` should return.
    ///
    /// A bound item carries the shared content against the source's own
    /// base, so an unseen hub change reads dirty. A missing item projects
    /// a `Created` append only once its body is held, so nothing fetches.
    pub fn project(
        &self,
        collection: &PimdirCollectionId,
        source: &PimdirSourceId,
    ) -> Vec<PimdirPlacement> {
        self.project_with(collection, source, |_| None)
    }

    /// [`project`](Self::project) with what the store derives from its
    /// bindings (SYNC §3): the origin a `Created` placement carries, where
    /// `source` already binds the identity in another collection with the
    /// placement's body, so its push is a server-side copy; and the
    /// destination a `Tombstone` carries, where `source` holds a pending
    /// create of the identity elsewhere, so its remove is a relocation.
    ///
    /// The resolver is asked for both statuses and answers by the
    /// placement's; the hub holds one collection and cannot.
    pub fn project_with(
        &self,
        collection: &PimdirCollectionId,
        source: &PimdirSourceId,
        mut origin: impl FnMut(&PimdirPlacement) -> Option<PimdirOrigin>,
    ) -> Vec<PimdirPlacement> {
        let mut out = Vec::new();

        for (link, item) in &self.items {
            // NOTE: an item no source holds is retained by the store (§11)
            // and projected for nobody.
            if item.sources.is_empty() {
                continue;
            }
            let mut placement = match (item.deleted, item.sources.get(source)) {
                (true, Some(binding)) => self.tombstone_placement(collection, link, item, binding),
                (true, None) => continue,
                (false, Some(binding)) => self.bound_placement(collection, link, item, binding),
                (false, None) => match self.created_placement(collection, link, item) {
                    Some(created) => created,
                    None => continue,
                },
            };
            if matches!(
                placement.status,
                PimdirStatus::Created | PimdirStatus::Tombstone
            ) {
                placement.origin = origin(&placement);
            }
            out.push(placement);
        }

        out
    }

    /// The placement for an item this source already holds.
    ///
    /// A recorded conflict outranks the base comparison, else the merge
    /// would re-derive the rejected push every run. A binding with no
    /// base was never reconciled with its remote, so it projects `Created`.
    /// An item holding no body, one another source's content pull
    /// dropped, is no divergence on the content axis: this source owes
    /// nothing until a hydration gives the item a body again.
    fn bound_placement(
        &self,
        collection: &PimdirCollectionId,
        link: &PimdirLinkId,
        item: &PimdirHubItem,
        binding: &PimdirBinding,
    ) -> PimdirPlacement {
        let in_sync = binding.base.as_ref().is_some_and(|b| {
            b.flags == item.flags && (item.object.is_none() || b.object == item.object)
        });
        let status = if binding.conflicted {
            PimdirStatus::Conflict
        } else if binding.base.is_none() {
            PimdirStatus::Created
        } else if in_sync {
            PimdirStatus::Clean
        } else {
            PimdirStatus::Dirty
        };

        let mut placement = item.project(collection, link, binding.handle.clone());
        placement.status = status;
        placement.conflict_revision = binding.conflict_revision.clone();
        placement.conflict_object = binding.conflict_object.clone();
        placement.base = binding.base.clone();
        placement
    }

    /// A `Tombstone` for an item deleted elsewhere but still held here.
    ///
    /// The next sync pushes a `Remove`. The content stays so an edit on
    /// the source's server can still resurrect it, and the binding's
    /// divergence stays since a delete elsewhere settles nothing here.
    fn tombstone_placement(
        &self,
        collection: &PimdirCollectionId,
        link: &PimdirLinkId,
        item: &PimdirHubItem,
        binding: &PimdirBinding,
    ) -> PimdirPlacement {
        let mut placement = item.project(collection, link, binding.handle.clone());
        placement.status = PimdirStatus::Tombstone;
        placement.base = binding.base.clone();
        placement.conflict_revision = binding.conflict_revision.clone();
        placement.conflict_object = binding.conflict_object.clone();
        placement
    }

    /// A `Created` append for an item missing on this source.
    ///
    /// Staged only when the body is held, so it claims `Full` and never
    /// triggers a fetch.
    fn created_placement(
        &self,
        collection: &PimdirCollectionId,
        link: &PimdirLinkId,
        item: &PimdirHubItem,
    ) -> Option<PimdirPlacement> {
        item.object.as_ref()?;

        let mut handle = link.0.clone();
        handle.push_str("\u{1}hub");

        let mut placement = item.project(collection, link, PimdirHandle(handle));
        placement.status = PimdirStatus::Created;
        placement.level = PimdirLevel::Full;
        Some(placement)
    }

    /// Folds a source's sync writes back into the hub.
    ///
    /// An upsert adopts the reconciled content and refreshes the source's
    /// binding, and a drop removes the binding. An item left with no
    /// binding stays, for the store to retain (STORAGE §11). `StoreObject`
    /// and `SetCheckpoint` are the storage's.
    pub fn absorb(&mut self, source: &PimdirSourceId, writes: &[PimdirWriteOp]) {
        for op in writes {
            match op {
                PimdirWriteOp::UpsertPlacement(placement) => self.absorb_upsert(source, placement),
                PimdirWriteOp::DropPlacement { handle, reason, .. } => {
                    self.absorb_drop(source, handle, *reason)
                }
                PimdirWriteOp::StoreObject { .. } | PimdirWriteOp::SetCheckpoint { .. } => {}
            }
        }
    }

    /// Folds one upsert in, refreshing the source's binding.
    ///
    /// An unlinked placement is not hubbed yet. A tombstone keeps its
    /// binding and adopts no content, so an edit elsewhere still
    /// resurrects it, while its known flags ride along (SYNC §9). An
    /// unknown flag set or sort key never erases a known one.
    fn absorb_upsert(&mut self, source: &PimdirSourceId, placement: &PimdirPlacement) {
        let Some(link) = placement.link_id.clone() else {
            return;
        };

        let policy = self.conflict;
        let item = self.items.entry(link).or_insert_with(|| PimdirHubItem {
            flags: placement.flags.clone(),
            object: placement.object.clone(),
            summary: placement.summary.clone(),
            sort_key: placement.sort_key.clone(),
            level: placement.level,
            deleted: false,
            conflicted: false,
            conflict_object: None,
            sources: BTreeMap::new(),
        });

        if placement.status == PimdirStatus::Tombstone {
            let agreed = item
                .sources
                .get(source)
                .and_then(|binding| binding.shared_object.clone());

            item.deleted = true;
            if !placement.flags.is_unknown() {
                item.flags = placement.flags.clone();
            }
            item.sources
                .insert(source.clone(), Self::binding_of(placement, agreed));
            return;
        }

        item.deleted = false;

        if !placement.flags.is_unknown() {
            item.flags = placement.flags.clone();
        }
        if placement.summary.is_some() {
            item.summary = placement.summary.clone();
        }
        if !placement.sort_key.is_unknown() {
            item.sort_key = placement.sort_key.clone();
        }
        // NOTE: a pull dropping the body lowers the level with it (SYNC
        // §5); anything else merges it as a maximum.
        let dropping = placement.object.is_none() && item.object.is_some();
        item.level = match dropping {
            true => placement.level,
            false => item.level.max(placement.level),
        };

        Self::reconcile_content(item, source, placement, policy);

        item.level = item.stored_level();

        let agreed = item.object.clone();

        item.sources
            .insert(source.clone(), Self::binding_of(placement, agreed));
    }

    /// The binding an upsert leaves for its source.
    ///
    /// The conflict is read from the divergence the row carries, not its
    /// status: the two agree everywhere but on a tombstone, which keeps
    /// the divergence its own server opened. A row carrying none clears it.
    fn binding_of(placement: &PimdirPlacement, shared_object: Option<PimdirHash>) -> PimdirBinding {
        let conflicted = placement.conflict_revision.is_some();
        PimdirBinding {
            handle: placement.handle.clone(),
            base: placement.base.clone(),
            conflicted,
            conflict_revision: placement.conflict_revision.clone(),
            conflict_object: conflicted
                .then(|| placement.conflict_object.clone())
                .flatten(),
            shared_object,
        }
    }

    /// Reconciles the shared body against an incoming upsert by policy.
    ///
    /// The source edited when the upsert differs from its sync base, or it
    /// leaves a conflict whatever body it restates; the hub moved when the
    /// shared body differs from what this source last reconciled against.
    fn reconcile_content(
        item: &mut PimdirHubItem,
        source: &PimdirSourceId,
        placement: &PimdirPlacement,
        policy: PimdirHubConflict,
    ) {
        let binding = item.sources.get(source);
        let prev = binding
            .and_then(|b| b.base.as_ref())
            .and_then(|b| b.object.clone());
        let agreed = binding
            .and_then(|b| b.shared_object.clone())
            .or_else(|| prev.clone());
        let shared = item.object.clone();
        let incoming = placement.object.clone();

        let resolving = binding.is_some_and(|binding| binding.conflicted)
            && placement.status != PimdirStatus::Conflict;
        let source_edited = incoming != prev || resolving;
        let hub_moved = shared != agreed;
        let body_changed = incoming != shared;
        let diverged =
            source_edited && hub_moved && body_changed && incoming.is_some() && shared.is_some();

        if diverged {
            match policy {
                PimdirHubConflict::Manual => {
                    item.conflicted = true;
                    item.conflict_object = incoming;
                }
                PimdirHubConflict::PreferIncoming => {
                    item.object = incoming;
                    item.conflicted = false;
                    item.conflict_object = None;
                }
                PimdirHubConflict::PreferExisting => {
                    item.conflicted = false;
                    item.conflict_object = None;
                }
            }
        } else if source_edited && !hub_moved && body_changed {
            item.object = incoming;
            item.conflicted = false;
            item.conflict_object = None;
        }
    }

    /// Unbinds the source; a `Deleted` drop also marks the item deleted.
    fn absorb_drop(
        &mut self,
        source: &PimdirSourceId,
        handle: &PimdirHandle,
        reason: PimdirDropReason,
    ) {
        for item in self.items.values_mut() {
            let bound_here = item
                .sources
                .get(source)
                .is_some_and(|binding| &binding.handle == handle);
            if bound_here {
                let genuine = reason == PimdirDropReason::Deleted;
                item.deleted |= genuine;
                item.sources.remove(source);
            }
        }
    }
}

#[cfg(test)]
mod tests;
