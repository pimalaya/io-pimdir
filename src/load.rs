//! # Storage seam
//!
//! Payloads the storage seam returns to a read, symmetric to
//! [`crate::remote`].
//!
//! The consumer answers storage from its index plus blob store (sqlite
//! plus a blob dir in the reference store). Writes travel the other way
//! as [`crate::change::PimdirWriteOp`].

use alloc::vec::Vec;

use crate::{
    collection::{PimdirCheckpoint, PimdirCoverage, PimdirRound},
    object::PimdirHash,
    placement::{PimdirHandle, PimdirLinkId, PimdirPlacement, PimdirSuperseded},
};

/// Which of a collection's placements a load has to return, its
/// selection (STORAGE §2, §14).
///
/// A floor, not a ceiling: a storage SHALL return at least the named
/// placements and MAY return the whole collection. Under-delivering is
/// wrong: a mutation blind to a colliding link id creates a duplicate.
/// `Handles` naming none asks for the sync state alone (checkpoint,
/// coverage and round), what a sync reads before it lists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirLoadSelection {
    /// Every placement of the collection.
    All,
    /// Every placement of the collection, as [`All`](Self::All), for the
    /// last page of the source's open round, with the bindings the round
    /// finds absent ([`PimdirLoaded::unstamped`]): `list_unstamped_bindings`
    /// bound with what the page states of its listing (SYNC §4, §5).
    LastPage {
        /// Whether a date filter narrowed the round's listing, whose
        /// silence then proves only the absence of a member dated before
        /// the round opened (`:narrowed`).
        narrowed: bool,
        /// The collection's handles an unfiltered enumeration listed, on
        /// the last page of a narrowed round: any member they lack is
        /// absent (`:enumerated` and the `handles` slot).
        enumerated: Option<Vec<PimdirHandle>>,
    },
    /// The placements holding these handles.
    Handles(Vec<PimdirHandle>),
    /// The placements holding these handles, and every one that may owe
    /// the source a push or a propagation: the bindings
    /// `list_owing_bindings` names and the items `list_lacking_items`
    /// names (STORAGE §14, SYNC §10). What a delta loads, so its load
    /// grows with what moved, not with the collection.
    Owing(Vec<PimdirHandle>),
    /// Every placement holding one of these link ids, however many rows.
    Links(Vec<PimdirLinkId>),
}

/// A loaded collection: its placements and its source's sync state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirLoaded {
    /// Every placement currently stored for the collection.
    pub placements: Vec<PimdirPlacement>,
    /// The last sync checkpoint, if ever synced.
    pub checkpoint: Option<PimdirCheckpoint>,
    /// What the source's last closed round covered, `None` before one
    /// closed (STORAGE §4.3).
    pub coverage: Option<PimdirCoverage>,
    /// The source's round under way, if any (SYNC §5).
    pub round: Option<PimdirRound>,
    /// On a [`LastPage`](PimdirLoadSelection::LastPage) load while a round
    /// is open: the based bindings of the source the round has not
    /// stamped, whose item's date is in its scope or unknown and whose
    /// absence its listing proves (`list_unstamped_bindings`), the members
    /// its last page finds absent unless that page lists them. Empty on
    /// any other load.
    pub unstamped: Vec<PimdirHandle>,
    /// The handles of the loaded tombstones whose remove is held: an add
    /// superseded them and the item it created is still a pending create
    /// (`list_superseded_pending`, STORAGE §15.3, SYNC §5).
    pub held: Vec<PimdirHandle>,
    /// The member each loaded pending create may be moved from, keyed by
    /// the create's handle (`list_superseded_bindings`, SYNC §3).
    pub superseded: Vec<(PimdirHandle, PimdirSuperseded)>,
    /// The loaded placements whose item is conflicted (`items.conflicted`,
    /// SYNC §9), each with the item's diverging body, `None` while nobody
    /// has read it, as [`PimdirHub::conflicted`] answers: what an
    /// [`Edit`](crate::mutate::PimdirMutation::Edit) meets beside its
    /// binding's conflict, which the placement carries (SYNC §7).
    ///
    /// [`PimdirHub::conflicted`]: crate::hub::PimdirHub::conflicted
    pub conflicted: Vec<(PimdirHandle, Option<PimdirHash>)>,
    /// The loaded `Created` placements the source binds not: the copies
    /// the store offers it of items other sources hold (SYNC §3, §9), as
    /// [`PimdirHub::offered`] answers. Every other `Created` placement is
    /// a create of this source's own, pending or delivered, which a sync
    /// reports a member minted apart from (`Disagreed`, SYNC §6); a
    /// storage stating none has every create read as the source's own.
    ///
    /// [`PimdirHub::offered`]: crate::hub::PimdirHub::offered
    pub offered: Vec<PimdirHandle>,
}
