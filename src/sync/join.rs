//! # The merge's join
//!
//! The walk of local placements beside remote items in handle order, one
//! candidate per handle, and the delta rule narrowing it (SYNC §5).

use core::{cmp::Ordering, iter::Peekable};

use alloc::{
    collections::{BTreeMap, BTreeSet, btree_map::IntoIter},
    vec::{IntoIter as VecIntoIter, Vec},
};

use crate::{
    collection::PimdirCheckpoint,
    placement::{PimdirHandle, PimdirPlacement, PimdirStatus},
    remote::PimdirRemoteItem,
};

/// The merge in progress: the enumerate's report and how far the join walked.
///
/// Held across yields, because the merge is bounded like the pushes are:
/// it stops at a full write batch and picks up where it left off.
pub(super) struct Merge {
    pub(super) join: Join,
    /// The handles the delta reported gone, as a set the delta rule consults.
    pub(super) vanished: BTreeSet<PimdirHandle>,
    /// Whether the snapshot is the whole remote, so an omission is a removal.
    pub(super) complete: bool,
    /// The cursor checkpointed once every candidate and push is recorded.
    pub(super) checkpoint: PimdirCheckpoint,
}

impl Merge {
    /// Narrows a joined handle to a delta candidate, or drops it as untouched.
    ///
    /// A vanished handle merges against no remote state, a listed one
    /// against what was listed. An unlisted non-clean one is unchanged
    /// upstream, so its base stands in and its pending push derives.
    pub(super) fn narrow(&self, candidate: Candidate) -> Option<Candidate> {
        if self.vanished.contains(&candidate.handle) {
            return Some(Candidate {
                remote: None,
                ..candidate
            });
        }
        if candidate.remote.is_some() {
            return Some(candidate);
        }

        let local = candidate.local.as_ref()?;
        if local.status == PimdirStatus::Clean {
            return None;
        }

        // NOTE: a staged create has no base to synthesize a remote from,
        // and is still a candidate: its add is what a delta never lists.
        let remote = local.base.as_ref().map(|base| PimdirRemoteItem {
            handle: candidate.handle.clone(),
            flags: base.flags.clone(),
            // NOTE: a conflicted placement has observed a revision past its
            // base; synthesizing the base one would regress the tracking.
            revision: local
                .conflict_revision
                .clone()
                .or_else(|| base.revision.clone()),
        });

        Some(Candidate {
            remote,
            ..candidate
        })
    }
}

/// One handle to merge: its local placement, its remote state, or both.
pub(super) struct Candidate {
    pub(super) handle: PimdirHandle,
    pub(super) local: Option<PimdirPlacement>,
    pub(super) remote: Option<PimdirRemoteItem>,
}

/// Walks local placements and remote items in handle order, pairing them.
///
/// Both sides are ordered already, the `BTreeMap` by nature and the
/// snapshot by the sort the merge gave it, so the union is a two-pointer
/// walk. Owning both lets the merge take a placement rather than clone
/// one.
pub(super) struct Join {
    local: Peekable<IntoIter<PimdirHandle, PimdirPlacement>>,
    remote: Peekable<VecIntoIter<PimdirRemoteItem>>,
}

impl Join {
    pub(super) fn new(
        local: BTreeMap<PimdirHandle, PimdirPlacement>,
        remote: Vec<PimdirRemoteItem>,
    ) -> Self {
        Self {
            local: local.into_iter().peekable(),
            remote: remote.into_iter().peekable(),
        }
    }
}

impl Iterator for Join {
    type Item = Candidate;

    fn next(&mut self) -> Option<Candidate> {
        let side = match (self.local.peek(), self.remote.peek()) {
            (None, None) => return None,
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (Some((handle, _)), Some(item)) => handle.cmp(&item.handle),
        };

        let candidate = match side {
            Ordering::Less => {
                let (handle, local) = self.local.next()?;
                Candidate {
                    handle,
                    local: Some(local),
                    remote: None,
                }
            }
            Ordering::Greater => {
                let item = self.remote.next()?;
                Candidate {
                    handle: item.handle.clone(),
                    local: None,
                    remote: Some(item),
                }
            }
            Ordering::Equal => {
                let (handle, local) = self.local.next()?;
                Candidate {
                    handle,
                    local: Some(local),
                    remote: self.remote.next(),
                }
            }
        };

        Some(candidate)
    }
}
