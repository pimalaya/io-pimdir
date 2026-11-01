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
}
