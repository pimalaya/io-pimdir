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
//!
//! [`PimdirSearchCoverage::is_complete`] is SEARCH §8.2's complete
//! answer, the lag included: one call says whether anything the coverage
//! reports could have changed the answer.

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
};

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
    /// For a query bounded by date, the calendar items in scope whose
    /// occurrences a bound of SEARCH §7 cut before the range's end
    /// (`cut_before`, SEARCH §8.2): the range does not see all of theirs.
    /// Always zero for the store's own reads.
    pub cut: u64,
    /// Whether the zone the query stated is another than the one the index
    /// read floating times and dates in (SEARCH §8.1), compared as names,
    /// for a query holding a date term with a calendar in scope: its days
    /// are read in one zone over times placed in another. Reported apart:
    /// it leaves an answer complete. Always false for the store's own
    /// reads.
    pub zone_differs: bool,
}

impl PimdirSearchCoverage {
    /// Folds another answer's coverage in, keeping the worst of each axis:
    /// counts add up, what the terms read is joined, the temporal bounds
    /// narrow, a non-exhaustive remote search wins.
    pub fn merge(&mut self, other: &Self) {
        for (kind, counts) in &other.local.kinds {
            self.local
                .kinds
                .entry(kind.clone())
                .or_default()
                .add(counts);
        }
        self.local.reads.merge(&other.local.reads);
        self.local.unindexed += other.local.unindexed;
        self.local.behind = self.local.behind.max(other.local.behind);
        self.temporal.merge(&other.temporal);
        self.remote.merge(&other.remote);
        self.horizon_left |= other.horizon_left;
        self.cut += other.cut;
        self.zone_differs |= other.zone_differs;
    }

    /// Whether the answer is complete (SEARCH §8.2): nothing its coverage
    /// reports could have changed it.
    ///
    /// The index lags the store by nothing (`behind` and `unindexed` both
    /// zero), the range is wholly held and inside the horizon, no series
    /// was cut before its end, no server-side search was left partial, and
    /// of the bodies out of sight: no kind counts a bodiless item,
    /// whatever the query; none counts a `missing` or an `unparseable`
    /// placement while a term reads a body of that kind; and none counts
    /// an `encrypted` one while a text term is matched in the `body` or
    /// the `attachment` field. A zone that differs leaves it complete.
    pub fn is_complete(&self) -> bool {
        let reads = &self.local.reads;
        let read = self.local.kinds.iter().all(|(kind, counts)| {
            counts.bodiless == 0
                && (!reads.kinds.contains(kind) || counts.missing + counts.unparseable == 0)
                && (!reads.content || counts.encrypted == 0)
        });

        read && self.local.unindexed == 0
            && self.local.behind == 0
            && self.temporal == PimdirSearchTemporal::Full
            && self.remote != PimdirSearchRemote::Augmented { exhaustive: false }
            && !self.horizon_left
            && self.cut == 0
    }
}

/// The local axis: items in scope a search cannot read the content of.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirSearchLocal {
    /// The bodies out of sight among the query's collections, per
    /// collection kind (media type), a kind counting nothing left out
    /// (`coverage`, SEARCH §8.2). Files are never counted: they are
    /// indexed from their summary whatever they hold (SEARCH §6).
    pub kinds: BTreeMap<String, PimdirSearchUnread>,
    /// The bodies the query's terms read, which says whether a count of
    /// `kinds` leaves the answer incomplete. Empty for the store's own
    /// reads, which match summaries alone.
    pub reads: PimdirSearchReads,
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

/// One kind's bodies out of sight among the query's collections (SEARCH
/// §8.2), the four counts `coverage` answers. A body filed in several
/// collections counts once per placement, and a placement recorded missing
/// whose body the index holds unparseable or encrypted counts under both.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PimdirSearchUnread {
    /// Live items holding no body, read from the store: a search matches
    /// their summary alone.
    pub bodiless: u64,
    /// Placements whose body the indexer could not read (SEARCH §4), read
    /// from the index: indexed from their summary until it is.
    pub missing: u64,
    /// Placements whose body no parser accepted (SEARCH §6), read from
    /// the index: indexed from their summary.
    pub unparseable: u64,
    /// Placements whose body is encrypted (SEARCH §6), read from the
    /// index: indexed by its headers alone. The store's own reads count
    /// none, knowing no extraction.
    pub encrypted: u64,
}

impl PimdirSearchUnread {
    /// Whether nothing is counted.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Adds another count in.
    fn add(&mut self, other: &Self) {
        self.bodiless += other.bodiless;
        self.missing += other.missing;
        self.unparseable += other.unparseable;
        self.encrypted += other.encrypted;
    }
}

/// The bodies a query's terms read (SEARCH §8.2): a term reads a body
/// when the index answers it from what the extraction recorded of one,
/// wherever it stands, negated or not. `PimdirQuery::reads` derives it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirSearchReads {
    /// The collection kinds (media types) whose bodies a term reads.
    pub kinds: BTreeSet<String>,
    /// Whether a text term is matched in the `body` or the `attachment`
    /// field (free text, `body:`, `attachment:`, `mimetype:`), the fields
    /// an encrypted body gives none of.
    pub content: bool,
}

impl PimdirSearchReads {
    /// Joins what another query's terms read.
    pub fn merge(&mut self, other: &Self) {
        self.kinds.extend(other.kinds.iter().cloned());
        self.content |= other.content;
    }
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
        mail.local.kinds.insert(
            "message/rfc822".into(),
            PimdirSearchUnread {
                bodiless: 2,
                encrypted: 1,
                ..Default::default()
            },
        );
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
            cut: 1,
            zone_differs: true,
        };
        coverage.merge(&other);
        coverage.merge(&mail);

        assert_eq!(
            coverage.local.kinds["message/rfc822"],
            PimdirSearchUnread {
                bodiless: 4,
                encrypted: 2,
                ..Default::default()
            }
        );
        assert!(coverage.zone_differs);
        assert_eq!(coverage.local.unindexed, 3);
        assert_eq!(coverage.cut, 1);
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
        coverage
            .local
            .kinds
            .insert("text/vcard".into(), PimdirSearchUnread::default());
        assert!(coverage.is_complete(), "a zero count is nothing bodiless");
    }

    /// One kind counting `counts`, under a query whose terms read `reads`
    /// and, with `content`, the body or the attachment field.
    fn unread(counts: PimdirSearchUnread, reads: &[&str], content: bool) -> PimdirSearchCoverage {
        let mut coverage = PimdirSearchCoverage::default();
        coverage.local.kinds.insert("message/rfc822".into(), counts);
        coverage.local.reads.kinds = reads.iter().map(ToString::to_string).collect();
        coverage.local.reads.content = content;
        coverage
    }

    #[test]
    fn a_body_out_of_sight_counts_only_against_a_term_that_reads_it() {
        let one = |key: &str| {
            let mut counts = PimdirSearchUnread::default();
            match key {
                "bodiless" => counts.bodiless = 1,
                "missing" => counts.missing = 1,
                "unparseable" => counts.unparseable = 1,
                _ => counts.encrypted = 1,
            }
            counts
        };
        let mail = &["message/rfc822"][..];
        let cards = &["text/vcard"][..];

        assert!(
            !unread(one("bodiless"), &[], false).is_complete(),
            "a bodiless item counts against every query"
        );
        for key in ["missing", "unparseable"] {
            assert!(unread(one(key), &[], false).is_complete(), "{key}: no read");
            assert!(
                unread(one(key), cards, true).is_complete(),
                "{key}: a term reading another kind's bodies"
            );
            assert!(
                !unread(one(key), mail, false).is_complete(),
                "{key}: a term reading this kind's"
            );
        }
        assert!(
            unread(one("encrypted"), mail, false).is_complete(),
            "the headers of an encrypted body are indexed"
        );
        assert!(
            !unread(one("encrypted"), mail, true).is_complete(),
            "its body and its attachments are not"
        );
    }

    #[test]
    fn a_lag_is_never_complete_and_another_zone_is() {
        let mut coverage = PimdirSearchCoverage::default();
        coverage.local.behind = 1;
        assert!(!coverage.is_complete(), "the index is behind the store");

        let coverage = PimdirSearchCoverage {
            zone_differs: true,
            ..Default::default()
        };
        assert!(coverage.is_complete(), "reported apart");
    }
}
