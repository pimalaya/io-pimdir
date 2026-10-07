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
    placement::{PimdirHandle, PimdirLinkId, PimdirPlacement},
};

/// Which of a collection's placements a load has to return.
///
/// A floor, not a ceiling: a storage SHALL return at least the named
/// placements and MAY return the whole collection. Under-delivering is
/// wrong: a mutation blind to a colliding link id creates a duplicate.
/// `Handles` naming none asks for the sync state alone (checkpoint,
/// coverage and round), what a sync reads before it lists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirLoadScope {
    /// Every placement of the collection.
    All,
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
    /// On an `All` load while a round is open: the based bindings of the
    /// source the round has not stamped and whose item's date is in its
    /// scope or unknown (`list_unstamped_bindings`), the members its last
    /// page finds absent unless that page lists them.
    pub unstamped: Vec<PimdirHandle>,
}
