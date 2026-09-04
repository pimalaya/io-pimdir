//! # Remote seam
//!
//! What a connector answers (SYNC §4): an enumeration, a fetch, a push,
//! each as a payload the verbs read, and the [`PimdirRemote`] trait the
//! std runner services them through.

use alloc::{string::String, vec::Vec};

use crate::{
    change::PimdirChange,
    collection::{PimdirCheckpoint, PimdirCollectionId},
    object::PimdirHash,
    placement::{PimdirFlags, PimdirHandle, PimdirLinkId, PimdirSortKey},
    summary::PimdirSummary,
};

/// The detail tier a fetch targets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirTier {
    /// Summary only: a header or property subset.
    Meta,
    /// The full item body; yields an object.
    Full,
}

/// One row of an enumerate snapshot: handle, flags and content revision.
///
/// Enough to merge without fetching a body, which keeps a partial body
/// cache safe. No link id: enumeration only has to yield handles, and the
/// link id is resolved at the [`PimdirTier::Meta`] fetch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirRemoteItem {
    /// The protocol handle.
    pub handle: PimdirHandle,
    /// The current remote flag set.
    pub flags: PimdirFlags,
    /// The remote content revision (a WebDAV etag) of a mutable body.
    ///
    /// `None` where content is immutable, which the merge reads as
    /// unchanged, never as unknown.
    pub revision: Option<String>,
}

/// The result of enumerating a collection: its member set and checkpoint.
///
/// A complete snapshot lists every member, so a local placement missing
/// from `items` was deleted upstream. A delta snapshot (QRESYNC, a JMAP
/// changes query) lists only what changed and names removals in `vanished`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirRemoteSnapshot {
    /// The members observed: all of them when `complete`, else the delta.
    ///
    /// Sorted by handle, each handle once, so the merge walks it beside the
    /// local placements without indexing it. An unsorted snapshot is sorted
    /// by the engine and a repeated handle collapsed to its first item.
    pub items: Vec<PimdirRemoteItem>,
    /// Handles removed upstream since the cursor, empty when `complete`.
    pub vanished: Vec<PimdirHandle>,
    /// Whether `items` is the whole member set (true) or a delta (false).
    pub complete: bool,
    /// The checkpoint these items are current as of.
    pub checkpoint: PimdirCheckpoint,
}

/// The body a [`PimdirTier::Full`] fetch reports for an item.
///
/// A streaming consumer MAY persist the body into its blob store itself and
/// report it [`Persisted`](PimdirFetchedBody::Persisted), so the engine
/// never holds it in memory. Either way the object is indexed by hash.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirFetchedBody {
    /// The body bytes and their content hash, for the engine to store.
    Inline {
        /// Content hash of the bytes.
        hash: PimdirHash,
        /// The body bytes.
        bytes: Vec<u8>,
    },
    /// An object the consumer already persisted, recorded without bytes.
    Persisted {
        /// Content hash of the persisted object.
        hash: PimdirHash,
        /// Size of the persisted object, in bytes.
        size: usize,
    },
}

/// The result of fetching one item at a requested tier.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirFetchedItem {
    /// The fetched handle.
    pub handle: PimdirHandle,
    /// The resolved link id.
    pub link_id: PimdirLinkId,
    /// The summary and addresses Annex A derives, `None` for a component
    /// the format has no table for.
    pub summary: Option<PimdirSummary>,
    /// The sort key from the same derivation, empty when undefined.
    pub sort_key: PimdirSortKey,
    /// The body; `None` at [`PimdirTier::Meta`].
    pub body: Option<PimdirFetchedBody>,
    /// The remote revision of the fetched body, `None` when immutable.
    pub revision: Option<String>,
}

/// The outcome of pushing one change.
///
/// Pushes are at-least-once ([`crate::change::PimdirChange`]): a remove
/// whose target is already gone is [`Accepted`](Self::Accepted), since
/// rejecting it would keep the tombstone retrying forever.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirPushOutcome {
    /// The remote accepted the change.
    Accepted,
    /// Optimistic concurrency rejected it: the base was stale.
    Rejected,
}

/// The result of pushing one change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirPushResult {
    /// The handle the change targeted (the provisional one for an add).
    pub handle: PimdirHandle,
    /// Whether the remote accepted it.
    pub outcome: PimdirPushOutcome,
    /// The server-assigned handle of an accepted add, `None` otherwise.
    pub assigned: Option<PimdirHandle>,
    /// The revision the remote now holds after an accepted content push.
    ///
    /// `None` when the remote reports none, and for flag and remove pushes.
    pub revision: Option<String>,
}

/// The connector seam (SYNC §4): what a runner asks of IMAP, JMAP or DAV.
pub trait PimdirRemote {
    /// The error this remote raises.
    type Error;

    /// Enumerates the collection: a full set, or a delta from `cursor`.
    fn enumerate(
        &mut self,
        collection: &PimdirCollectionId,
        cursor: Option<PimdirCheckpoint>,
    ) -> Result<PimdirRemoteSnapshot, Self::Error>;

    /// Fetches each handle at the requested tier, results keyed by handle.
    fn fetch(
        &mut self,
        collection: &PimdirCollectionId,
        handles: Vec<PimdirHandle>,
        tier: PimdirTier,
    ) -> Result<Vec<PimdirFetchedItem>, Self::Error>;

    /// Pushes each change, returning an outcome each; pushes are
    /// at-least-once, keyed so a replay is recognised.
    fn push(
        &mut self,
        collection: &PimdirCollectionId,
        changes: Vec<PimdirChange>,
    ) -> Result<Vec<PimdirPushResult>, Self::Error>;
}
