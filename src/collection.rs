//! # Collection
//!
//! A mailbox, address book or calendar: the id every verb is scoped to,
//! the opaque sync token round-tripped between the two seams, and what a
//! source's listing covers (SYNC §5): the scope a round lists, the
//! coverage the last closed round left, and the round under way.

use alloc::{string::String, vec::Vec};

crate::pimdir_id! {
    /// The account-scoped identity of a collection.
    PimdirCollectionId, Ord, PartialOrd, Hash,
}

impl AsRef<str> for PimdirCollectionId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// An opaque per-collection sync token.
///
/// A QRESYNC pack, a JMAP state string or a WebDAV sync-token, never
/// inspected by the engine, only round-tripped between the two seams.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirCheckpoint(pub Vec<u8>);

/// The connector's opaque position in an open round (SYNC §4), landed
/// with each page so an interrupted round resumes from it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirCursor(pub Vec<u8>);

/// A scope `[since, until)` on a message's summary `date` (SYNC §5):
/// what a round lists and the only place its absence means anything.
///
/// Each bound is an RFC 3339 instant in UTC at seconds precision with the
/// `Z` designator, the form Annex A writes `date` in, so bounds and dates
/// compare as strings; `None` is an open bound. The default is the
/// unbounded scope, the only one a collection of another kind than mail
/// syncs under.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirScope {
    /// The floor, inclusive; `None` for no floor.
    pub since: Option<String>,
    /// The ceiling, exclusive; `None` for no ceiling.
    pub until: Option<String>,
}

impl PimdirScope {
    /// The scope with no bound at all.
    pub fn unbounded() -> Self {
        Self::default()
    }

    /// The scope from `since` on, with no ceiling: the usual mail scope.
    pub fn since(since: impl Into<String>) -> Self {
        Self {
            since: Some(since.into()),
            until: None,
        }
    }

    /// Whether neither bound is set.
    pub fn is_unbounded(&self) -> bool {
        self.since.is_none() && self.until.is_none()
    }

    /// Whether a message dated `date` is in scope: `since <= date <
    /// until`, or no usable date at all, which is in every scope.
    pub fn contains(&self, date: Option<&str>) -> bool {
        let Some(date) = date.filter(|date| !date.is_empty()) else {
            return true;
        };

        self.since.as_deref().is_none_or(|since| since <= date)
            && self.until.as_deref().is_none_or(|until| date < until)
    }

    /// Whether `other` lies inside this scope, bounds included: a
    /// checkpoint made under this scope serves `other` (SYNC §5).
    pub fn covers(&self, other: &Self) -> bool {
        let floor = match (&self.since, &other.since) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(mine), Some(theirs)) => mine <= theirs,
        };
        let ceiling = match (&self.until, &other.until) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(mine), Some(theirs)) => theirs <= mine,
        };

        floor && ceiling
    }

    /// The one band `wanted` reaches outside this scope when it adjoins
    /// it on a single side, with the span the two then cover together
    /// (SYNC §5): what a connector whose checkpoint is bound to no scope
    /// lists instead of the whole of `wanted`. `None` when `wanted` lies
    /// inside, reaches out on both sides, or leaves a gap.
    pub fn band(&self, wanted: &Self) -> Option<(Self, Self)> {
        if self.covers(wanted) {
            return None;
        }

        let below = match (&wanted.since, &self.since) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(theirs), Some(mine)) => theirs < mine,
        };
        let above = match (&wanted.until, &self.until) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(theirs), Some(mine)) => theirs > mine,
        };

        match (below, above) {
            // NOTE: the band ends at the coverage's floor, so the wanted
            // ceiling has to reach it or a gap is left unlisted.
            (true, false) => {
                let reaches = match (&wanted.until, &self.since) {
                    (Some(until), Some(floor)) => until >= floor,
                    _ => true,
                };
                reaches.then(|| {
                    let band = Self {
                        since: wanted.since.clone(),
                        until: self.since.clone(),
                    };
                    let span = Self {
                        since: wanted.since.clone(),
                        until: self.until.clone(),
                    };
                    (band, span)
                })
            }
            (false, true) => {
                let reaches = match (&wanted.since, &self.until) {
                    (Some(since), Some(ceiling)) => since <= ceiling,
                    _ => true,
                };
                reaches.then(|| {
                    let band = Self {
                        since: self.until.clone(),
                        until: wanted.until.clone(),
                    };
                    let span = Self {
                        since: self.since.clone(),
                        until: wanted.until.clone(),
                    };
                    (band, span)
                })
            }
            _ => None,
        }
    }
}

/// What one source's last closed round covered (STORAGE §4.3): the store
/// holds every member of the collection the source has in `scope`, and
/// the checkpoint serves it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirCoverage {
    /// The scope of the last round that closed.
    pub scope: PimdirScope,
    /// When it closed, an RFC 3339 instant stamped by SQLite.
    pub at: String,
}

/// A round under way (SYNC §5): opened by its first page, closed when
/// its last page lands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirRound {
    /// The scope the round lists.
    pub scope: PimdirScope,
    /// The connector's resume cursor the last landed page left.
    pub cursor: Option<PimdirCursor>,
    /// The checkpoint a page handed, which the round lands when it closes.
    pub checkpoint: Option<PimdirCheckpoint>,
    /// When the round opened, an RFC 3339 instant stamped by SQLite.
    pub started_at: String,
    /// Whether it lists only the band its coverage lacks, whose absence
    /// infers no delete of an undated member (SYNC §5).
    pub band: bool,
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use crate::collection::{PimdirCollectionId, PimdirScope};

    fn scope(since: Option<&str>, until: Option<&str>) -> PimdirScope {
        PimdirScope {
            since: since.map(String::from),
            until: until.map(String::from),
        }
    }

    #[test]
    fn id_converts_from_owned_and_borrowed_strings() {
        let owned = PimdirCollectionId::from(String::from("inbox"));
        let borrowed = PimdirCollectionId::from("inbox");
        assert_eq!(owned, borrowed);
        assert_eq!(owned.as_str(), "inbox");
    }

    #[test]
    fn a_scope_holds_its_floor_not_its_ceiling_and_every_undated_message() {
        let september = scope(Some("2026-09-01T00:00:00Z"), Some("2026-10-01T00:00:00Z"));
        assert!(september.contains(Some("2026-09-01T00:00:00Z")));
        assert!(!september.contains(Some("2026-10-01T00:00:00Z")));
        assert!(!september.contains(Some("2026-08-31T23:59:59Z")));
        assert!(september.contains(None));
        assert!(september.contains(Some("")));
        assert!(PimdirScope::unbounded().contains(Some("1970-01-01T00:00:00Z")));
    }

    #[test]
    fn a_scope_covers_the_scopes_inside_it() {
        let since_july = PimdirScope::since("2026-07-01T00:00:00Z");
        let since_september = PimdirScope::since("2026-09-01T00:00:00Z");
        assert!(since_july.covers(&since_september));
        assert!(!since_september.covers(&since_july));
        assert!(PimdirScope::unbounded().covers(&since_july));
        assert!(!since_july.covers(&PimdirScope::unbounded()));
    }

    #[test]
    fn a_widening_lists_the_one_band_it_lacks() {
        let coverage = PimdirScope::since("2026-09-01T00:00:00Z");
        let wanted = PimdirScope::since("2026-07-01T00:00:00Z");
        let (band, span) = coverage.band(&wanted).expect("one band");
        assert_eq!(
            band,
            scope(Some("2026-07-01T00:00:00Z"), Some("2026-09-01T00:00:00Z"))
        );
        assert_eq!(span, wanted);

        let september = scope(Some("2026-09-01T00:00:00Z"), Some("2026-10-01T00:00:00Z"));
        let (band, span) = september
            .band(&PimdirScope::since("2026-09-01T00:00:00Z"))
            .expect("one band above");
        assert_eq!(band, PimdirScope::since("2026-10-01T00:00:00Z"));
        assert_eq!(span, PimdirScope::since("2026-09-01T00:00:00Z"));

        assert_eq!(september.band(&PimdirScope::unbounded()), None);
        assert_eq!(coverage.band(&coverage), None);
        let gap = scope(Some("2026-01-01T00:00:00Z"), Some("2026-02-01T00:00:00Z"));
        assert_eq!(coverage.band(&gap), None);
    }
}
