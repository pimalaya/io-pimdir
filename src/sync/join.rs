//! # The merge's join
//!
//! The walk of local placements beside listed members in handle order,
//! one candidate per handle, and the rules narrowing it (SYNC §5): a
//! delta's, and a page's before the last of its round.

use core::{cmp::Ordering, iter::Peekable};

use alloc::{
    collections::{BTreeMap, BTreeSet, btree_map::IntoIter},
    string::String,
    vec::{IntoIter as VecIntoIter, Vec},
};

use crate::placement::{PimdirFlags, PimdirHandle, PimdirPlacement, PimdirStatus};

/// The remote side of a candidate: what a page listed for a handle, or
/// what the base stands in for when the listing did not name it.
///
/// The member's meta is held apart, by handle, so a synthesized side
/// needs none.
#[derive(Clone, Debug)]
pub(super) struct Remote {
    pub(super) handle: PimdirHandle,
    pub(super) flags: PimdirFlags,
    pub(super) revision: Option<String>,
}

/// How a page narrows the join to its candidates (SYNC §5).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Narrowing {
    /// A delta, or the last page of a round: the listed and vanished
    /// handles, plus every placement that is not clean, whose pending
    /// push the listing would never revisit.
    Delta,
    /// A page before the last: its members and vanished handles alone.
    Page,
}

/// The merge in progress: the page and how far the join walked.
///
/// Held across yields, because the merge is bounded like the pushes are:
/// it stops at a full write batch and picks up where it left off.
pub(super) struct Merge {
    pub(super) join: Join,
    /// The handles merged against no remote state: the ones the source
    /// states removed and, on a round's last page, the ones it found
    /// absent in scope.
    pub(super) vanished: BTreeSet<PimdirHandle>,
    pub(super) narrowing: Narrowing,
}

impl Merge {
    /// Narrows a joined handle to a candidate, or drops it as untouched.
    ///
    /// A vanished handle merges against no remote state, a listed one
    /// against what was listed. Under [`Narrowing::Delta`] an unlisted
    /// non-clean one is unchanged upstream, so its base stands in and its
    /// pending push derives; under [`Narrowing::Page`] it waits for the
    /// round's last page.
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
        if self.narrowing == Narrowing::Page {
            return None;
        }

        let local = candidate.local.as_ref()?;
        if local.status == PimdirStatus::Clean {
            return None;
        }

        // NOTE: a staged create has no base to synthesize a remote from,
        // and is still a candidate: its add is what a listing never names.
        let remote = local.base.as_ref().map(|base| Remote {
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
    pub(super) remote: Option<Remote>,
}

/// Walks local placements and remote members in handle order, pairing them.
///
/// Both sides are ordered already, the `BTreeMap` by nature and the page
/// by the sort the merge gave it, so the union is a two-pointer walk.
/// Owning both lets the merge take a placement rather than clone one.
pub(super) struct Join {
    local: Peekable<IntoIter<PimdirHandle, PimdirPlacement>>,
    remote: Peekable<VecIntoIter<Remote>>,
}

impl Join {
    pub(super) fn new(local: BTreeMap<PimdirHandle, PimdirPlacement>, remote: Vec<Remote>) -> Self {
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
