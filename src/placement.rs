//! # Placement
//!
//! An item's presence in one collection: handle, link id, level, flags
//! and sync base.
//!
//! One of the two identity axes, next to [`crate::object`]. It pins an
//! item to a collection through the protocol [`PimdirHandle`], carries
//! the per-location mutable state at some [`PimdirLevel`], and holds the
//! [`PimdirBase`] the three-way merge reconciles against.

use alloc::{
    collections::BTreeSet,
    string::{String, ToString},
};

use crate::{collection::PimdirCollectionId, object::PimdirHash, summary::PimdirSummary};

crate::pimdir_id! {
    /// The protocol's per-collection location of an item.
    ///
    /// IMAP uidvalidity plus uid, WebDAV href, JMAP id, always a string
    /// so non-integer ids are a non-issue.
    PimdirHandle, Ord, PartialOrd, Hash,
}

crate::pimdir_id! {
    /// The item identity grouping copies across collections and protocols.
    ///
    /// A source global id (a JMAP id), else a stable content id (the
    /// Message-ID header, the vCard or iCalendar UID). Never a per-copy
    /// value a provider may rewrite.
    PimdirLinkId, Ord, PartialOrd, Hash,
}

impl PimdirLinkId {
    /// The key a second copy of this identity takes in one collection.
    ///
    /// `dup:`, the hint, `#`, the handle verbatim: a form fixed by pimdir
    /// STORAGE §9, opaque and never parsed back. Derived from the hint and
    /// handle alone, so a rebuilt replica mints the same keys again.
    pub fn minted(&self, handle: &PimdirHandle) -> Self {
        let mut key = String::from("dup:");
        key.push_str(self.as_str());
        key.push('#');
        key.push_str(handle.as_str());

        Self(key)
    }

    /// This identity while free, else a [`minted`](Self::minted) one that is.
    ///
    /// Minting repeats while the key is taken, since an identity may itself
    /// be spelled like a minted one. It terminates: each round lengthens
    /// the key and `taken` names a finite set.
    pub fn claim(&self, handle: &PimdirHandle, taken: impl Fn(&Self) -> bool) -> Self {
        let mut key = self.clone();

        while taken(&key) {
            key = key.minted(handle);
        }

        key
    }
}

/// An item's position in its collection's natural order.
///
/// Opaque to the engine and derived beside the summary, from the same
/// parse. Empty means unknown and is the default, a plain value rather
/// than an `Option` because that is what the reference storage records.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirSortKey(pub String);

impl<T: Into<String>> From<T> for PimdirSortKey {
    fn from(key: T) -> Self {
        Self(key.into())
    }
}

impl PimdirSortKey {
    /// Borrows the key as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the key was never derived, as opposed to sorting first.
    pub fn is_unknown(&self) -> bool {
        self.0.is_empty()
    }
}

/// An item's set of state markers, normalized by the consumer.
///
/// A plain string set, the consumer folding equivalent spellings first.
/// [`Unknown`](Self::Unknown) is distinct from a known-empty set, or the
/// absence would be pushed onto the side that did read the markers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirFlags {
    /// Never read, only a local [probed](PimdirLevel::Probed) item is.
    Unknown,
    /// The markers as read, empty when the item carries none.
    Known(BTreeSet<String>),
}

impl PimdirFlags {
    /// Reports whether `flag` is present, which an unknown set never is.
    pub fn contains(&self, flag: &str) -> bool {
        match self {
            Self::Unknown => false,
            Self::Known(flags) => flags.contains(flag),
        }
    }

    /// Whether the set has never been read, as opposed to being empty.
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }

    /// The markers as read, or `None` while they are unknown.
    pub fn known(&self) -> Option<&BTreeSet<String>> {
        match self {
            Self::Unknown => None,
            Self::Known(flags) => Some(flags),
        }
    }

    /// Three-way merges two flag sets element-wise against their base.
    ///
    /// The side that flipped a flag wins it, so nothing ever conflicts:
    /// (local AND remote) OR (local MINUS base) OR (remote MINUS base). An
    /// unknown side holds no opinion, and an unknown base is no base.
    pub fn merge(base: &PimdirFlags, local: &PimdirFlags, remote: &PimdirFlags) -> PimdirFlags {
        let (Some(local), Some(remote)) = (local.known(), remote.known()) else {
            return match (local, remote) {
                (Self::Unknown, remote) => remote.clone(),
                (local, _) => local.clone(),
            };
        };
        let base = base.known().cloned().unwrap_or_default();

        let kept = local.intersection(remote).cloned();
        let local_adds = local.difference(&base).cloned();
        let remote_adds = remote.difference(&base).cloned();

        Self::Known(kept.chain(local_adds).chain(remote_adds).collect())
    }
}

/// A known-empty set: unknown is stated outright, never defaulted to.
impl Default for PimdirFlags {
    fn default() -> Self {
        Self::Known(BTreeSet::new())
    }
}

impl<S: ToString> FromIterator<S> for PimdirFlags {
    fn from_iter<I: IntoIterator<Item = S>>(flags: I) -> Self {
        Self::Known(flags.into_iter().map(|f| f.to_string()).collect())
    }
}

/// The detail level of a placement, a ladder each rung including the last.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PimdirLevel {
    /// Handle known, nothing else; kept complete per collection.
    Probed,
    /// Minimal summary cached.
    Meta,
    /// Linked to a stored object body.
    Full,
}

/// How a placement relates to its sync base.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirStatus {
    /// In sync with the base; no pending push.
    Clean,
    /// Locally changed since the base; a push is pending.
    Dirty,
    /// Locally deleted since the base; a remove is pending.
    Tombstone,
    /// Content diverged on both sides, awaiting a resolving edit.
    Conflict,
    /// Locally created, its handle provisional until the push assigns one.
    Created,
}

/// Where a pending create's body already lives, for a server-side copy.
///
/// `None` on the placement means a genuine append of content the server
/// has never seen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirOrigin {
    /// The collection the source member lives in.
    pub collection: PimdirCollectionId,
    /// The source member's handle.
    pub handle: PimdirHandle,
}

/// The last-synced state a placement reconciles against.
///
/// Its existence is the membership base: a based placement was a member
/// as of the last sync. `revision` detects an in-place edit where content
/// is mutable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirBase {
    /// Last-synced flag set.
    pub flags: PimdirFlags,
    /// Last-synced content revision, `None` where content is immutable.
    pub revision: Option<String>,
    /// Last-synced body, pinned so a content merge keeps its base bytes.
    ///
    /// `None` where content is immutable, the current object then always
    /// being the synced one.
    pub object: Option<PimdirHash>,
}

/// One item's presence in one collection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirPlacement {
    /// The collection this placement belongs to.
    pub collection: PimdirCollectionId,
    /// The protocol handle within that collection.
    pub handle: PimdirHandle,
    /// The cross-collection link id; `None` until [`PimdirLevel::Meta`].
    pub link_id: Option<PimdirLinkId>,
    /// The stored object body; `None` until [`PimdirLevel::Full`].
    pub object: Option<PimdirHash>,
    /// The current detail level.
    pub level: PimdirLevel,
    /// The summary and addresses (Annex A), `None` until fetched.
    ///
    /// Kept as a stale display fallback when a remote content change drops
    /// the level back to [`PimdirLevel::Probed`].
    pub summary: Option<PimdirSummary>,
    /// The sort key, derived beside the summary and as opaque here.
    pub sort_key: PimdirSortKey,
    /// The current flag set.
    pub flags: PimdirFlags,
    /// How this placement relates to its base.
    pub status: PimdirStatus,
    /// The remote revision a [`PimdirStatus::Conflict`] was observed at.
    pub conflict_revision: Option<String>,
    /// The remote body at `conflict_revision`, so a resolver needs no network.
    ///
    /// Set, taken into the base and dropped along with `conflict_revision`.
    /// The upgrade pass supplies it, so a conflict holding `None` here is
    /// listable already and resolvable once the body lands.
    pub conflict_object: Option<PimdirHash>,
    /// The last-synced base; `None` until first reconciled.
    pub base: Option<PimdirBase>,
    /// Where a [`PimdirStatus::Created`] body lives, `None` for an append.
    pub origin: Option<PimdirOrigin>,
}

impl PimdirPlacement {
    /// The body staged locally but never synced, if any.
    ///
    /// Says nothing about the status, which each caller guards: a
    /// [`PimdirStatus::Created`] placement has no base, so its body is
    /// staged too, a create rather than an edit.
    pub fn staged_edit(&self) -> Option<&PimdirHash> {
        let object = self.object.as_ref()?;
        let synced = self
            .base
            .as_ref()
            .is_some_and(|base| base.object.as_ref() == Some(object));

        (!synced).then_some(object)
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use crate::placement::{PimdirFlags, PimdirHandle, PimdirLinkId};

    fn flags(flags: &[&str]) -> PimdirFlags {
        PimdirFlags::from_iter(flags.iter().copied())
    }

    #[test]
    fn ids_convert_from_owned_and_borrowed_strings() {
        assert_eq!(
            PimdirHandle::from(String::from("1")),
            PimdirHandle::from("1")
        );
        assert_eq!(PimdirHandle::from("1").as_str(), "1");
        assert_eq!(
            PimdirLinkId::from(String::from("m")),
            PimdirLinkId::from("m")
        );
        assert_eq!(PimdirLinkId::from("m").as_str(), "m");
    }

    #[test]
    fn merge_keeps_unchanged_flags() {
        let base = flags(&["seen"]);
        let merged = PimdirFlags::merge(&base, &base, &base);
        assert_eq!(merged, base);
    }

    #[test]
    fn merge_takes_each_side_addition() {
        let merged = PimdirFlags::merge(&flags(&[]), &flags(&["a"]), &flags(&["b"]));
        assert_eq!(merged, flags(&["a", "b"]));
    }

    #[test]
    fn merge_takes_each_side_removal() {
        let base = flags(&["a", "b", "c"]);
        let merged = PimdirFlags::merge(&base, &flags(&["b", "c"]), &flags(&["a", "b"]));
        assert_eq!(merged, flags(&["b"]));
    }

    #[test]
    fn merge_removal_beats_the_other_side_keeping() {
        let base = flags(&["seen"]);
        let merged = PimdirFlags::merge(&base, &flags(&[]), &base);
        assert_eq!(merged, flags(&[]));
    }

    #[test]
    fn merge_agreeing_changes_converge() {
        let base = flags(&["old"]);
        let both = flags(&["new"]);
        let merged = PimdirFlags::merge(&base, &both, &both);
        assert_eq!(merged, both);
    }

    #[test]
    fn an_unknown_side_takes_the_other_and_two_stay_unknown() {
        let base = flags(&["seen"]);
        let known = flags(&["flagged"]);
        let unknown = PimdirFlags::Unknown;
        assert_eq!(PimdirFlags::merge(&base, &unknown, &known), known);
        assert_eq!(PimdirFlags::merge(&base, &known, &unknown), known);
        assert_eq!(PimdirFlags::merge(&base, &unknown, &unknown), unknown);
    }

    #[test]
    fn an_unknown_base_keeps_both_sides_markers() {
        let merged = PimdirFlags::merge(&PimdirFlags::Unknown, &flags(&["a"]), &flags(&["b"]));
        assert_eq!(merged, flags(&["a", "b"]));
    }

    #[test]
    fn an_unknown_set_is_not_an_empty_one() {
        assert_ne!(PimdirFlags::Unknown, flags(&[]));
        assert!(PimdirFlags::Unknown.is_unknown());
        assert!(!PimdirFlags::Unknown.contains("seen"));
        assert_eq!(PimdirFlags::Unknown.known(), None);
        assert!(!PimdirFlags::default().is_unknown());
    }
}
