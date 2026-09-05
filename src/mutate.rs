//! # Local mutation
//!
//! I/O-free coroutine applying one edit to a collection offline: it loads
//! the target placement, changes it in memory and writes it back.
//!
//! The base stays untouched so the next [`crate::sync`] derives the
//! pending push. A resolution is the exception: its base becomes the
//! remote state the conflict recorded, revision and body, so the
//! resolving push is conditioned on what the remote holds.
//!
//! A copy or a move carries an identity into another collection and reads
//! that collection first: a collection holds one key once, so a target
//! already holding the identity gives the create a minted key instead.

use core::{error, fmt};

use alloc::{collections::BTreeSet, string::String, vec, vec::Vec};

use log::{debug, trace};

use crate::{
    change::PimdirWriteOp,
    collection::PimdirCollectionId,
    coroutine::*,
    load::PimdirLoadScope,
    object::PimdirObject,
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirOrigin,
        PimdirPlacement, PimdirSortKey, PimdirStatus,
    },
    summary::PimdirSummary,
};

/// A local edit applied offline.
///
/// Each mutation reads one source placement in the coroutine's
/// collection and stages the resulting writes, to be reconciled on the
/// next sync. A copy stages a [`PimdirStatus::Created`] placement in
/// another collection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirMutation {
    /// Replace a placement's flag set.
    ///
    /// The change rides along with whatever else is pending: a create
    /// stays a create, a conflict stays one, a tombstone stays deleted.
    SetFlags {
        /// The placement to update.
        handle: PimdirHandle,
        /// The new flag set.
        flags: PimdirFlags,
    },
    /// Mark a placement deleted, keeping it as a tombstone until synced.
    Remove(PimdirHandle),
    /// Replace a placement's body with locally edited content.
    ///
    /// The new object is stored, the placement repointed at it and marked
    /// dirty, its base keeping the synced body so the next sync derives
    /// the push. Editing a conflict resolves it, a tombstone is revived.
    Edit {
        /// The placement to update.
        handle: PimdirHandle,
        /// The new body's object metadata.
        object: PimdirObject,
        /// The new body bytes.
        body: Vec<u8>,
        /// The refreshed summary, if derived; `None` keeps the stored one.
        summary: Option<PimdirSummary>,
        /// The refreshed sort key, on the same terms as `summary`.
        ///
        /// An edit changing what the key derives from has to say so, or
        /// the item stays where it was in the list.
        sort_key: Option<PimdirSortKey>,
    },
    /// Copy a placement into `target` as a pending create; the source is kept.
    ///
    /// The next sync pushes a server-side copy, no body re-upload. The
    /// copy takes the source's identity where `target` has it free, and a
    /// minted one where a live placement there already holds it.
    Copy {
        /// The source placement to copy.
        handle: PimdirHandle,
        /// The collection to copy it into.
        target: PimdirCollectionId,
        /// The provisional handle the copy is staged under in `target`.
        placeholder: PimdirHandle,
    },
    /// Move a placement into `target`: a create there, a tombstone here.
    ///
    /// Whichever half syncs first delivers, the link id sparing a second
    /// copy as [`PimdirChange`](crate::change::PimdirChange) states: the
    /// tombstone's destination is the pending create the store derives
    /// it from (SYNC §3). A held identity is minted.
    Move {
        /// The source placement to move.
        handle: PimdirHandle,
        /// The collection to move it into.
        target: PimdirCollectionId,
        /// The provisional handle the move is staged under in `target`.
        placeholder: PimdirHandle,
    },
    /// Create a locally-authored item with no remote origin (compose, import).
    ///
    /// Stages a pending create the next sync pushes as an append,
    /// uploading the body. Reads no existing source.
    Add {
        /// The provisional handle, rekeyed once the push reports the server's.
        handle: PimdirHandle,
        /// The item's cross-source link id (a `Message-ID` header).
        link_id: PimdirLinkId,
        /// The initial flag set.
        flags: PimdirFlags,
        /// The new body's object metadata.
        object: PimdirObject,
        /// The new body bytes.
        body: Vec<u8>,
        /// The summary and addresses, when the consumer derived them.
        summary: Option<PimdirSummary>,
        /// The sort key, when the consumer's kind defines one.
        sort_key: PimdirSortKey,
    },
}

impl PimdirMutation {
    /// The source handle the mutation reads, `None` for [`Add`](Self::Add).
    fn handle(&self) -> Option<&PimdirHandle> {
        match self {
            Self::SetFlags { handle, .. } => Some(handle),
            Self::Remove(handle) => Some(handle),
            Self::Edit { handle, .. } => Some(handle),
            Self::Copy { handle, .. } => Some(handle),
            Self::Move { handle, .. } => Some(handle),
            Self::Add { .. } => None,
        }
    }

    /// The target and placeholder of a staged create, else `None`.
    fn create_target(&self) -> Option<(&PimdirCollectionId, &PimdirHandle)> {
        match self {
            Self::Copy {
                target,
                placeholder,
                ..
            }
            | Self::Move {
                target,
                placeholder,
                ..
            } => Some((target, placeholder)),
            _ => None,
        }
    }

    /// What the mutation reads: its placement, or the rows an `Add` may hit.
    ///
    /// Named variant by variant: a mutation added later has to say what
    /// it reads, where a catch-all would hand it the whole collection.
    fn scope(&self) -> PimdirLoadScope {
        match self {
            Self::Add { link_id, .. } => PimdirLoadScope::Links(vec![link_id.clone()]),
            Self::SetFlags { handle, .. }
            | Self::Remove(handle)
            | Self::Edit { handle, .. }
            | Self::Copy { handle, .. }
            | Self::Move { handle, .. } => PimdirLoadScope::Handles(vec![handle.clone()]),
        }
    }
}

/// Failure causes during a MUTATE flow.
#[derive(Clone, Debug)]
pub enum PimdirMutateError {
    /// The targeted handle has no placement in the collection.
    UnknownHandle(String),
    /// The targeted handle is a probe: a `Meta` upgrade names it first.
    Probed(String),
    /// An `Add` names a link id a live placement already holds.
    LinkExists(String),
    /// The caller broke the coroutine contract.
    Arg(PimdirArgError),
}

impl fmt::Display for PimdirMutateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownHandle(handle) => {
                write!(f, "Pimdir MUTATE failed: unknown handle {handle}")
            }
            Self::Probed(handle) => {
                write!(f, "Pimdir MUTATE failed: handle {handle} is a probe")
            }
            Self::LinkExists(link_id) => {
                write!(
                    f,
                    "Pimdir MUTATE failed: link id already present: {link_id}"
                )
            }
            Self::Arg(err) => write!(f, "{err}"),
        }
    }
}

impl error::Error for PimdirMutateError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Arg(err) => Some(err),
            _ => None,
        }
    }
}

impl From<PimdirArgError> for PimdirMutateError {
    fn from(err: PimdirArgError) -> Self {
        Self::Arg(err)
    }
}

/// I/O-free MUTATE coroutine.
pub struct PimdirMutate {
    collection: PimdirCollectionId,
    mutation: PimdirMutation,
    /// The source of a staged create, held while the target's keys are read.
    source: Option<PimdirPlacement>,
    state: State,
}

impl PimdirMutate {
    /// Creates a coroutine that applies `mutation` to `collection`.
    pub fn new(collection: impl Into<PimdirCollectionId>, mutation: PimdirMutation) -> Self {
        let collection = collection.into();
        debug!("mutate collection {}", collection.as_str());

        Self {
            collection,
            mutation,
            source: None,
            state: State::Start,
        }
    }

    /// Stages the writes for the loaded `source` placement.
    ///
    /// `key` is the identity a staged create takes in its target. Flag
    /// sets and removes rewrite the source in place; a copy leaves it
    /// untouched and stages a pending create in the target.
    fn writes(&self, mut source: PimdirPlacement, key: Option<PimdirLinkId>) -> Vec<PimdirWriteOp> {
        match &self.mutation {
            PimdirMutation::SetFlags { flags, .. } => {
                source.flags = flags.clone();
                if source.status == PimdirStatus::Clean {
                    source.status = PimdirStatus::Dirty;
                }
                vec![PimdirWriteOp::UpsertPlacement(source)]
            }
            PimdirMutation::Remove(_) => {
                source.status = PimdirStatus::Tombstone;
                vec![PimdirWriteOp::UpsertPlacement(source)]
            }
            PimdirMutation::Edit {
                object,
                body,
                summary,
                sort_key,
                ..
            } => {
                source.object = Some(object.hash.clone());
                source.level = PimdirLevel::Full;
                if summary.is_some() {
                    source.summary = summary.clone();
                }
                if let Some(sort_key) = sort_key {
                    source.sort_key = sort_key.clone();
                }

                // NOTE: a tombstone can carry a divergence too, so the
                // row's divergence, not its status, says whether this
                // edit resolves one.
                let resolving = source.conflict_revision.is_some();
                if resolving {
                    let revision = source.conflict_revision.take();
                    let settled = source.conflict_object.take();
                    let base = source.base.get_or_insert_with(|| PimdirBase {
                        flags: source.flags.clone(),
                        revision: None,
                        object: None,
                    });
                    base.revision = revision;
                    base.object = settled;
                }

                let staged = source
                    .base
                    .as_ref()
                    .is_none_or(|base| base.object.as_ref() != Some(&object.hash));
                if staged || resolving {
                    // NOTE: a revived tombstone goes nowhere any more, and
                    // a create copied from its origin would deliver the
                    // body this edit replaced (SYNC §7).
                    source.origin = None;
                    if source.status != PimdirStatus::Created {
                        source.status = PimdirStatus::Dirty;
                    }
                }

                vec![
                    PimdirWriteOp::StoreObject {
                        object: object.clone(),
                        body: Some(body.clone()),
                    },
                    PimdirWriteOp::UpsertPlacement(source),
                ]
            }
            PimdirMutation::Copy {
                target,
                placeholder,
                ..
            } => {
                let create = Self::staged_copy(&source, target, placeholder, key);
                vec![PimdirWriteOp::UpsertPlacement(create)]
            }
            PimdirMutation::Move {
                target,
                placeholder,
                ..
            } => {
                let create = source
                    .link_id
                    .is_some()
                    .then(|| Self::staged_copy(&source, target, placeholder, key));

                source.status = PimdirStatus::Tombstone;
                source.origin = Some(PimdirOrigin {
                    collection: target.clone(),
                    handle: source.handle.clone(),
                });

                create
                    .map(PimdirWriteOp::UpsertPlacement)
                    .into_iter()
                    .chain([PimdirWriteOp::UpsertPlacement(source)])
                    .collect()
            }
            PimdirMutation::Add { .. } => self.create_writes(),
        }
    }

    /// The `Created` placement a copy or a move stages in its target.
    ///
    /// It carries the source as its [`PimdirOrigin`], so the push is a
    /// server-side copy, and `key` as its identity there, the source's
    /// own or a minted one.
    fn staged_copy(
        source: &PimdirPlacement,
        target: &PimdirCollectionId,
        placeholder: &PimdirHandle,
        key: Option<PimdirLinkId>,
    ) -> PimdirPlacement {
        let origin = Some(PimdirOrigin {
            collection: source.collection.clone(),
            handle: source.handle.clone(),
        });

        PimdirPlacement {
            collection: target.clone(),
            handle: placeholder.clone(),
            link_id: key,
            object: source.object.clone(),
            level: source.level,
            summary: source.summary.clone(),
            sort_key: source.sort_key.clone(),
            flags: source.flags.clone(),
            status: PimdirStatus::Created,
            conflict_revision: None,
            conflict_object: None,
            base: None,
            origin,
        }
    }

    /// Stages the writes for an [`Add`](PimdirMutation::Add).
    ///
    /// A `Created` placement with no base and no origin, so the next sync
    /// appends it rather than server-copying, plus its object.
    fn create_writes(&self) -> Vec<PimdirWriteOp> {
        let PimdirMutation::Add {
            handle,
            link_id,
            flags,
            object,
            body,
            summary,
            sort_key,
        } = &self.mutation
        else {
            return Vec::new();
        };

        let create = PimdirPlacement {
            collection: self.collection.clone(),
            handle: handle.clone(),
            link_id: Some(link_id.clone()),
            object: Some(object.hash.clone()),
            level: PimdirLevel::Full,
            summary: summary.clone(),
            sort_key: sort_key.clone(),
            flags: flags.clone(),
            status: PimdirStatus::Created,
            conflict_revision: None,
            conflict_object: None,
            base: None,
            origin: None,
        };
        vec![
            PimdirWriteOp::StoreObject {
                object: object.clone(),
                body: Some(body.clone()),
            },
            PimdirWriteOp::UpsertPlacement(create),
        ]
    }
}

impl PimdirCoroutine for PimdirMutate {
    type Yield = PimdirYield;
    type Return = Result<(), PimdirMutateError>;

    fn resume(
        &mut self,
        arg: Option<PimdirArg>,
    ) -> PimdirCoroutineState<Self::Yield, Self::Return> {
        match (&mut self.state, arg) {
            (State::Start, None) => {
                debug!("load target item from storage");
                self.state = State::Loading;
                PimdirCoroutineState::Yielded(PimdirYield::WantsLoad {
                    collection: self.collection.clone(),
                    scope: self.mutation.scope(),
                })
            }
            (State::Loading, Some(PimdirArg::Load(loaded))) => {
                let ops = if let PimdirMutation::Add { link_id, .. } = &self.mutation {
                    let collides = loaded.placements.iter().any(|p| {
                        p.status != PimdirStatus::Tombstone && p.link_id.as_ref() == Some(link_id)
                    });
                    if collides {
                        let err = PimdirMutateError::LinkExists(link_id.0.clone());
                        return PimdirCoroutineState::Complete(Err(err));
                    }
                    self.create_writes()
                } else {
                    let handle = self
                        .mutation
                        .handle()
                        .expect("non-Add mutation has a handle");
                    let Some(placement) =
                        loaded.placements.into_iter().find(|p| &p.handle == handle)
                    else {
                        let err = PimdirMutateError::UnknownHandle(handle.as_str().into());
                        return PimdirCoroutineState::Complete(Err(err));
                    };
                    if placement.link_id.is_none() {
                        let err = PimdirMutateError::Probed(handle.as_str().into());
                        return PimdirCoroutineState::Complete(Err(err));
                    }

                    if let Some((target, placeholder)) = self.mutation.create_target()
                        && let Some(hint) = placement.link_id.clone()
                    {
                        let collection = target.clone();
                        let scope =
                            PimdirLoadScope::Links(vec![hint.clone(), hint.minted(placeholder)]);

                        debug!("read what {} holds of that identity", collection.as_str());
                        self.source = Some(placement);
                        self.state = State::LoadingTarget;
                        return PimdirCoroutineState::Yielded(PimdirYield::WantsLoad {
                            collection,
                            scope,
                        });
                    }

                    self.writes(placement, None)
                };

                debug!("stage local change, {} write(s)", ops.len());
                trace!("writes: {ops:?}");

                self.state = State::Writing;
                PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops))
            }
            (State::LoadingTarget, Some(PimdirArg::Load(loaded))) => {
                let source = self.source.take().expect("the source of a staged create");
                let (_, placeholder) = self
                    .mutation
                    .create_target()
                    .expect("a mutation staging a create");
                let hint = source.link_id.clone().expect("a linked source");
                let held: BTreeSet<PimdirLinkId> = loaded
                    .placements
                    .into_iter()
                    .filter(|p| p.status != PimdirStatus::Tombstone)
                    .filter_map(|p| p.link_id)
                    .collect();

                let key = hint.claim(placeholder, |key| held.contains(key));
                let ops = self.writes(source, Some(key));

                debug!("stage local change, {} write(s)", ops.len());
                trace!("writes: {ops:?}");

                self.state = State::Writing;
                PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops))
            }
            (State::Writing, Some(PimdirArg::Write)) => {
                debug!("local change written");
                self.state = State::Done;
                PimdirCoroutineState::Complete(Ok(()))
            }
            (State::Done, _) | (_, Some(_)) => {
                PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg.into()))
            }
            (_, None) => PimdirCoroutineState::Complete(Err(PimdirArgError::MissingArg.into())),
        }
    }
}

/// What the coroutine is doing while it waits for the caller.
enum State {
    Start,
    Loading,
    LoadingTarget,
    Writing,
    Done,
}

#[cfg(test)]
mod tests;
