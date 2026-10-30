//! # Remote seam
//!
//! What a connector answers (SYNC §4): an enumeration, a fetch, a push,
//! each as a payload the verbs read, and the [`PimdirRemote`] trait the
//! std runner services them through.
//!
//! An enumeration is asked for as a [`PimdirEnumerate`]: a delta from the
//! checkpoint, or a round over a [`PimdirScope`], from its start or
//! resumed from a cursor. It is answered one page at a time, a
//! [`PimdirRemoteSnapshot`] whose every member arrives named by its
//! [`PimdirRemoteMeta`], or by [`PimdirEnumerated::CursorRejected`] when
//! the source refuses the cursor a resumed round handed it.
//!
//! While a round is open, a run resuming it first asks for the round's
//! head ([`PimdirListing::Head`], SYNC §4, §5): what changed since the
//! round began, so what arrives during a long round shows before it
//! closes. A connector states which heads it lists
//! ([`PimdirRemote::heads`]); one listing none is never asked for one.
//!
//! A push applies each [`PimdirChange`] on the terms SYNC §4 fixes: a
//! `SetFlags` adds and removes, never replaces; a precondition guards only
//! where the source enforces it, a source enforcing none leaving a window
//! in which a server change under the push is lost; and an identity named
//! to the server is the key's [`stated`](PimdirLinkId::stated) form, its
//! one leading `esc:` stripped. A connector mirroring collections between
//! sources deletes one on a server because another source stopped listing
//! it only when its configuration enables it, and then only when it is
//! empty or every member was reconciled in the run (SYNC §5): the engine
//! derives no such delete.

use alloc::{string::String, vec::Vec};

use crate::{
    change::PimdirChange,
    collection::{PimdirCheckpoint, PimdirCollectionId, PimdirCursor, PimdirScope},
    object::PimdirHash,
    placement::{PimdirFlags, PimdirHandle, PimdirLinkId, PimdirSortKey},
    summary::{PimdirDerivation, PimdirSummary},
};

/// The detail tier a fetch targets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirTier {
    /// Summary only: a header or property subset.
    ///
    /// A listing names every member at this tier already (SYNC §4); a
    /// `Meta` fetch remains to revisit a claim a row does not hold.
    Meta,
    /// The full item body; yields an object.
    Full,
}

/// What names a member without its body (SYNC §2): the identity hint,
/// the summary and address rows and the sort key of STORAGE Annex A.
///
/// Every member a listing carries carries it, read in the listing itself
/// (an IMAP `ENVELOPE` with the header fields Annex A needs, a Graph
/// `$select`, Gmail's metadata, a JMAP `Email/get`), so nothing reaches
/// the store unnamed. A kind whose meta is the body (DAV) fetches each
/// page's bodies before it hands the page over, and MAY carry the body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirRemoteMeta {
    /// The identity hint the content states, or the kind's fallback key
    /// (STORAGE §9): never a minted key, which the engine decides.
    pub link_id: PimdirLinkId,
    /// The summary and addresses Annex A derives, `None` for a component
    /// the format has no table for.
    ///
    /// A mail summary read without the body carries the attachment mark
    /// of Annex A.1 read without it: the source's own flag where it
    /// states one, else the top-level `Content-Type`
    /// ([`crate::summary::mail::derive_meta`]).
    pub summary: Option<PimdirSummary>,
    /// The sort key from the same derivation, empty when undefined.
    pub sort_key: PimdirSortKey,
    /// The body, when the listing read it (DAV); `None` otherwise.
    pub body: Option<PimdirFetchedBody>,
}

impl PimdirRemoteMeta {
    /// The meta a derivation names, carrying no body.
    pub fn new(derivation: PimdirDerivation) -> Self {
        Self {
            link_id: derivation.link_id,
            summary: derivation.summary,
            sort_key: derivation.sort_key,
            body: None,
        }
    }

    /// The same meta carrying the body it was derived from.
    pub fn with_body(mut self, body: PimdirFetchedBody) -> Self {
        self.body = Some(body);
        self
    }
}

impl From<PimdirDerivation> for PimdirRemoteMeta {
    fn from(derivation: PimdirDerivation) -> Self {
        Self::new(derivation)
    }
}

/// One member of a listed page: handle, flags, content revision and meta.
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
    /// What names the member (SYNC §4).
    pub meta: PimdirRemoteMeta,
}

/// One page of an enumeration (SYNC §4).
///
/// A page belongs to a round, `complete`, every member of the scope
/// listed across the round's pages, so a member a round never listed in
/// scope was deleted upstream; or to a delta, listing only what changed
/// since the checkpoint and naming removals in `vanished`. A round of
/// one page is the complete enumeration of a connector that does not
/// page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirRemoteSnapshot {
    /// The members this page lists.
    ///
    /// Sorted by handle, each handle once, so the merge walks it beside the
    /// local placements without indexing it. An unsorted page is sorted
    /// by the engine and a repeated handle collapsed to its first item.
    pub items: Vec<PimdirRemoteItem>,
    /// Handles the source states removed (an IMAP `VANISHED`, a Graph
    /// `@removed`), which apply whatever the scope.
    pub vanished: Vec<PimdirHandle>,
    /// Whether the page belongs to a round (true) or a delta (false).
    pub complete: bool,
    /// Whether the page closes its listing, which a delta's one page
    /// always does.
    pub last: bool,
    /// The resume cursor, on every page of a round but the last.
    pub cursor: Option<PimdirCursor>,
    /// The checkpoint, on the page where the source gives one: a delta's,
    /// a round's first where the source gives it up front (IMAP
    /// `HIGHESTMODSEQ`, Gmail's `historyId`), its last for Graph.
    pub checkpoint: Option<PimdirCheckpoint>,
}

impl PimdirRemoteSnapshot {
    /// A round of one page: every member of the scope, and the checkpoint.
    pub fn round(items: Vec<PimdirRemoteItem>, checkpoint: Option<PimdirCheckpoint>) -> Self {
        Self {
            items,
            vanished: Vec::new(),
            complete: true,
            last: true,
            cursor: None,
            checkpoint,
        }
    }

    /// One page of a round, the last when `cursor` is `None`.
    pub fn page(
        items: Vec<PimdirRemoteItem>,
        cursor: Option<PimdirCursor>,
        checkpoint: Option<PimdirCheckpoint>,
    ) -> Self {
        Self {
            items,
            vanished: Vec::new(),
            complete: true,
            last: cursor.is_none(),
            cursor,
            checkpoint,
        }
    }

    /// A delta: what changed since the checkpoint, and what was removed.
    pub fn delta(
        items: Vec<PimdirRemoteItem>,
        vanished: Vec<PimdirHandle>,
        checkpoint: PimdirCheckpoint,
    ) -> Self {
        Self {
            items,
            vanished,
            complete: false,
            last: true,
            cursor: None,
            checkpoint: Some(checkpoint),
        }
    }

    /// A band head (SYNC §4): the members dated from the head's instant,
    /// with no removal and no checkpoint. A delta head is answered as a
    /// [`delta`](Self::delta).
    pub fn band_head(items: Vec<PimdirRemoteItem>) -> Self {
        Self {
            items,
            vanished: Vec::new(),
            complete: false,
            last: true,
            cursor: None,
            checkpoint: None,
        }
    }
}

/// The heads a connector lists while a round is open (SYNC §4).
///
/// A connector states it as it states
/// [`scope_bound`](PimdirRemote::scope_bound), from what the source
/// offers: IMAP lists delta heads with QRESYNC and band heads without
/// it, Gmail and JMAP delta heads, Graph band heads, a connector that
/// cannot list one before its round closes none.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PimdirHeads {
    /// Lists a delta head: what changed since a checkpoint
    /// ([`PimdirHead::Delta`]).
    pub delta: bool,
    /// Lists a band head: every member the source dates from an instant
    /// ([`PimdirHead::Band`]).
    pub band: bool,
}

impl PimdirHeads {
    /// No head: the round resumes as it would without (the default).
    pub const NONE: Self = Self {
        delta: false,
        band: false,
    };

    /// Delta heads only.
    pub const DELTA: Self = Self {
        delta: true,
        band: false,
    };

    /// Band heads only.
    pub const BAND: Self = Self {
        delta: false,
        band: true,
    };

    /// Both: a delta head whenever the run holds a checkpoint to list
    /// from, else a band head.
    pub const BOTH: Self = Self {
        delta: true,
        band: true,
    };
}

/// The head a run asks for while a round is open (SYNC §4, §5).
///
/// Answered by one page ([`PimdirEnumerated::Page`]) whose `complete`,
/// `last` and `cursor` the engine ignores, a head never moving the
/// round's cursor; or by [`PimdirEnumerated::HeadRejected`] or
/// [`PimdirEnumerated::NoHead`]. The page is checked against the
/// request's scope, which is the run's own and not the band a band
/// round lists: the connector drops what its own check on the `Date`
/// puts out of it, a stated removal excepted, as for a delta.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirHead {
    /// What changed since this checkpoint, as a delta answers it: the
    /// members added or changed, each with its flags and meta, the
    /// handles the source states removed, and the checkpoint the source
    /// holds now, which the page MUST carry (IMAP: a QRESYNC `SELECT`
    /// with `CHANGEDSINCE`; Gmail: the history since the history id;
    /// JMAP: `Email/changes`; DAV: `sync-collection`). The engine stamps
    /// the members with the round's id and the round closes on that
    /// checkpoint, unless a later page carries a newer one.
    ///
    /// The checkpoint is the round's, or for a band round, which carries
    /// none of its own, the source's. A connector listing delta heads
    /// MUST read every later page's members anew, leaving out what the
    /// source no longer holds in the collection, so a page never lists a
    /// member as it stood before a removal a head stated.
    Delta(PimdirCheckpoint),
    /// Every member the source dates at or after `since` less a margin
    /// of two days, by an instant it keeps for each member that never
    /// moves back: the last time the member changed where the source
    /// states it and can filter on it, else when it arrived (Graph's
    /// `receivedDateTime`, IMAP's `INTERNALDATE` through `SEARCH SINCE`).
    /// The page carries no checkpoint and no vanished handle, a listing
    /// by date stating no removal, and the engine stamps none of its
    /// members.
    ///
    /// A connector listing band heads MUST list again, in the last page
    /// of every round, every member a band head of that round would list
    /// then, read no earlier than the checkpoint that page carries: that
    /// page is what keeps a band head's members when the round closes,
    /// whatever is gone being found absent.
    Band {
        /// The instant the round opened, RFC 3339 as the store stamped
        /// it (`round_started_at`).
        since: String,
    },
}

/// What a connector is asked to list (SYNC §5).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirListing {
    /// What changed since the checkpoint, checked against the scope: one
    /// page, always the last.
    Delta(PimdirCheckpoint),
    /// A round over the scope: from its first page when `cursor` is
    /// `None`, else resumed from it.
    Round {
        /// The resume cursor the last landed page left.
        cursor: Option<PimdirCursor>,
        /// Whether the round lists only the band a coverage lacks, for a
        /// connector whose checkpoint is bound to no scope
        /// ([`PimdirRemote::scope_bound`]): it keeps its checkpoint and
        /// hands none, and the engine ignores any it does.
        band: bool,
    },
    /// The head of the open round, listed before the round resumes
    /// (SYNC §5).
    Head(PimdirHead),
}

/// One enumeration request: the listing and the scope it lists (SYNC §4).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirEnumerate {
    /// A delta, a round or a head.
    pub listing: PimdirListing,
    /// The scope: a round lists it, a delta is checked against it. A
    /// connector MAY narrow its listing by the provider's received-date
    /// filter to a superset of it, with a margin of two days below
    /// `since` (one for IMAP `SENTSINCE`), and drops what its own check
    /// on the `Date` puts out of it; unbounded for every kind but mail.
    pub scope: PimdirScope,
}

impl PimdirEnumerate {
    /// The checkpoint a delta lists from, `None` for a round.
    pub fn checkpoint(&self) -> Option<&PimdirCheckpoint> {
        match &self.listing {
            PimdirListing::Delta(checkpoint) => Some(checkpoint),
            PimdirListing::Round { .. } | PimdirListing::Head(_) => None,
        }
    }

    /// The head asked for, `None` for a delta or a round.
    pub fn head(&self) -> Option<&PimdirHead> {
        match &self.listing {
            PimdirListing::Head(head) => Some(head),
            PimdirListing::Delta(_) | PimdirListing::Round { .. } => None,
        }
    }

    /// The cursor a resumed round lists from, `None` otherwise.
    pub fn cursor(&self) -> Option<&PimdirCursor> {
        match &self.listing {
            PimdirListing::Round { cursor, .. } => cursor.as_ref(),
            PimdirListing::Delta(_) | PimdirListing::Head(_) => None,
        }
    }
}

/// What an enumeration answered.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PimdirEnumerated {
    /// One page of the listing asked for.
    Page(PimdirRemoteSnapshot),
    /// The source refused the resume cursor (an expired Graph skip token,
    /// a Gmail page token): the round restarts under a new id rather than
    /// the connector answering from the beginning as if it resumed.
    CursorRejected,
    /// The source refused the position a head was asked from (an expired
    /// Gmail history id, a JMAP state it can no longer calculate changes
    /// from, a DAV sync token refused as invalid): the checkpoint the
    /// round would close with is void, so the round restarts over the
    /// run's scope, never a band (SYNC §5). A moved handle space (an
    /// IMAP `UIDVALIDITY` bump) is a failure naming a rekey, as for a
    /// delta.
    HeadRejected,
    /// The connector cannot list the head it was asked for (an IMAP
    /// server without QRESYNC asked for a delta head): the round resumes
    /// as it would without.
    NoHead,
}

impl From<PimdirRemoteSnapshot> for PimdirEnumerated {
    fn from(page: PimdirRemoteSnapshot) -> Self {
        Self::Page(page)
    }
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

impl PimdirFetchedBody {
    /// The content hash the body is stored under.
    pub fn hash(&self) -> &PimdirHash {
        match self {
            Self::Inline { hash, .. } | Self::Persisted { hash, .. } => hash,
        }
    }
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

    /// Lists one page of the collection: of a delta from the checkpoint,
    /// or of a round over the scope, from its start or from a cursor.
    ///
    /// A connector whose checkpoint the source rejects because the handle
    /// space changed (an IMAP `UIDVALIDITY` bump) fails and names a rekey
    /// (SYNC §8); one whose cursor the source rejects answers
    /// [`PimdirEnumerated::CursorRejected`].
    fn enumerate(
        &mut self,
        collection: &PimdirCollectionId,
        request: PimdirEnumerate,
    ) -> Result<PimdirEnumerated, Self::Error>;

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

    /// Whether this connector's checkpoint is bound to the scope it was
    /// made under (SYNC §4): a Graph delta link made under a `$filter`
    /// is; IMAP `CHANGEDSINCE`, Gmail's history and JMAP's state, all
    /// filtered locally, are not, and such a connector widening a scope
    /// lists only the band its coverage lacks. Bound by default, which
    /// always relists the whole wider scope.
    fn scope_bound(&self) -> bool {
        true
    }

    /// The heads this connector lists while a round is open (SYNC §4),
    /// none by default: such a connector is never asked for one, and an
    /// arrival during a round shows once the round has closed.
    ///
    /// A connector listing band heads takes on the last page's
    /// obligation [`PimdirHead::Band`] states.
    fn heads(&self) -> PimdirHeads {
        PimdirHeads::NONE
    }
}
