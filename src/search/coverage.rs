//! # Coverage
//!
//! What a search answer could not see (SEARCH §8.2), as three axes a
//! client reads from one value rather than guesses: the local axis
//! (items whose content is not searchable here), the temporal axis (the
//! range the store is known to hold every member of) and the remote axis
//! (whether a consumer merged a server-side search in).
//!
//! The roll-up over collections and kinds keeps the worst of each axis,
//! so a value is never more complete than its least complete part. The
//! index fills it for every answer, and the store's `LIKE` reads fill it
//! too (`PimdirReader::search_coverage`), before any index exists.

use alloc::{collections::BTreeMap, string::String};

use crate::collection::PimdirScope;

/// What a search answer could not see, on three axes (SEARCH §8.2).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirSearchCoverage {
    /// The items in scope whose content is not searchable here.
    pub local: PimdirSearchLocal,
    /// The range the store is known to hold every member of.
    pub temporal: PimdirSearchTemporal,
    /// Whether a server-side search was merged in.
    pub remote: PimdirSearchRemote,
    /// Whether the query's date range leaves the horizon calendar
    /// occurrences are materialised within (SEARCH §7, §8.1), with a
    /// calendar in scope: an occurrence outside it is not seen. Always
    /// false for the store's own reads, which read no occurrence.
    pub horizon_left: bool,
}

impl PimdirSearchCoverage {
    /// Folds another answer's coverage in, keeping the worst of each axis:
    /// counts add up, the temporal bounds narrow, a non-exhaustive remote
    /// search wins.
    pub fn merge(&mut self, other: &Self) {
        for (kind, count) in &other.local.bodiless {
            *self.local.bodiless.entry(kind.clone()).or_default() += count;
        }
        self.local.unindexed += other.local.unindexed;
        self.local.behind = self.local.behind.max(other.local.behind);
        self.temporal.merge(&other.temporal);
        self.remote.merge(&other.remote);
        self.horizon_left |= other.horizon_left;
    }

    /// Whether the answer is the whole answer: nothing bodiless or
    /// unindexed in scope, the range wholly held and inside the horizon,
    /// and no server-side search left partial.
    pub fn is_complete(&self) -> bool {
        self.local.bodiless.values().all(|count| *count == 0)
            && self.local.unindexed == 0
            && self.temporal == PimdirSearchTemporal::Full
            && self.remote != PimdirSearchRemote::Augmented { exhaustive: false }
            && !self.horizon_left
    }
}

/// The local axis: items in scope a search cannot read the content of.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirSearchLocal {
    /// Live items holding no body, per collection kind (media type): a
    /// search matches their summary alone. Files are never counted: they
    /// are indexed from their summary whatever they hold (SEARCH §6).
    pub bodiless: BTreeMap<String, u64>,
    /// Live items whose last change the index has not folded in yet, so
    /// what it answers about them may be stale or missing. Always zero for
    /// the store's own reads, which need no index.
    pub unindexed: u64,
    /// How far the index is behind the store, whole: the change stamps the
    /// store drew above the index cursor (STORAGE §4.5, SEARCH §4), so a
    /// client can say the index is N changes behind and to run its
    /// indexer. Unlike `unindexed` it is not scoped to the query, and it
    /// counts deletions too. Always zero for the store's own reads.
    pub behind: u64,
}

/// The temporal axis: whether the store holds every member of the range
/// asked (STORAGE §10 coverage rows).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum PimdirSearchTemporal {
    /// Every queried collection holds every member of the range.
    #[default]
    Full,
    /// Some queried collection is not known to hold every member.
    Bounded {
        /// The instant below which the store is not known to hold every
        /// member, RFC 3339; `None` when the floor is not the cause.
        since: Option<String>,
        /// The instant from which it is not known to, exclusive; `None`
        /// when the ceiling is not the cause.
        until: Option<String>,
        /// Whether a source never closed a round over a queried
        /// collection, typically a first sync still filling in: nothing
        /// is known complete there.
        filling: bool,
    },
}

impl PimdirSearchTemporal {
    /// One collection's axis over the range `[since, until)`, RFC 3339
    /// instants with either side open, from each source's coverage: the
    /// scope of its last closed round, `None` when it never closed one.
    ///
    /// A collection no source syncs holds everything there is, so no
    /// coverage at all is [`Full`](Self::Full).
    pub fn over<'a>(
        coverages: impl IntoIterator<Item = Option<&'a PimdirScope>>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Self {
        let mut temporal = Self::Full;

        for coverage in coverages {
            let Some(scope) = coverage else {
                temporal.merge(&Self::Bounded {
                    since: None,
                    until: None,
                    filling: true,
                });
                continue;
            };

            let floor = match (&scope.since, since) {
                (None, _) => None,
                (Some(floor), Some(since)) if since >= floor.as_str() => None,
                (Some(floor), _) => Some(floor.clone()),
            };
            let ceiling = match (&scope.until, until) {
                (None, _) => None,
                (Some(ceiling), Some(until)) if until <= ceiling.as_str() => None,
                (Some(ceiling), _) => Some(ceiling.clone()),
            };

            if floor.is_some() || ceiling.is_some() {
                temporal.merge(&Self::Bounded {
                    since: floor,
                    until: ceiling,
                    filling: false,
                });
            }
        }

        temporal
    }

    /// Narrows this axis by another: the highest floor, the lowest
    /// ceiling, either filling.
    pub fn merge(&mut self, other: &Self) {
        let Self::Bounded {
            since: other_since,
            until: other_until,
            filling: other_filling,
        } = other
        else {
            return;
        };

        match self {
            Self::Full => *self = other.clone(),
            Self::Bounded {
                since,
                until,
                filling,
            } => {
                if other_since.as_deref() > since.as_deref() {
                    since.clone_from(other_since);
                }
                if let Some(other_until) = other_until
                    && until.as_ref().is_none_or(|until| other_until < until)
                {
                    *until = Some(other_until.clone());
                }
                *filling |= *other_filling;
            }
        }
    }
}

/// The remote axis: whether a consumer merged a server-side search into
/// the hits. The library holds no credentials and never searches a
/// server, so it answers [`LocalOnly`](Self::LocalOnly) and a consumer
/// that does search one says so in the same value.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum PimdirSearchRemote {
    /// Only the local store was searched.
    #[default]
    LocalOnly,
    /// A server-side search was merged in.
    Augmented {
        /// Whether the server answered every match, not a first page.
        exhaustive: bool,
    },
}

impl PimdirSearchRemote {
    /// Combines two answers' remote axes: augmented when either is, and
    /// exhaustive only when every augmented part is.
    pub fn merge(&mut self, other: &Self) {
        match (&*self, other) {
            (_, Self::LocalOnly) => (),
            (Self::LocalOnly, _) => *self = other.clone(),
            (Self::Augmented { exhaustive }, Self::Augmented { exhaustive: theirs }) => {
                *self = Self::Augmented {
                    exhaustive: *exhaustive && *theirs,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use crate::{collection::PimdirScope, search::coverage::*};

    fn bounded(since: Option<&str>, until: Option<&str>, filling: bool) -> PimdirSearchTemporal {
        PimdirSearchTemporal::Bounded {
            since: since.map(ToString::to_string),
            until: until.map(ToString::to_string),
            filling,
        }
    }

    #[test]
    fn the_roll_up_keeps_the_worst_of_each_axis() {
        let mut coverage = PimdirSearchCoverage::default();
        assert!(coverage.is_complete());

        let mut mail = PimdirSearchCoverage {
            temporal: bounded(Some("2026-01-01T00:00:00Z"), None, false),
            ..Default::default()
        };
        mail.local.bodiless.insert("message/rfc822".into(), 2);
        coverage.merge(&mail);

        let other = PimdirSearchCoverage {
            local: PimdirSearchLocal {
                unindexed: 3,
                ..Default::default()
            },
            temporal: bounded(
                Some("2026-06-01T00:00:00Z"),
                Some("2027-01-01T00:00:00Z"),
                true,
            ),
            remote: PimdirSearchRemote::Augmented { exhaustive: true },
            horizon_left: false,
        };
        coverage.merge(&other);
        coverage.merge(&mail);

        assert_eq!(coverage.local.bodiless["message/rfc822"], 4);
        assert_eq!(coverage.local.unindexed, 3);
        assert_eq!(
            coverage.temporal,
            bounded(
                Some("2026-06-01T00:00:00Z"),
                Some("2027-01-01T00:00:00Z"),
                true
            ),
            "the highest floor, the lowest ceiling, either filling"
        );
        assert_eq!(
            coverage.remote,
            PimdirSearchRemote::Augmented { exhaustive: true }
        );
        assert!(!coverage.is_complete());

        coverage
            .remote
            .merge(&PimdirSearchRemote::Augmented { exhaustive: false });
        assert_eq!(
            coverage.remote,
            PimdirSearchRemote::Augmented { exhaustive: false },
            "one partial server answer leaves the whole partial"
        );
    }

    #[test]
    fn a_range_is_full_only_inside_every_source_coverage() {
        let since = PimdirScope {
            since: Some("2026-01-01T00:00:00Z".into()),
            until: None,
        };
        let open = PimdirScope::default();

        let over = |scopes: &[Option<&PimdirScope>], from: Option<&str>| {
            PimdirSearchTemporal::over(scopes.iter().copied(), from, None)
        };

        assert_eq!(over(&[], None), PimdirSearchTemporal::Full, "no source");
        assert_eq!(over(&[Some(&open)], None), PimdirSearchTemporal::Full);
        assert_eq!(
            over(&[Some(&open), Some(&since)], Some("2026-03-01T00:00:00Z")),
            PimdirSearchTemporal::Full,
            "a range inside the floor"
        );
        assert_eq!(
            over(&[Some(&open), Some(&since)], None),
            bounded(Some("2026-01-01T00:00:00Z"), None, false),
            "an open range reaches below the floor"
        );
        assert_eq!(
            over(&[None, Some(&open)], None),
            bounded(None, None, true),
            "a source never closed a round"
        );
    }

    #[test]
    fn an_exhaustive_server_search_is_complete() {
        let coverage = PimdirSearchCoverage {
            remote: PimdirSearchRemote::Augmented { exhaustive: true },
            ..Default::default()
        };
        assert!(coverage.is_complete());

        let mut coverage = PimdirSearchCoverage::default();
        coverage.local.bodiless.insert("text/vcard".into(), 0);
        assert!(coverage.is_complete(), "a zero count is nothing bodiless");
    }
}
