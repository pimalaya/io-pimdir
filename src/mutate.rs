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
    /// copy as [`PimdirChange`](crate::change::PimdirChange) states. A
    /// held identity is minted.
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
                if source.status != PimdirStatus::Created && (staged || resolving) {
                    // NOTE: the destination goes with the delete it
                    // belonged to, else the revived row's next plain
                    // delete reads as a relocation.
                    if source.status == PimdirStatus::Tombstone {
                        source.origin = None;
                    }
                    source.status = PimdirStatus::Dirty;
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
                self.state = State::PendingLoad;
                PimdirCoroutineState::Yielded(PimdirYield::WantsLoad {
                    collection: self.collection.clone(),
                    scope: self.mutation.scope(),
                })
            }
            (State::PendingLoad, Some(PimdirArg::Load(loaded))) => {
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
                        self.state = State::PendingTargetLoad;
                        return PimdirCoroutineState::Yielded(PimdirYield::WantsLoad {
                            collection,
                            scope,
                        });
                    }

                    self.writes(placement, None)
                };

                debug!("stage local change, {} write(s)", ops.len());
                trace!("writes: {ops:?}");

                self.state = State::PendingWrite;
                PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops))
            }
            (State::PendingTargetLoad, Some(PimdirArg::Load(loaded))) => {
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

                self.state = State::PendingWrite;
                PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops))
            }
            (State::PendingWrite, Some(PimdirArg::Write)) => {
                debug!("local change written");
                self.state = State::Done;
                PimdirCoroutineState::Complete(Ok(()))
            }
            (_, Some(_)) => {
                PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg.into()))
            }
            (_, None) => PimdirCoroutineState::Complete(Err(PimdirArgError::MissingArg.into())),
        }
    }
}

enum State {
    Start,
    PendingLoad,
    PendingTargetLoad,
    PendingWrite,
    Done,
}

#[cfg(test)]
mod tests {
    use alloc::{string::ToString, vec};

    use crate::{
        load::PimdirLoaded,
        mutate::*,
        object::PimdirHash,
        placement::{PimdirBase, PimdirLevel, PimdirStatus},
    };

    fn loaded(handle: &str) -> PimdirLoaded {
        crate::testlog::init();
        PimdirLoaded {
            placements: vec![PimdirPlacement {
                sort_key: Default::default(),
                collection: "inbox".into(),
                handle: PimdirHandle::from(handle),
                link_id: Some(PimdirLinkId::from(handle)),
                object: None,
                level: PimdirLevel::Meta,
                summary: None,
                flags: PimdirFlags::default(),
                conflict_revision: None,
                conflict_object: None,
                status: PimdirStatus::Clean,
                base: Some(PimdirBase {
                    flags: PimdirFlags::default(),
                    revision: None,
                    object: None,
                }),
                origin: None,
            }],
            checkpoint: None,
        }
    }

    #[test]
    fn set_flags_marks_dirty() {
        let mutation = PimdirMutation::SetFlags {
            handle: PimdirHandle::from("1"),
            flags: PimdirFlags::from_iter(["seen"]),
        };
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);

        let ops = match mutate.resume(Some(PimdirArg::Load(loaded("1")))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };
        let PimdirWriteOp::UpsertPlacement(p) = &ops[0] else {
            panic!("expected UpsertPlacement, got {:?}", ops[0]);
        };
        assert_eq!(p.status, PimdirStatus::Dirty);
        assert!(p.flags.contains("seen"));
        assert!(p.base.is_some(), "base must be preserved for sync");
    }

    /// The flag rides along, so the sync never reads the row as plain dirty.
    #[test]
    fn set_flags_on_a_conflicted_placement_keeps_the_conflict() {
        let mutation = PimdirMutation::SetFlags {
            handle: PimdirHandle::from("1"),
            flags: PimdirFlags::from_iter(["seen"]),
        };
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);

        let mut loaded = loaded("1");
        loaded.placements[0].status = PimdirStatus::Conflict;
        loaded.placements[0].conflict_revision = Some("r2".into());

        let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };
        let PimdirWriteOp::UpsertPlacement(p) = &ops[0] else {
            panic!("expected UpsertPlacement, got {:?}", ops[0]);
        };
        assert_eq!(p.status, PimdirStatus::Conflict);
        assert_eq!(p.conflict_revision.as_deref(), Some("r2"));
        assert!(p.flags.contains("seen"));
    }

    /// A pending create keeps its status, else the sync never pushes the add.
    #[test]
    fn set_flags_on_a_created_placement_stays_created() {
        let mutation = PimdirMutation::SetFlags {
            handle: PimdirHandle::from("1"),
            flags: PimdirFlags::from_iter(["seen"]),
        };
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);

        let mut loaded = loaded("1");
        loaded.placements[0].status = PimdirStatus::Created;
        loaded.placements[0].base = None;

        let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };
        let PimdirWriteOp::UpsertPlacement(p) = &ops[0] else {
            panic!("expected UpsertPlacement, got {:?}", ops[0]);
        };
        assert_eq!(p.status, PimdirStatus::Created);
        assert!(p.flags.contains("seen"));
    }

    #[test]
    fn remove_marks_tombstone() {
        let mutation = PimdirMutation::Remove(PimdirHandle::from("1"));
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);

        let ops = match mutate.resume(Some(PimdirArg::Load(loaded("1")))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };
        let PimdirWriteOp::UpsertPlacement(p) = &ops[0] else {
            panic!("expected UpsertPlacement, got {:?}", ops[0]);
        };
        assert_eq!(p.status, PimdirStatus::Tombstone);
    }

    #[test]
    fn unknown_handle_errors() {
        let mutation = PimdirMutation::Remove(PimdirHandle::from("nope"));
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);

        match mutate.resume(Some(PimdirArg::Load(loaded("1")))) {
            PimdirCoroutineState::Complete(Err(PimdirMutateError::UnknownHandle(h))) => {
                assert_eq!(h, "nope");
            }
            state => panic!("expected UnknownHandle, got {state:?}"),
        }
    }

    /// A probe has no identity to stage anything under.
    #[test]
    fn a_probe_refuses_a_mutation() {
        let mutation = PimdirMutation::Remove(PimdirHandle::from("1"));
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);

        let mut probed = loaded("1");
        probed.placements[0].link_id = None;
        probed.placements[0].level = PimdirLevel::Probed;
        probed.placements[0].base = None;

        match mutate.resume(Some(PimdirArg::Load(probed))) {
            PimdirCoroutineState::Complete(Err(PimdirMutateError::Probed(h))) => {
                assert_eq!(h, "1");
            }
            state => panic!("expected Probed, got {state:?}"),
        }
    }

    /// No base and no origin: the shape the sync pushes as an append.
    #[test]
    fn add_stages_an_append_create() {
        use crate::object::{PimdirHash, PimdirObject};

        let mutation = PimdirMutation::Add {
            sort_key: Default::default(),
            handle: PimdirHandle::from("draft-1"),
            link_id: PimdirLinkId("mid:new".into()),
            flags: PimdirFlags::from_iter(["\\Draft"]),
            object: PimdirObject {
                hash: PimdirHash::from("deadbeef"),
                size: 5,
            },
            body: b"hello".to_vec(),
            summary: Some(crate::summary::stub("{\"v\":1}")),
        };
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);

        let ops = match mutate.resume(Some(PimdirArg::Load(loaded("other")))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };

        let PimdirWriteOp::StoreObject { body, object } = &ops[0] else {
            panic!("expected StoreObject, got {:?}", ops[0]);
        };
        assert_eq!(body.as_deref(), Some(&b"hello"[..]));
        assert_eq!(object.hash, PimdirHash::from("deadbeef"));

        let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
            panic!("expected UpsertPlacement, got {:?}", ops[1]);
        };
        assert_eq!(p.status, PimdirStatus::Created);
        assert!(p.base.is_none(), "no prior sync");
        assert!(p.origin.is_none(), "an append, not a server copy");
        assert_eq!(p.link_id, Some(PimdirLinkId("mid:new".into())));
        assert_eq!(p.level, PimdirLevel::Full);
        assert!(p.flags.contains("\\Draft"));
    }

    #[test]
    fn add_rejects_a_live_link_id_collision() {
        use crate::object::{PimdirHash, PimdirObject};

        let mutation = PimdirMutation::Add {
            sort_key: Default::default(),
            handle: PimdirHandle::from("draft-1"),
            link_id: PimdirLinkId("mid:dup".into()),
            flags: PimdirFlags::default(),
            object: PimdirObject {
                hash: PimdirHash::from("deadbeef"),
                size: 1,
            },
            body: b"x".to_vec(),
            summary: None,
        };
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);

        let mut loaded = loaded("existing");
        loaded.placements[0].link_id = Some(PimdirLinkId("mid:dup".into()));

        match mutate.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Complete(Err(PimdirMutateError::LinkExists(l))) => {
                assert_eq!(l, "mid:dup");
            }
            state => panic!("expected LinkExists, got {state:?}"),
        }
    }

    /// The delete is in flight and the new item supersedes it.
    #[test]
    fn add_over_a_tombstone_link_id_is_allowed() {
        use crate::object::{PimdirHash, PimdirObject};

        let mutation = PimdirMutation::Add {
            sort_key: Default::default(),
            handle: PimdirHandle::from("draft-1"),
            link_id: PimdirLinkId("mid:gone".into()),
            flags: PimdirFlags::default(),
            object: PimdirObject {
                hash: PimdirHash::from("deadbeef"),
                size: 1,
            },
            body: b"x".to_vec(),
            summary: None,
        };
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);

        let mut loaded = loaded("existing");
        loaded.placements[0].link_id = Some(PimdirLinkId("mid:gone".into()));
        loaded.placements[0].status = PimdirStatus::Tombstone;

        match mutate.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(_)) => {}
            state => panic!("expected WantsWrite, got {state:?}"),
        }
    }

    #[test]
    fn write_completes() {
        let mutation = PimdirMutation::Remove(PimdirHandle::from("1"));
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);
        let _ = mutate.resume(Some(PimdirArg::Load(loaded("1"))));

        match mutate.resume(Some(PimdirArg::Write)) {
            PimdirCoroutineState::Complete(Ok(())) => {}
            state => panic!("expected Complete(Ok), got {state:?}"),
        }
    }

    #[test]
    fn missing_arg_errors() {
        let mutation = PimdirMutation::Remove(PimdirHandle::from("1"));
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);

        match mutate.resume(None) {
            PimdirCoroutineState::Complete(Err(PimdirMutateError::Arg(
                PimdirArgError::MissingArg,
            ))) => {}
            state => panic!("expected MissingArg, got {state:?}"),
        }
    }

    /// A caller resuming a finished coroutine is told, not handed a success.
    #[test]
    fn a_completed_mutate_does_not_resume() {
        let mutation = PimdirMutation::Remove(PimdirHandle::from("1"));
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);
        let _ = mutate.resume(Some(PimdirArg::Load(loaded("1"))));
        let _ = mutate.resume(Some(PimdirArg::Write));

        match mutate.resume(Some(PimdirArg::Write)) {
            PimdirCoroutineState::Complete(Err(PimdirMutateError::Arg(
                PimdirArgError::UnexpectedArg,
            ))) => {}
            state => panic!("expected UnexpectedArg, got {state:?}"),
        }
    }

    #[test]
    fn unexpected_arg_errors() {
        let mutation = PimdirMutation::Remove(PimdirHandle::from("1"));
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);

        match mutate.resume(Some(PimdirArg::Write)) {
            PimdirCoroutineState::Complete(Err(PimdirMutateError::Arg(
                PimdirArgError::UnexpectedArg,
            ))) => {}
            state => panic!("expected UnexpectedArg, got {state:?}"),
        }
    }

    /// The base keeps the synced state, so the next sync derives the push.
    #[test]
    fn edit_stages_a_dirty_body() {
        use crate::object::{PimdirHash, PimdirObject};

        let mutation = PimdirMutation::Edit {
            sort_key: Default::default(),
            handle: PimdirHandle::from("1"),
            object: PimdirObject {
                hash: PimdirHash::from("h2"),
                size: 4,
            },
            body: b"body".to_vec(),
            summary: None,
        };
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);

        let ops = match mutate.resume(Some(PimdirArg::Load(loaded("1")))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };

        assert!(
            matches!(&ops[0], PimdirWriteOp::StoreObject { object, .. } if object.hash == PimdirHash::from("h2"))
        );
        let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
            panic!("expected UpsertPlacement, got {:?}", ops[1]);
        };
        assert_eq!(p.status, PimdirStatus::Dirty);
        assert_eq!(p.object, Some(PimdirHash::from("h2")));
        assert_eq!(p.level, PimdirLevel::Full);
        assert!(p.base.is_some(), "base must be preserved for sync");
    }

    /// An edit says when the key moves; one that says nothing leaves it.
    #[test]
    fn an_edit_restates_the_sort_key_or_keeps_it() {
        use crate::object::{PimdirHash, PimdirObject};

        let edit = |sort_key: Option<PimdirSortKey>| {
            let mutation = PimdirMutation::Edit {
                sort_key,
                handle: PimdirHandle::from("1"),
                object: PimdirObject {
                    hash: PimdirHash::from("h2"),
                    size: 4,
                },
                body: b"body".to_vec(),
                summary: None,
            };
            let mut mutate = PimdirMutate::new("inbox", mutation);
            let _ = mutate.resume(None);

            let mut loaded = loaded("1");
            loaded.placements[0].sort_key = PimdirSortKey::from("2026-01-01");
            let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
                PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
                state => panic!("expected WantsWrite, got {state:?}"),
            };
            let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
                panic!("expected UpsertPlacement, got {:?}", ops[1]);
            };
            p.sort_key.clone()
        };

        assert_eq!(edit(None), PimdirSortKey::from("2026-01-01"));
        assert_eq!(
            edit(Some(PimdirSortKey::from("2026-02-02"))),
            PimdirSortKey::from("2026-02-02"),
        );
    }

    /// No push to pend, so no dirty status `staged_edit` would contradict.
    #[test]
    fn an_edit_restating_the_synced_body_stages_nothing() {
        use crate::object::{PimdirHash, PimdirObject};

        let mutation = PimdirMutation::Edit {
            sort_key: Default::default(),
            handle: PimdirHandle::from("1"),
            object: PimdirObject {
                hash: PimdirHash::from("h1"),
                size: 4,
            },
            body: b"body".to_vec(),
            summary: None,
        };
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);

        let mut loaded = loaded("1");
        loaded.placements[0].object = Some(PimdirHash::from("h1"));
        loaded.placements[0].level = PimdirLevel::Full;
        loaded.placements[0].base.as_mut().expect("a base").object = Some(PimdirHash::from("h1"));

        let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };
        let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
            panic!("expected UpsertPlacement, got {:?}", ops[1]);
        };
        assert_eq!(p.status, PimdirStatus::Clean);
        assert_eq!(p.staged_edit(), None, "the status agrees with the reading");
    }

    /// Keeping the ancestor is a decision the remote has to hear.
    #[test]
    fn resolving_a_conflict_with_the_base_body_still_pushes() {
        use crate::object::{PimdirHash, PimdirObject};

        let mutation = PimdirMutation::Edit {
            sort_key: Default::default(),
            handle: PimdirHandle::from("1"),
            object: PimdirObject {
                hash: PimdirHash::from("h1"),
                size: 4,
            },
            body: b"body".to_vec(),
            summary: None,
        };
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);

        let mut loaded = loaded("1");
        loaded.placements[0].status = PimdirStatus::Conflict;
        loaded.placements[0].object = Some(PimdirHash::from("h2"));
        loaded.placements[0].level = PimdirLevel::Full;
        loaded.placements[0].conflict_revision = Some("r2".into());
        loaded.placements[0].conflict_object = Some(PimdirHash::from("h-remote"));
        loaded.placements[0].base.as_mut().expect("a base").object = Some(PimdirHash::from("h1"));

        let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };
        let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
            panic!("expected UpsertPlacement, got {:?}", ops[1]);
        };
        assert_eq!(p.status, PimdirStatus::Dirty);
        assert_eq!(p.conflict_revision, None);
        assert_eq!(p.conflict_object, None);
    }

    /// The base takes revision and body together.
    ///
    /// The revision alone would claim one the base object was never the
    /// content of, and the next sync reads a resolution keeping the
    /// ancestor as nothing to push.
    #[test]
    fn a_resolution_adopts_the_whole_remote_state_into_the_base() {
        use crate::object::{PimdirHash, PimdirObject};

        let mutation = PimdirMutation::Edit {
            sort_key: Default::default(),
            handle: PimdirHandle::from("1"),
            object: PimdirObject {
                hash: PimdirHash::from("h-base"),
                size: 4,
            },
            body: b"base".to_vec(),
            summary: None,
        };
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);

        let mut loaded = loaded("1");
        loaded.placements[0].status = PimdirStatus::Conflict;
        loaded.placements[0].object = Some(PimdirHash::from("h-local"));
        loaded.placements[0].level = PimdirLevel::Full;
        loaded.placements[0].conflict_revision = Some("r2".into());
        loaded.placements[0].conflict_object = Some(PimdirHash::from("h-remote"));
        let base = loaded.placements[0].base.as_mut().expect("a base");
        base.revision = Some("r1".into());
        base.object = Some(PimdirHash::from("h-base"));

        let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };
        let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
            panic!("expected UpsertPlacement, got {:?}", ops[1]);
        };
        let base = p.base.as_ref().expect("a base");
        assert_eq!(base.revision.as_deref(), Some("r2"));
        assert_eq!(
            base.object,
            Some(PimdirHash::from("h-remote")),
            "the base object is the body the adopted revision names",
        );
        assert_eq!(
            p.staged_edit(),
            Some(&PimdirHash::from("h-base")),
            "so the ancestor the resolution kept is a body to push",
        );
    }

    /// A create collision has no ancestor, and left base-less it never pushes.
    #[test]
    fn a_resolution_gives_a_base_less_conflict_a_base() {
        use crate::object::{PimdirHash, PimdirObject};

        let mutation = PimdirMutation::Edit {
            sort_key: Default::default(),
            handle: PimdirHandle::from("1"),
            object: PimdirObject {
                hash: PimdirHash::from("h-merged"),
                size: 6,
            },
            body: b"merged".to_vec(),
            summary: None,
        };
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);

        let mut loaded = loaded("1");
        loaded.placements[0].status = PimdirStatus::Conflict;
        loaded.placements[0].object = Some(PimdirHash::from("h-local"));
        loaded.placements[0].level = PimdirLevel::Full;
        loaded.placements[0].conflict_revision = Some("r2".into());
        loaded.placements[0].conflict_object = Some(PimdirHash::from("h-remote"));
        loaded.placements[0].base = None;

        let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };
        let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
            panic!("expected UpsertPlacement, got {:?}", ops[1]);
        };
        let base = p.base.as_ref().expect("the resolution establishes a base");
        assert_eq!(base.revision.as_deref(), Some("r2"));
        assert_eq!(base.object, Some(PimdirHash::from("h-remote")));
        assert_eq!(base.flags, p.flags, "nothing else is known of it");
    }

    /// A summary projected from the edited body replaces the cached one.
    #[test]
    fn edit_refreshes_the_projected_meta() {
        use crate::object::{PimdirHash, PimdirObject};

        let mutation = PimdirMutation::Edit {
            sort_key: Default::default(),
            handle: PimdirHandle::from("1"),
            object: PimdirObject {
                hash: PimdirHash::from("h2"),
                size: 4,
            },
            body: b"body".to_vec(),
            summary: Some(crate::summary::stub("fresh")),
        };
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);

        let ops = match mutate.resume(Some(PimdirArg::Load(loaded("1")))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };
        let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
            panic!("expected UpsertPlacement, got {:?}", ops[1]);
        };
        assert_eq!(p.summary, Some(crate::summary::stub("fresh")));
    }

    /// The base adopts the observed revision and the recorded pair is cleared.
    #[test]
    fn edit_resolves_a_conflict() {
        use crate::object::{PimdirHash, PimdirObject};

        let mutation = PimdirMutation::Edit {
            sort_key: Default::default(),
            handle: PimdirHandle::from("1"),
            object: PimdirObject {
                hash: PimdirHash::from("h3"),
                size: 6,
            },
            body: b"merged".to_vec(),
            summary: None,
        };
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);

        let mut loaded = loaded("1");
        loaded.placements[0].status = PimdirStatus::Conflict;
        loaded.placements[0].conflict_revision = Some("r2".into());
        loaded.placements[0].conflict_object = Some(PimdirHash::from("h-remote"));

        let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };
        let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
            panic!("expected UpsertPlacement, got {:?}", ops[1]);
        };

        assert_eq!(p.status, PimdirStatus::Dirty);
        assert_eq!(p.conflict_revision, None);
        assert_eq!(
            p.conflict_object, None,
            "the diverging body is dropped with the revision it named"
        );
        let base = p.base.as_ref().expect("a base");
        assert_eq!(base.revision.as_deref(), Some("r2"));
    }

    /// Services the target read a staged create makes, answering `holds`.
    fn stage(mutate: &mut PimdirMutate, holds: Vec<PimdirPlacement>) -> Vec<PimdirWriteOp> {
        let loaded = PimdirLoaded {
            placements: holds,
            checkpoint: None,
        };
        match mutate.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        }
    }

    /// The staged create carries the origin, so the push is a server copy.
    #[test]
    fn copy_stages_created_placement_in_target() {
        let mutation = PimdirMutation::Copy {
            handle: PimdirHandle::from("1"),
            target: "archive".into(),
            placeholder: PimdirHandle::from("tmp-1"),
        };
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);

        match mutate.resume(Some(PimdirArg::Load(loaded("1")))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsLoad { collection, scope }) => {
                assert_eq!(collection.as_str(), "archive");
                assert_eq!(
                    scope,
                    PimdirLoadScope::Links(vec![
                        PimdirLinkId::from("1"),
                        PimdirLinkId::from("dup:1#tmp-1"),
                    ]),
                );
            }
            state => panic!("expected WantsLoad, got {state:?}"),
        }

        let ops = stage(&mut mutate, Vec::new());
        let PimdirWriteOp::UpsertPlacement(p) = &ops[0] else {
            panic!("expected UpsertPlacement, got {:?}", ops[0]);
        };
        assert_eq!(p.collection.as_str(), "archive");
        assert_eq!(p.handle.as_str(), "tmp-1");
        assert_eq!(
            p.link_id,
            Some(PimdirLinkId::from("1")),
            "the identity is the source's while the target has it free",
        );
        assert_eq!(p.status, PimdirStatus::Created);
        assert!(p.base.is_none());
        let origin = p.origin.as_ref().expect("the copy carries its origin");
        assert_eq!(origin.collection.as_str(), "inbox");
        assert_eq!(origin.handle.as_str(), "1");
    }

    /// The target's half copies and the source's half removes.
    #[test]
    fn move_stages_target_create_and_source_tombstone() {
        let mutation = PimdirMutation::Move {
            handle: PimdirHandle::from("1"),
            target: "archive".into(),
            placeholder: PimdirHandle::from("tmp-1"),
        };
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);
        let _ = mutate.resume(Some(PimdirArg::Load(loaded("1"))));
        let ops = stage(&mut mutate, Vec::new());

        let PimdirWriteOp::UpsertPlacement(create) = &ops[0] else {
            panic!("expected UpsertPlacement, got {:?}", ops[0]);
        };
        assert_eq!(create.collection.as_str(), "archive");
        assert_eq!(create.handle.as_str(), "tmp-1");
        assert_eq!(create.status, PimdirStatus::Created);
        assert!(create.base.is_none());
        assert_eq!(
            create
                .origin
                .as_ref()
                .expect("the move carries its origin")
                .handle
                .as_str(),
            "1",
        );

        let PimdirWriteOp::UpsertPlacement(source) = &ops[1] else {
            panic!("expected UpsertPlacement, got {:?}", ops[1]);
        };
        assert_eq!(
            source.collection.as_str(),
            "inbox",
            "the source row, tombstoned"
        );
        assert_eq!(source.status, PimdirStatus::Tombstone);
        assert_eq!(
            source
                .origin
                .as_ref()
                .expect("a move destination, so a source-first sync relocates rather than deletes")
                .collection
                .as_str(),
            "archive",
        );
    }

    /// The copy lands beside the held identity as the second resource it is.
    ///
    /// Under the same key, a storage keying by identity would keep one of
    /// the two rows, and the other's body with it.
    #[test]
    fn a_copy_into_a_collection_holding_the_identity_is_minted() {
        let mutation = PimdirMutation::Copy {
            handle: PimdirHandle::from("1"),
            target: "archive".into(),
            placeholder: PimdirHandle::from("tmp-1"),
        };
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);
        let _ = mutate.resume(Some(PimdirArg::Load(loaded("1"))));

        let mut holder = loaded("1").placements.remove(0);
        holder.collection = "archive".into();
        holder.handle = PimdirHandle::from("a1");
        let ops = stage(&mut mutate, vec![holder]);

        let PimdirWriteOp::UpsertPlacement(copy) = &ops[0] else {
            panic!("expected UpsertPlacement, got {:?}", ops[0]);
        };
        assert_eq!(copy.link_id, Some(PimdirLinkId::from("dup:1#tmp-1")));
    }

    /// A row on its way out holds no key against a create, as for an `Add`.
    #[test]
    fn a_tombstoned_holder_does_not_block_a_copy() {
        let mutation = PimdirMutation::Copy {
            handle: PimdirHandle::from("1"),
            target: "archive".into(),
            placeholder: PimdirHandle::from("tmp-1"),
        };
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);
        let _ = mutate.resume(Some(PimdirArg::Load(loaded("1"))));

        let mut holder = loaded("1").placements.remove(0);
        holder.collection = "archive".into();
        holder.handle = PimdirHandle::from("a1");
        holder.status = PimdirStatus::Tombstone;
        let ops = stage(&mut mutate, vec![holder]);

        let PimdirWriteOp::UpsertPlacement(copy) = &ops[0] else {
            panic!("expected UpsertPlacement, got {:?}", ops[0]);
        };
        assert_eq!(copy.link_id, Some(PimdirLinkId::from("1")));
    }

    /// A mutation touches one row; an `Add` sees only the rows it may hit.
    #[test]
    fn a_mutation_reads_only_what_it_edits() {
        let mut mutate = PimdirMutate::new("inbox", PimdirMutation::Remove("7".into()));
        match mutate.resume(None) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsLoad { scope, .. }) => {
                assert_eq!(
                    scope,
                    PimdirLoadScope::Handles(vec![PimdirHandle::from("7")])
                );
            }
            state => panic!("expected WantsLoad, got {state:?}"),
        }

        let add = PimdirMutation::Add {
            handle: PimdirHandle::from("tmp"),
            link_id: PimdirLinkId::from("m1"),
            flags: PimdirFlags::default(),
            object: PimdirObject {
                hash: PimdirHash::from("h"),
                size: 1,
            },
            body: vec![],
            summary: None,
            sort_key: Default::default(),
        };
        let mut mutate = PimdirMutate::new("inbox", add);
        match mutate.resume(None) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsLoad { scope, .. }) => {
                assert_eq!(
                    scope,
                    PimdirLoadScope::Links(vec![PimdirLinkId::from("m1")]),
                );
            }
            state => panic!("expected WantsLoad, got {state:?}"),
        }
    }

    /// An edited tombstone is revived, its destination going with the delete.
    #[test]
    fn an_edit_revives_a_tombstone_and_drops_its_destination() {
        use crate::object::{PimdirHash, PimdirObject};

        let mut loaded = loaded("1");
        loaded.placements[0].status = PimdirStatus::Tombstone;
        loaded.placements[0].origin = Some(PimdirOrigin {
            collection: "archive".into(),
            handle: PimdirHandle::from("1"),
        });

        let mutation = PimdirMutation::Edit {
            sort_key: None,
            handle: PimdirHandle::from("1"),
            object: PimdirObject {
                hash: PimdirHash::from("h2"),
                size: 4,
            },
            body: b"body".to_vec(),
            summary: None,
        };
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);
        let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };

        let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
            panic!("expected UpsertPlacement, got {:?}", ops[1]);
        };
        assert_eq!(p.status, PimdirStatus::Dirty);
        assert_eq!(p.origin, None, "a revived row is going nowhere: {p:?}",);
    }

    /// A flag change is not content: the delete stands, destination included.
    #[test]
    fn a_flag_change_leaves_a_tombstone_deleted() {
        let mut loaded = loaded("1");
        loaded.placements[0].status = PimdirStatus::Tombstone;
        loaded.placements[0].origin = Some(PimdirOrigin {
            collection: "archive".into(),
            handle: PimdirHandle::from("1"),
        });

        let mutation = PimdirMutation::SetFlags {
            handle: PimdirHandle::from("1"),
            flags: PimdirFlags::from_iter(["seen"]),
        };
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);
        let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };

        let PimdirWriteOp::UpsertPlacement(p) = &ops[0] else {
            panic!("expected UpsertPlacement, got {:?}", ops[0]);
        };
        assert_eq!(p.status, PimdirStatus::Tombstone);
        assert!(p.flags.contains("seen"), "the marker rides along");
        assert!(p.origin.is_some(), "and the move it was part of stands");
    }

    /// Editing the diverged tombstone a hub projects is the resolution.
    #[test]
    fn an_edit_resolves_a_divergence_a_tombstone_carries() {
        use crate::object::{PimdirHash, PimdirObject};

        let mut loaded = loaded("1");
        loaded.placements[0].status = PimdirStatus::Tombstone;
        loaded.placements[0].conflict_revision = Some("r2".into());
        loaded.placements[0].conflict_object = Some(PimdirHash::from("remote"));
        loaded.placements[0].base = Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: Some("r1".into()),
            object: Some(PimdirHash::from("h1")),
        });

        let mutation = PimdirMutation::Edit {
            sort_key: None,
            handle: PimdirHandle::from("1"),
            object: PimdirObject {
                hash: PimdirHash::from("merged"),
                size: 6,
            },
            body: b"merged".to_vec(),
            summary: None,
        };
        let mut mutate = PimdirMutate::new("inbox", mutation);
        let _ = mutate.resume(None);
        let ops = match mutate.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsWrite(ops)) => ops,
            state => panic!("expected WantsWrite, got {state:?}"),
        };

        let PimdirWriteOp::UpsertPlacement(p) = &ops[1] else {
            panic!("expected UpsertPlacement, got {:?}", ops[1]);
        };
        assert_eq!(p.status, PimdirStatus::Dirty);
        assert_eq!(p.conflict_revision, None, "the divergence is settled");
        assert_eq!(p.conflict_object, None);
        let base = p.base.as_ref().expect("a base");
        assert_eq!(
            base.revision.as_deref(),
            Some("r2"),
            "measured against the remote state it settled",
        );
        assert_eq!(base.object, Some(PimdirHash::from("remote")));
    }

    /// A failure names its cause, a contract break riding as the source.
    #[test]
    fn a_mutate_failure_says_which_it_is() {
        let unknown = PimdirMutateError::UnknownHandle("7".into());
        assert_eq!(
            unknown.to_string(),
            "Pimdir MUTATE failed: unknown handle 7",
        );
        assert!(error::Error::source(&unknown).is_none());

        let exists = PimdirMutateError::LinkExists("mid".into());
        assert_eq!(
            exists.to_string(),
            "Pimdir MUTATE failed: link id already present: mid",
        );

        let arg = PimdirMutateError::from(PimdirArgError::MissingArg);
        assert_eq!(arg.to_string(), PimdirArgError::MissingArg.to_string());
        assert!(error::Error::source(&arg).is_some());
    }
}
