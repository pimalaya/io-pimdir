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
    collection::PimdirCheckpoint,
    placement::{PimdirHandle, PimdirLinkId, PimdirPlacement},
};

/// Which of a collection's placements a load has to return.
///
/// A floor, not a ceiling: a storage SHALL return at least the named
/// placements and MAY return the whole collection. Under-delivering is
/// wrong: a mutation blind to a colliding link id creates a duplicate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirLoadScope {
    /// Every placement of the collection.
    All,
    /// The placements holding these handles.
    Handles(Vec<PimdirHandle>),
    /// Every placement holding one of these link ids, however many rows.
    Links(Vec<PimdirLinkId>),
}

/// A loaded collection: its placements and its last checkpoint.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirLoaded {
    /// Every placement currently stored for the collection.
    pub placements: Vec<PimdirPlacement>,
    /// The last sync checkpoint, if ever synced.
    pub checkpoint: Option<PimdirCheckpoint>,
}
