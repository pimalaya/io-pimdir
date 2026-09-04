//! # Coroutine contract
//!
//! The generator-shape contract every verb implements, mirroring
//! `core::ops::Coroutine` with a `Yield`, a `Return` and a two-variant
//! [`PimdirCoroutineState`].
//!
//! Every verb yields the standard [`PimdirYield`], which gathers every
//! effect the engine emits, remote (enumerate, fetch, push) and storage
//! (load, lookup object, write) alike. A consumer services each yield and
//! resumes with the matching [`PimdirArg`]; [`crate::client`] is one.

use core::{error, fmt};

use alloc::{collections::BTreeMap, vec::Vec};

use crate::{
    change::{PimdirChange, PimdirWriteOp},
    collection::{PimdirCheckpoint, PimdirCollectionId},
    load::{PimdirLoadScope, PimdirLoaded},
    object::PimdirHash,
    placement::{PimdirHandle, PimdirLinkId},
    remote::{PimdirFetchedItem, PimdirPushResult, PimdirRemoteSnapshot, PimdirTier},
};

/// State yielded by an [`PimdirCoroutine::resume`] step.
///
/// Two-variant by design, matching std's `core::ops::CoroutineState`: any
/// further variation lives inside the per-coroutine `Yield` type.
#[derive(Debug)]
pub enum PimdirCoroutineState<Y, R> {
    /// Intermediate yield: the caller reacts to `Y` and resumes.
    Yielded(Y),
    /// Terminal yield. By convention `R = Result<Output, Error>`.
    Complete(R),
}

/// A caller that broke the coroutine contract.
///
/// The only way any verb but [`mutate`](crate::mutate) fails, since
/// nothing a verb does can go wrong inside the engine. One type for every
/// verb, because it is one bug and it sits in the caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirArgError {
    /// An arg not matching the pending yield, or a resume after completion.
    UnexpectedArg,
    /// The caller resumed without the arg the pending yield required.
    MissingArg,
}

impl fmt::Display for PimdirArgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedArg => write!(f, "Pimdir coroutine failed: unexpected arg"),
            Self::MissingArg => write!(f, "Pimdir coroutine failed: missing arg"),
        }
    }
}

impl error::Error for PimdirArgError {}

/// Standard-shape offline coroutine.
///
/// Implementors own their state machine and declare their terminal
/// `Return`. The caller reacts to each [`PimdirYield`] variant and
/// resumes until `Complete`.
pub trait PimdirCoroutine {
    /// The value yielded on every step, always [`PimdirYield`] here.
    type Yield;
    /// Terminal value. By convention `Result<Output, Error>`.
    type Return;

    /// Advances one step, with the arg matching the previous yield.
    ///
    /// [`None`] on the initial call only.
    fn resume(&mut self, arg: Option<PimdirArg>)
    -> PimdirCoroutineState<Self::Yield, Self::Return>;
}

/// Standard offline Yield. Every verb picks `type Yield = PimdirYield`.
///
/// Each variant is paired with the [`PimdirArg`] the caller feeds back
/// on the next `resume`. The first group is the remote seam, the second
/// the storage seam.
#[derive(Debug)]
pub enum PimdirYield {
    /// Enumerate the remote collection, answered by [`PimdirArg::Enumerate`].
    WantsEnumerate {
        /// The collection to enumerate.
        collection: PimdirCollectionId,
        /// The last checkpoint to delta from, if any.
        cursor: Option<PimdirCheckpoint>,
    },
    /// Fetch each handle at `tier`, answered by [`PimdirArg::Fetch`].
    WantsFetch {
        /// The owning collection.
        collection: PimdirCollectionId,
        /// The handles to fetch.
        handles: Vec<PimdirHandle>,
        /// The detail tier.
        tier: PimdirTier,
    },
    /// Push each change, answered by [`PimdirArg::Push`].
    WantsPush {
        /// The owning collection.
        collection: PimdirCollectionId,
        /// The changes to push.
        changes: Vec<PimdirChange>,
    },
    /// Load the collection from storage, answered by [`PimdirArg::Load`].
    WantsLoad {
        /// The collection to read.
        collection: PimdirCollectionId,
        /// Which of its placements are needed.
        scope: PimdirLoadScope,
    },
    /// Resolve stored objects by link id, see [`PimdirArg::LookupObject`].
    ///
    /// The dedup check skipping the download of a body another collection
    /// already holds.
    WantsLookupObject(Vec<PimdirLinkId>),
    /// Apply the writes atomically, answered by [`PimdirArg::Write`].
    WantsWrite(Vec<PimdirWriteOp>),
}

/// Reply fed back into [`PimdirCoroutine::resume`] by the caller.
///
/// Each variant matches the corresponding [`PimdirYield`] request and
/// carries the value the caller gathered while servicing it.
#[derive(Clone, Debug)]
pub enum PimdirArg {
    /// Reply to [`PimdirYield::WantsEnumerate`].
    Enumerate(PimdirRemoteSnapshot),
    /// Reply to [`PimdirYield::WantsFetch`].
    Fetch(Vec<PimdirFetchedItem>),
    /// Reply to [`PimdirYield::WantsPush`].
    Push(Vec<PimdirPushResult>),
    /// Reply to [`PimdirYield::WantsLoad`].
    Load(PimdirLoaded),
    /// Reply to [`PimdirYield::WantsLookupObject`], the link ids found.
    LookupObject(BTreeMap<PimdirLinkId, PimdirHash>),
    /// Reply to [`PimdirYield::WantsWrite`].
    Write,
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use crate::coroutine::PimdirArgError;

    /// The two contract breaks read apart, so a consumer log tells them.
    #[test]
    fn the_two_contract_breaks_read_apart() {
        assert_eq!(
            PimdirArgError::UnexpectedArg.to_string(),
            "Pimdir coroutine failed: unexpected arg",
        );
        assert_eq!(
            PimdirArgError::MissingArg.to_string(),
            "Pimdir coroutine failed: missing arg",
        );
    }
}
