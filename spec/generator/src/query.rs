//! # Search answers
//!
//! SEARCH §8 read over the model, written from SEARCH.md's text alone and
//! never from an implementation's, so a bug the two shared would not hide:
//! the hits of a query as `(account, seq)`, their order, and the coverage
//! an answer states (§8.2), from the terms the store answers, the words
//! the generator rendered into each body (§5, §6, §8.5), the occurrences
//! its calendar bodies expand to (§7) and the threads its mail forms (§9).
//!
//! The model never parses a body (README.md, "No parser"), so a fixture's
//! text, extraction status and occurrences are unknown to it. Every term
//! is three-valued on each hit: a hit it cannot place is
//! [`Tri::Unknown`], and an answer holding one, or an order or a count it
//! cannot settle, says so rather than guess. Where SEARCH reads two ways,
//! the oracle answers under both and settles only what they agree on.

pub mod date;
pub mod grammar;
pub mod text;
pub mod vectors;

use std::{
    cell::Cell,
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    ops::Not,
    ptr,
};

use rusqlite::types::Value;

use crate::{
    body::{Time, Times},
    model::{Item, Kind, Model, State, Table},
    query::{
        date::{DateValue, Span, Zone, stamps},
        grammar::{Node, ROLES, Refusal, TEXT_FIELDS, Term, reference_role},
        text::{Words, fold_name, tokens},
    },
    time::Instant,
};

/// The fields free text matches in (SEARCH §6, §8.5).
const FREE: [&str; 7] = [
    "title",
    "people",
    "body",
    "attachment",
    "place",
    "org",
    "note",
];

/// A stamp no store the oracle answers over draws: it writes far fewer
/// rows.
const STAMPS_MAX: i64 = 1_000_000_000;

/// How far from a horizon's bound an occurrence is too close to place on
/// either side of it, the bound being the indexer's to draw.
const NEAR_HORIZON: i64 = 2 * 86_400;

/// A truth that may not be known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tri {
    /// It holds.
    Yes,
    /// It does not.
    No,
    /// The model cannot say.
    Unknown,
}

impl Tri {
    /// A known truth.
    pub fn of(holds: bool) -> Self {
        match holds {
            true => Self::Yes,
            false => Self::No,
        }
    }

    /// Kleene's conjunction.
    pub fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::No, _) | (_, Self::No) => Self::No,
            (Self::Yes, Self::Yes) => Self::Yes,
            _ => Self::Unknown,
        }
    }

    /// Kleene's disjunction.
    pub fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::Yes, _) | (_, Self::Yes) => Self::Yes,
            (Self::No, Self::No) => Self::No,
            _ => Self::Unknown,
        }
    }

    /// Whether any holds.
    pub fn any(truths: impl IntoIterator<Item = Self>) -> Self {
        truths.into_iter().fold(Self::No, Self::or)
    }

    /// What every reading answers alike, else unknown.
    pub fn agree(truths: impl IntoIterator<Item = Self>) -> Self {
        let mut truths = truths.into_iter();
        let Some(first) = truths.next() else {
            return Self::Unknown;
        };
        match truths.all(|truth| truth == first) {
            true => first,
            false => Self::Unknown,
        }
    }

    /// The truth, when known.
    pub fn known(self) -> Option<bool> {
        match self {
            Self::Yes => Some(true),
            Self::No => Some(false),
            Self::Unknown => None,
        }
    }
}

impl Not for Tri {
    type Output = Self;

    /// Kleene's negation.
    fn not(self) -> Self {
        match self {
            Self::Yes => Self::No,
            Self::No => Self::Yes,
            Self::Unknown => Self::Unknown,
        }
    }
}

/// What the oracle answers over: a model, the properties its items hold
/// (STORAGE §4.3), which the store's writer leaves to its owner, the
/// placements an indexer could not read the body of (SEARCH §4), and the
/// index's horizon and floating zone (§7).
#[derive(Clone, Debug)]
pub struct Corpus {
    /// The store.
    pub model: Model,
    /// The properties, by `seq`.
    pub properties: Vec<Property>,
    /// The placements whose body the indexer could not read, by
    /// collection and `seq`.
    pub missing: BTreeSet<(String, i64)>,
    /// The placements whose summary the model does not know, a body it
    /// holds no derivation of: only what the store states beside it (its
    /// collection, `seq`, link id, flags and properties) decides a term.
    pub opaque: BTreeSet<(String, i64)>,
    /// The horizon occurrences are written within, Unix seconds.
    pub horizon: (i64, i64),
    /// The zone floating times and dates are placed in.
    pub floating: Zone,
    /// The readings it settles where SEARCH reads two ways.
    pub readings: Readings,
}

/// The readings the oracle states where SEARCH reads two ways, each none
/// by default, which leaves undecided what turns on it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Readings {
    /// Whether a series every rule of which ends before its `DTSTART`
    /// occurs at its `DTSTART` (finding G28 of io-pimdir tests/generated):
    /// RFC 5545 §3.8.5.3 has `DTSTART` define the first instance of the
    /// set, where ical-rs voids such a series as one deleted from its first
    /// instance on, and SEARCH §7 says neither.
    pub start_counts: Option<bool>,
}

/// One row of `item_property`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Property {
    /// The public id holding it.
    pub seq: i64,
    /// Its key.
    pub key: String,
    /// Its value.
    pub value: String,
}

/// The instant and the zone a query's dates resolve in (SEARCH §8.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clock {
    /// Its now, Unix seconds.
    pub now: i64,
    /// Its zone.
    pub zone: Zone,
}

/// The orders of SEARCH §8.2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// `newest-first`.
    NewestFirst,
    /// `oldest-first`.
    OldestFirst,
    /// `title`.
    Title,
    /// `sender`.
    Sender,
    /// `relevance`, the index's own.
    Relevance,
}

/// A hit's address (SEARCH §8.2).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hit {
    /// Its account, none in a single-account store.
    pub account: Option<String>,
    /// Its public id.
    pub seq: i64,
}

/// What a query answers, as far as the model decides it.
#[derive(Clone, Debug)]
pub struct Answer {
    /// Every item the store holds live, per account, with whether it is a
    /// hit.
    pub hits: BTreeMap<Hit, Tri>,
    /// The hits in the order asked, when every hit and the order are
    /// decided.
    pub order: Option<Vec<Hit>>,
    /// The coverage it states.
    pub coverage: Coverage,
    /// What each hit's text terms matched on, for the hits the model
    /// decides it of, none when no text term held for it.
    pub matched: BTreeMap<Hit, Option<Matched>>,
}

/// What a hit's text terms matched on (SEARCH §8.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Matched {
    /// A body the index read.
    Body,
    /// The summary of a placement whose body the index did not read.
    Summary,
}

impl Answer {
    /// The hits, when every item is decided.
    pub fn decided(&self) -> Option<BTreeSet<Hit>> {
        let mut hits = BTreeSet::new();
        for (hit, truth) in &self.hits {
            if truth.known()? {
                hits.insert(hit.clone());
            }
        }
        Some(hits)
    }

    /// The hits decided, and those the model cannot place.
    pub fn split(&self) -> (BTreeSet<Hit>, BTreeSet<Hit>) {
        let pick = |wanted: Tri| {
            self.hits
                .iter()
                .filter(|(_, truth)| **truth == wanted)
                .map(|(hit, _)| hit.clone())
                .collect()
        };
        (pick(Tri::Yes), pick(Tri::Unknown))
    }
}

/// The coverage an answer states (SEARCH §8.2), each part none where the
/// model cannot say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    /// The four counts per kind of the query's collections, by media
    /// type, a kind counting nothing left out.
    pub kinds: Option<BTreeMap<String, Counts>>,
    /// The temporal axis.
    pub temporal: Option<Temporal>,
    /// Whether the date range left the horizon.
    pub horizon_left: Option<bool>,
    /// How many calendar items were cut before the range's end.
    pub cut: Option<u64>,
    /// Whether the client's zone differs from the index's.
    pub zone_differs: Option<bool>,
    /// Whether the answer is complete, the lag taken as zero.
    pub complete: Option<bool>,
}

/// One kind's counts (SEARCH §8.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Live items holding no body.
    pub bodiless: u64,
    /// Placements whose body could not be read.
    pub missing: u64,
    /// Placements whose body no parser accepted.
    pub unparseable: u64,
    /// Placements whose body is encrypted.
    pub encrypted: u64,
}

/// The temporal axis (SEARCH §8.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Temporal {
    /// Every queried collection is known complete over the range.
    Full,
    /// Some is not.
    Bounded {
        /// The instant below which one is not known complete.
        since: Option<String>,
        /// The instant from which one is not.
        until: Option<String>,
        /// Whether a source never closed a round.
        filling: bool,
    },
}

impl Corpus {
    /// Answers `query` at `clock` in `order`, or the refusal §8.4 gives.
    pub fn ask(&self, query: &str, clock: &Clock, order: Order) -> Result<Answer, Refusal> {
        let node = grammar::parse(query)?;
        Ok(self.answer(&node, clock, order))
    }

    /// Answers a parsed query.
    pub fn answer(&self, node: &Node, clock: &Clock, order: Order) -> Answer {
        let mut terms = Vec::new();
        every_term(node, &mut terms);
        let mut conjuncts = Vec::new();
        conjuncts_of(node, &mut conjuncts);

        let asking = Asking {
            corpus: self,
            clock,
            candidates: self.candidates(),
            blind: Cell::new(false),
            undated_named: terms.iter().any(|term| {
                term.field.as_deref() == Some("kind")
                    && matches!(term.value.as_str(), "contact" | "file")
            }),
        };

        let truths = asking.eval(node);
        let scope = self.scope(&conjuncts);
        let hits: BTreeMap<Hit, Tri> = asking
            .candidates
            .iter()
            .zip(&truths)
            .map(|(candidate, truth)| (candidate.hit.clone(), *truth))
            .collect();

        let order = match truths.iter().all(|truth| truth.known().is_some()) {
            true => {
                let yes: Vec<&Candidate> = asking
                    .candidates
                    .iter()
                    .zip(&truths)
                    .filter(|(_, truth)| **truth == Tri::Yes)
                    .map(|(candidate, _)| candidate)
                    .collect();
                self.order(&yes, order, &scope)
            }
            false => None,
        };
        let coverage = self.coverage(&terms, &conjuncts, &scope, clock);
        let matched = asking.matched(node, &truths);

        Answer {
            hits,
            order,
            coverage,
            matched,
        }
    }

    /// Every live item of the store per account, its placements in
    /// collection order.
    fn candidates(&self) -> Vec<Candidate<'_>> {
        let mut grouped: BTreeMap<Hit, Vec<&Item>> = BTreeMap::new();
        for item in &self.model.items {
            if item.state != State::Live {
                continue;
            }
            let account = self.model.collections[&item.collection].account.clone();
            let hit = Hit {
                account,
                seq: item.seq,
            };
            grouped.entry(hit).or_default().push(item);
        }
        grouped
            .into_iter()
            .map(|(hit, mut placements)| {
                placements.sort_by(|a, b| a.collection.cmp(&b.collection));
                Candidate { hit, placements }
            })
            .collect()
    }

    /// The query's collections (SEARCH §8.2): every collection, less those
    /// a conjunct of the whole query rules out.
    fn scope(&self, conjuncts: &[&Term]) -> BTreeSet<String> {
        self.model
            .collections
            .values()
            .filter(|collection| {
                conjuncts.iter().all(|term| match term.field.as_deref() {
                    Some("account") => collection.account.as_deref() == Some(term.value.as_str()),
                    Some("collection" | "folder") => collection.id == term.value,
                    Some("kind") => collection.kind == Some(kind_named(&term.value)),
                    _ => true,
                })
            })
            .map(|collection| collection.id.clone())
            .collect()
    }

    fn is_missing(&self, item: &Item) -> bool {
        self.missing.contains(&(item.collection.clone(), item.seq))
    }

    fn is_opaque(&self, item: &Item) -> bool {
        self.opaque.contains(&(item.collection.clone(), item.seq))
    }

    /// Whether the index reads `item`'s body rather than its summary.
    fn bodied(&self, item: &Item) -> bool {
        item.object.is_some() && !self.is_missing(item)
    }

    /// The values a placement's document holds in `field`, under each
    /// reading of SEARCH §6 that differs; none when the model cannot say.
    fn field(&self, item: &Item, field: &str) -> Option<Vec<Vec<String>>> {
        if self.is_opaque(item) {
            return None;
        }
        let calendar = item.summary.table.kind() == Kind::Calendar;
        let people = matches!(field, "people" | "organizer" | "attendee");

        if self.bodied(item) {
            let object = item.object.as_ref()?;
            let values = match self.model.words.get(object) {
                Some(words) => words.fields.get(field).cloned().unwrap_or_default(),
                None if item.summary.table == Table::Mail
                    && matches!(field, "title" | "people" | "from" | "to" | "cc" | "bcc") =>
                {
                    self.summary_values(item, field)
                }
                None => return None,
            };
            if calendar && people {
                let uri = values
                    .iter()
                    .map(|value| match value.contains('@') {
                        true => format!("mailto:{value}"),
                        false => value.clone(),
                    })
                    .collect();
                return Some(vec![values, uri]);
            }
            return Some(vec![values]);
        }

        if calendar && field == "place" && item.summary.get("location") != Value::Null {
            return None;
        }
        Some(vec![self.summary_values(item, field)])
    }

    /// The values a placement's summary and addresses give `field` (SEARCH
    /// §6, a placement with no body).
    fn summary_values(&self, item: &Item, field: &str) -> Vec<String> {
        let named = |role: Option<&str>| {
            item.addresses
                .iter()
                .filter(|address| role.is_none_or(|role| address.role == role))
                .flat_map(|address| address.name.iter().chain([&address.address]).cloned())
                .collect()
        };
        match field {
            "title" => title(item).into_iter().collect(),
            "people" => named(None),
            role if ROLES.contains(&role) => named(Some(role)),
            _ => Vec::new(),
        }
    }

    /// The occurrences of a calendar placement under each reading, none
    /// when the model cannot expand its body. A placement whose body the
    /// index holds none of has no occurrence: SEARCH §7 expands bodies
    /// alone, and §8.1 compares occurrences, never a sort key.
    fn occurrences(&self, item: &Item) -> Option<Vec<Vec<(i64, i64)>>> {
        if self.is_opaque(item) {
            return None;
        }
        if !self.bodied(item) {
            return Some(vec![Vec::new()]);
        }
        let times = self
            .model
            .words
            .get(item.object.as_ref()?)?
            .times
            .as_ref()?;

        let mut readings = self.expand(times)?;
        let (start, end) = self.horizon;
        for occurrences in &mut readings {
            for (from, to) in occurrences.iter() {
                let near =
                    |at: i64| (at - start).abs() < NEAR_HORIZON || (at - end).abs() < NEAR_HORIZON;
                if near(*from) || near(*to) {
                    return None;
                }
            }
            occurrences.retain(|(from, to)| match from == to {
                true => start <= *from && *from < end,
                false => *from < end && *to > start,
            });
        }
        Some(readings)
    }

    /// SEARCH §7's occurrences of the times a body states, a journal's date
    /// read both as an instant and as its day, a series whose `UNTIL` falls
    /// before its `DTSTART` under [`Readings::start_counts`].
    fn expand(&self, times: &Times) -> Option<Vec<Vec<(i64, i64)>>> {
        let zone = self.floating;
        let wall = |time: &Time| match time {
            Time::Utc(at) => (at.0, true),
            Time::Floating(at) => (at.0, false),
            Time::Date(at) => (at.midnight().0, false),
        };
        let place = |(at, utc): (i64, bool)| match utc {
            true => Some(at),
            false => zone.instant(at),
        };

        match times.table {
            Table::Event => {
                let start = times.start?;
                let (from, utc) = wall(&start);
                let to = match (&times.end, start) {
                    (Some(end), _) => Some(wall(end).0),
                    (None, Time::Date(_)) => Some(from + 86_400),
                    (None, _) => None,
                };
                let first = place((from, utc))?;
                if times.until.is_some_and(|until| until.0 < first) && !self.readings.start_counts?
                {
                    return Some(vec![Vec::new()]);
                }
                let mut occurrences = Vec::new();
                for week in 0..10_000 {
                    let shift = week * 7 * 86_400;
                    let at = place((from + shift, utc))?;
                    if week > 0 && times.until.is_none_or(|until| at > until.0) {
                        break;
                    }
                    let until = match to {
                        Some(to) => place((to + shift, utc))?,
                        None => at,
                    };
                    occurrences.push((at, until));
                }
                Some(vec![occurrences])
            }
            Table::Task => match &times.end {
                Some(due) => {
                    let at = place(wall(due))?;
                    Some(vec![vec![(at, at)]])
                }
                None => Some(vec![Vec::new()]),
            },
            Table::Journal => match times.start {
                Some(Time::Date(day)) => {
                    let at = place((day.midnight().0, false))?;
                    let end = place((day.midnight().0 + 86_400, false))?;
                    Some(vec![vec![(at, at)], vec![(at, end)]])
                }
                Some(start) => {
                    let at = place(wall(&start))?;
                    Some(vec![vec![(at, at)]])
                }
                None => Some(vec![Vec::new()]),
            },
            _ => None,
        }
    }

    fn order(
        &self,
        hits: &[&Candidate],
        order: Order,
        scope: &BTreeSet<String>,
    ) -> Option<Vec<Hit>> {
        let opaque = hits
            .iter()
            .any(|candidate| candidate.placements.iter().any(|item| self.is_opaque(item)));
        if order == Order::Relevance || opaque {
            return None;
        }

        let mut lists: Vec<Vec<Hit>> = Vec::new();
        for scoped in [false, true] {
            for undated_empty in [false, true] {
                for seq_up in [true, false] {
                    for account_up in [true, false] {
                        let mut keyed = Vec::new();
                        for candidate in hits {
                            let within: Vec<&&Item> = candidate
                                .placements
                                .iter()
                                .filter(|item| !scoped || scope.contains(&item.collection))
                                .collect();
                            let held = match within.is_empty() {
                                true => candidate.placements.iter().collect(),
                                false => within,
                            };
                            let key = held
                                .iter()
                                .map(|item| item.sort_key.clone())
                                .max()
                                .unwrap_or_default();
                            let first = candidate.placements[0];
                            let undated =
                                matches!(first.summary.table, Table::Contact | Table::File)
                                    || (undated_empty && key.is_empty());
                            let titles: BTreeSet<String> = candidate
                                .placements
                                .iter()
                                .map(|item| fold_name(&title(item).unwrap_or_default()))
                                .collect();
                            if order == Order::Title && titles.len() > 1 {
                                return None;
                            }
                            let sender = match first.summary.table {
                                Table::Mail => match first.summary.get("sender") {
                                    Value::Text(sender) if !sender.is_empty() => {
                                        Some(fold_name(&sender))
                                    }
                                    _ => None,
                                },
                                _ => None,
                            };
                            let title = titles.into_iter().next().unwrap_or_default();
                            keyed.push((candidate.hit.clone(), key, undated, title, sender));
                        }

                        keyed.sort_by(|a, b| {
                            let newest = || match (a.2, b.2) {
                                (false, true) => Ordering::Less,
                                (true, false) => Ordering::Greater,
                                (true, true) => a.1.cmp(&b.1),
                                (false, false) => b.1.cmp(&a.1),
                            };
                            let primary = match order {
                                Order::NewestFirst => newest(),
                                Order::OldestFirst => match (a.2, b.2) {
                                    (false, true) => Ordering::Less,
                                    (true, false) => Ordering::Greater,
                                    _ => a.1.cmp(&b.1),
                                },
                                Order::Title => a.3.cmp(&b.3),
                                Order::Sender => match (&a.4, &b.4) {
                                    (Some(left), Some(right)) => left.cmp(right),
                                    (Some(_), None) => Ordering::Less,
                                    (None, Some(_)) => Ordering::Greater,
                                    (None, None) => newest(),
                                },
                                Order::Relevance => Ordering::Equal,
                            };
                            let seq = match seq_up {
                                true => a.0.seq.cmp(&b.0.seq),
                                false => b.0.seq.cmp(&a.0.seq),
                            };
                            let account = match account_up {
                                true => a.0.account.cmp(&b.0.account),
                                false => b.0.account.cmp(&a.0.account),
                            };
                            primary.then(seq).then(account)
                        });
                        lists.push(keyed.into_iter().map(|(hit, ..)| hit).collect());
                    }
                }
            }
        }

        let first = lists.first()?.clone();
        lists.iter().all(|list| *list == first).then_some(first)
    }

    fn coverage(
        &self,
        terms: &[&Term],
        conjuncts: &[&Term],
        scope: &BTreeSet<String>,
        clock: &Clock,
    ) -> Coverage {
        let collections: Vec<_> = scope.iter().map(|id| &self.model.collections[id]).collect();
        let calendars = collections
            .iter()
            .any(|collection| collection.kind == Some(Kind::Calendar));

        let mut kinds: BTreeMap<String, Counts> = BTreeMap::new();
        let mut unsure: BTreeSet<Kind> = BTreeSet::new();
        let mut cut = Some(0u64);
        for item in &self.model.items {
            let Some(kind) = self.model.collections[&item.collection].kind else {
                continue;
            };
            if item.state != State::Live || !scope.contains(&item.collection) || kind == Kind::File
            {
                continue;
            }
            let counts = kinds.entry(String::from(kind.media_type())).or_default();
            match &item.object {
                None => counts.bodiless += 1,
                Some(_) if self.is_missing(item) => counts.missing += 1,
                Some(object) if !self.model.words.contains_key(object) => {
                    unsure.insert(kind);
                    if kind == Kind::Calendar {
                        cut = None;
                    }
                }
                Some(_) => {}
            }
        }
        kinds.retain(|_, counts| *counts != Counts::default());

        let dates: Vec<&&Term> = terms
            .iter()
            .filter(|term| matches!(term.field.as_deref(), Some("date" | "when")))
            .collect();
        let range = match dates.as_slice() {
            [] => Some(Span {
                start: None,
                end: None,
            }),
            [term] if conjuncts.iter().any(|conjunct| ptr::eq(*conjunct, **term)) => {
                DateValue::parse(&term.value).and_then(|value| value.span(clock.now, clock.zone))
            }
            _ => None,
        };

        let temporal = range.map(|range| {
            let mut since: Option<String> = None;
            let mut until: Option<String> = None;
            let mut filling = false;
            for collection in &collections {
                for source in collection.sources.values() {
                    let Some(scope) = &source.coverage else {
                        filling = true;
                        continue;
                    };
                    if let Some(floor) = scope.since
                        && range.start.is_none_or(|start| floor.0 > start)
                    {
                        let floor = floor.rfc3339();
                        since = Some(since.map_or(floor.clone(), |held| held.max(floor)));
                    }
                    if let Some(ceiling) = scope.until
                        && range.end.is_none_or(|end| ceiling.0 < end)
                    {
                        let ceiling = ceiling.rfc3339();
                        until = Some(until.map_or(ceiling.clone(), |held| held.min(ceiling)));
                    }
                }
            }
            match (since, until, filling) {
                (None, None, false) => Temporal::Full,
                (since, until, filling) => Temporal::Bounded {
                    since,
                    until,
                    filling,
                },
            }
        });

        let horizon_left = match (dates.is_empty(), range) {
            (true, _) => Some(false),
            (false, _) if !calendars => Some(false),
            (false, Some(range)) => Some(
                range.start.is_none_or(|start| start < self.horizon.0)
                    || range.end.is_none_or(|end| end > self.horizon.1),
            ),
            (false, None) => None,
        };
        let cut = match dates.is_empty() {
            true => Some(0),
            false => cut,
        };
        let zone_differs = Some(!dates.is_empty() && calendars && clock.zone != self.floating);

        let reads = Reads::of(terms);
        let mut parts = vec![
            Tri::of(
                temporal
                    .as_ref()
                    .is_some_and(|temporal| *temporal == Temporal::Full),
            ),
            horizon_left.map_or(Tri::Unknown, |left| Tri::of(!left)),
            cut.map_or(Tri::Unknown, |cut| Tri::of(cut == 0)),
        ];
        if temporal.is_none() {
            parts[0] = Tri::Unknown;
        }
        for (media_type, counts) in &kinds {
            let kind = kind_of_media(media_type);
            if reads.kinds.contains(&kind) {
                parts.push(Tri::of(
                    counts.bodiless + counts.missing + counts.unparseable == 0,
                ));
            }
            if reads.mark && kind == Kind::Mail {
                parts.push(Tri::of(counts.bodiless == 0));
            }
            if reads.content {
                parts.push(Tri::of(counts.encrypted == 0));
            }
        }
        for kind in &unsure {
            if reads.kinds.contains(kind) || reads.content {
                parts.push(Tri::Unknown);
            }
        }
        let complete = parts.into_iter().fold(Tri::Yes, Tri::and).known();

        Coverage {
            kinds: unsure.is_empty().then_some(kinds),
            temporal,
            horizon_left,
            cut,
            zone_differs,
            complete,
        }
    }
}

/// An item live in one account, a candidate hit.
struct Candidate<'c> {
    hit: Hit,
    placements: Vec<&'c Item>,
}

/// One query being answered.
struct Asking<'c> {
    corpus: &'c Corpus,
    clock: &'c Clock,
    candidates: Vec<Candidate<'c>>,
    /// Whether the term being answered reads a summary, which an opaque
    /// placement cannot answer.
    blind: Cell<bool>,
    /// Whether a `kind:contact` or `kind:file` term stands anywhere.
    undated_named: bool,
}

impl Asking<'_> {
    fn eval(&self, node: &Node) -> Vec<Tri> {
        match node {
            Node::Or(nodes) => self.fold(nodes, Tri::No, Tri::or),
            Node::And(nodes) => self.fold(nodes, Tri::Yes, Tri::and),
            Node::Not(node) => self.eval(node).into_iter().map(Tri::not).collect(),
            Node::Group(node) => self.eval(node),
            Node::Term(term) => self.term(term),
        }
    }

    fn fold(&self, nodes: &[Node], start: Tri, with: fn(Tri, Tri) -> Tri) -> Vec<Tri> {
        let mut truths = vec![start; self.candidates.len()];
        for node in nodes {
            for (truth, other) in truths.iter_mut().zip(self.eval(node)) {
                *truth = with(*truth, other);
            }
        }
        truths
    }

    fn each(&self, test: impl Fn(&Candidate) -> Tri) -> Vec<Tri> {
        self.candidates.iter().map(test).collect()
    }

    fn any_placement(&self, test: impl Fn(&Item) -> Tri) -> Vec<Tri> {
        let blind = self.blind.get();
        self.each(|candidate| {
            Tri::any(candidate.placements.iter().map(|item| {
                match blind && self.corpus.is_opaque(item) {
                    true => Tri::Unknown,
                    false => test(item),
                }
            }))
        })
    }

    fn term(&self, term: &Term) -> Vec<Tri> {
        let corpus = self.corpus;
        let value = term.value.as_str();
        let field = term.field.as_deref();
        let sighted = matches!(
            field,
            Some("tag" | "flag" | "account" | "collection" | "folder" | "kind")
        ) || (field == Some("is") && value != "encrypted")
            || (field == Some("has") && value == "body");
        self.blind.set(!sighted);

        match field {
            None => self.text(term, &FREE),
            Some("subject" | "title" | "summary") => self.text(term, &["title"]),
            Some("body") => self.text(term, &["body"]),
            Some("attachment" | "mimetype") => self.text(term, &["attachment"]),
            Some("location") => self.text(term, &["place"]),
            Some(role @ ("from" | "to" | "cc" | "bcc" | "attendee" | "organizer" | "email")) => {
                match value.contains('@') {
                    true => self.address(term, Some(role)),
                    false => self.text(term, &[role]),
                }
            }
            Some("with") => match value.contains('@') {
                true => self.address(term, None),
                false => self.text(term, &["people"]),
            },
            Some("person") => self.person(term),
            Some("tag" | "flag") => self.any_placement(|item| Tri::of(flags(item).contains(value))),
            Some("is") => match value {
                "unread" => self
                    .any_placement(|item| Tri::of(flags(item).contains("\\Seen")))
                    .into_iter()
                    .map(Tri::not)
                    .collect(),
                "read" => self.any_placement(|item| Tri::of(flags(item).contains("\\Seen"))),
                "flagged" => self.any_placement(|item| Tri::of(flags(item).contains("\\Flagged"))),
                "answered" => {
                    self.any_placement(|item| Tri::of(flags(item).contains("\\Answered")))
                }
                "draft" => self.any_placement(|item| Tri::of(flags(item).contains("\\Draft"))),
                _ => self.any_placement(|item| match item.summary.table {
                    Table::Mail if corpus.bodied(item) => match corpus
                        .model
                        .words
                        .contains_key(item.object.as_ref().expect("a body"))
                    {
                        true => Tri::No,
                        false => Tri::Unknown,
                    },
                    _ => Tri::No,
                }),
            },
            Some("has") => self.has(value),
            Some("kind") => self.any_placement(|item| {
                let kind = corpus.model.collections[&item.collection].kind;
                match corpus.is_opaque(item) {
                    true if kind == Some(Kind::Calendar) => match value {
                        "event" | "task" | "journal" => Tri::Unknown,
                        _ => Tri::No,
                    },
                    true => Tri::of(
                        kind.is_some_and(|kind| kind == kind_named(value))
                            && !matches!(value, "event" | "task" | "journal"),
                    ),
                    false => Tri::of(kind_value(item.summary.table) == value),
                }
            }),
            Some("status") => {
                self.any_placement(
                    |item| match (item.summary.table, item.summary.get("status")) {
                        (Table::Task, Value::Text(status)) => {
                            Tri::of(status.to_ascii_lowercase() == value)
                        }
                        _ => Tri::No,
                    },
                )
            }
            Some("priority") => self.any_placement(|item| match item.summary.table {
                Table::Task => {
                    let priority = match item.summary.get("priority") {
                        Value::Integer(priority) => Some(priority),
                        _ => None,
                    };
                    Tri::of(match value {
                        "high" => priority.is_some_and(|p| (1..=4).contains(&p)),
                        "medium" => priority == Some(5),
                        "low" => priority.is_some_and(|p| (6..=9).contains(&p)),
                        _ => priority.is_none_or(|p| p == 0),
                    })
                }
                _ => Tri::No,
            }),
            Some("category") => {
                let wanted = fold_name(value);
                self.any_placement(|item| match item.summary.table {
                    Table::Task => Tri::of(
                        item.summary
                            .categories
                            .iter()
                            .any(|category| fold_name(category) == wanted),
                    ),
                    Table::Contact if corpus.bodied(item) => match corpus
                        .model
                        .words
                        .get(item.object.as_ref().expect("a body"))
                    {
                        Some(words) => Tri::of(
                            words
                                .categories
                                .iter()
                                .any(|category| fold_name(category) == wanted),
                        ),
                        None => Tri::Unknown,
                    },
                    _ => Tri::No,
                })
            }
            Some("label") => {
                let wanted = fold_name(value);
                self.each(|candidate| {
                    Tri::of(corpus.properties.iter().any(|property| {
                        property.seq == candidate.hit.seq
                            && property.key == "pimdir/label"
                            && fold_name(&property.value) == wanted
                    }))
                })
            }
            Some("property") => {
                let (key, wanted) = value.split_once('=').expect("a checked property value");
                self.each(|candidate| {
                    Tri::of(corpus.properties.iter().any(|property| {
                        property.seq == candidate.hit.seq
                            && property.key == key
                            && property.value == wanted
                    }))
                })
            }
            Some("account") => {
                self.each(|candidate| Tri::of(candidate.hit.account.as_deref() == Some(value)))
            }
            Some("collection" | "folder") => {
                self.any_placement(|item| Tri::of(item.collection == value))
            }
            Some("date" | "when") => self.date(value),
            Some("thread") => self.thread(value),
            Some("id") => self.each(|candidate| Tri::of(names(candidate, value))),
            Some("mid") => {
                let wanted = value
                    .strip_prefix('<')
                    .and_then(|inner| inner.strip_suffix('>'))
                    .unwrap_or(value);
                self.any_placement(|item| {
                    match (item.summary.table, item.summary.get("message_id")) {
                        (Table::Mail, Value::Text(id)) => Tri::of(id == wanted),
                        _ => Tri::No,
                    }
                })
            }
            Some("linked") => self.linked(value),
            Some("changed") => {
                let (from, to) = stamps(value).expect("a checked stamp value");
                let from = from.unwrap_or(0);
                let to = to.unwrap_or(i64::MAX);
                let truth = match (from <= 1 && to >= STAMPS_MAX, to < 1 || from >= STAMPS_MAX) {
                    (true, _) => Tri::Yes,
                    (_, true) => Tri::No,
                    _ => Tri::Unknown,
                };
                vec![truth; self.candidates.len()]
            }
            Some(_) => vec![Tri::Unknown; self.candidates.len()],
        }
    }

    /// A text term over `fields` (SEARCH §8.5): a hit holds it when any of
    /// its placements' documents does.
    fn text(&self, term: &Term, fields: &[&str]) -> Vec<Tri> {
        let Some(words) = tokens(&term.value).map(|tokens| Words {
            tokens,
            prefix: term.wildcard,
        }) else {
            return vec![Tri::Unknown; self.candidates.len()];
        };
        if words.tokens.is_empty() {
            return vec![Tri::No; self.candidates.len()];
        }
        self.any_placement(|item| self.document(item, fields, &words))
    }

    /// What each hit's text terms matched on (SEARCH §8.2): a body the
    /// index read when one of its placements holding one matches a text
    /// term of the query, else a summary when a placement whose body it
    /// did not read does. Decided for no hit of a query negating a text
    /// term or holding `person:`, whose match lies on other items.
    fn matched(&self, node: &Node, truths: &[Tri]) -> BTreeMap<Hit, Option<Matched>> {
        let mut texts = Vec::new();
        if !positive_texts(node, false, &mut texts) {
            return BTreeMap::new();
        }
        let texts: Vec<(Vec<&str>, Words)> = texts
            .into_iter()
            .filter_map(|(term, fields)| {
                let tokens = tokens(&term.value)?;
                let words = Words {
                    tokens,
                    prefix: term.wildcard,
                };
                Some((fields, words))
            })
            .collect();

        let mut matched = BTreeMap::new();
        for (candidate, truth) in self.candidates.iter().zip(truths) {
            if *truth != Tri::Yes {
                continue;
            }
            let mut body = Tri::No;
            let mut summary = Tri::No;
            for (fields, words) in &texts {
                for item in &candidate.placements {
                    let holds = self.document(item, fields, words);
                    match self.corpus.bodied(item) {
                        true => body = body.or(holds),
                        false => summary = summary.or(holds),
                    }
                }
            }
            let on = match (body, summary) {
                (Tri::Yes, _) => Some(Some(Matched::Body)),
                (Tri::No, Tri::Yes) => Some(Some(Matched::Summary)),
                (Tri::No, Tri::No) => Some(None),
                _ => None,
            };
            if let Some(on) = on {
                matched.insert(candidate.hit.clone(), on);
            }
        }
        matched
    }

    fn document(&self, item: &Item, fields: &[&str], words: &Words) -> Tri {
        Tri::any(
            fields
                .iter()
                .map(|field| match self.corpus.field(item, field) {
                    None => Tri::Unknown,
                    Some(readings) => {
                        Tri::agree(readings.iter().map(|values| words.in_values(values)))
                    }
                }),
        )
    }

    /// An address term (SEARCH §8): the items whose summary states the
    /// address in the role, any role for `with:`, a bare `@domain` any
    /// address of the domain.
    fn address(&self, term: &Term, role: Option<&str>) -> Vec<Tri> {
        let value = term.value.as_str();
        if term.wildcard || value.to_lowercase() != value {
            return vec![Tri::Unknown; self.candidates.len()];
        }
        self.any_placement(|item| {
            Tri::of(item.addresses.iter().any(|address| {
                role.is_none_or(|role| address.role == role)
                    && match value.strip_prefix('@') {
                        Some(domain) => address
                            .address
                            .rsplit_once('@')
                            .is_some_and(|(_, held)| held == domain),
                        None => address.address == value,
                    }
            }))
        })
    }

    /// `person:` (SEARCH §8): the contacts whose title matches, expanded to
    /// `with:` over the addresses their cards state.
    fn person(&self, term: &Term) -> Vec<Tri> {
        let Some(words) = tokens(&term.value).map(|tokens| Words {
            tokens,
            prefix: term.wildcard,
        }) else {
            return vec![Tri::Unknown; self.candidates.len()];
        };
        if words.tokens.is_empty() {
            return vec![Tri::No; self.candidates.len()];
        }

        let mut sure = BTreeSet::new();
        let mut maybe = BTreeSet::new();
        for candidate in &self.candidates {
            for item in &candidate.placements {
                if item.summary.table != Table::Contact {
                    continue;
                }
                let emails = item
                    .addresses
                    .iter()
                    .filter(|address| address.role == "email")
                    .map(|address| address.address.clone());
                match self.document(item, &["title"], &words) {
                    Tri::Yes => sure.extend(emails),
                    Tri::Unknown => maybe.extend(emails),
                    Tri::No => {}
                }
            }
        }
        self.any_placement(|item| {
            let held = |set: &BTreeSet<String>| {
                item.addresses
                    .iter()
                    .any(|address| set.contains(&address.address))
            };
            match (held(&sure), held(&maybe)) {
                (true, _) => Tri::Yes,
                (false, true) => Tri::Unknown,
                (false, false) => Tri::No,
            }
        })
    }

    fn has(&self, value: &str) -> Vec<Tri> {
        let corpus = self.corpus;
        let words = |item: &Item| {
            item.object
                .as_ref()
                .and_then(|object| corpus.model.words.get(object))
        };
        let mark = match value {
            "phone" => Some(0),
            "address" => Some(1),
            "birthday" => Some(2),
            "photo" => Some(3),
            _ => None,
        };
        self.any_placement(|item| match (value, item.summary.table) {
            ("body", _) => Tri::of(item.object.is_some()),
            ("attachment", Table::Mail) => {
                Tri::of(item.summary.get("attachment") == Value::Integer(1))
            }
            ("attachment", Table::Event | Table::Task | Table::Journal) if corpus.bodied(item) => {
                match words(item) {
                    Some(words) => Tri::of(
                        words
                            .fields
                            .get("attachment")
                            .is_some_and(|values| !values.is_empty()),
                    ),
                    None => Tri::Unknown,
                }
            }
            (_, Table::Contact) if mark.is_some() && corpus.bodied(item) => match words(item) {
                Some(words) => Tri::of(words.marks[mark.expect("a mark")]),
                None => Tri::Unknown,
            },
            _ => Tri::No,
        })
    }

    fn date(&self, value: &str) -> Vec<Tri> {
        let corpus = self.corpus;
        let Some(span) =
            DateValue::parse(value).and_then(|value| value.span(self.clock.now, self.clock.zone))
        else {
            return vec![Tri::Unknown; self.candidates.len()];
        };
        if span
            .start
            .zip(span.end)
            .is_some_and(|(start, end)| start >= end)
        {
            return vec![Tri::No; self.candidates.len()];
        }
        self.any_placement(|item| match item.summary.table {
            Table::Mail => match item.sort_key.as_str() {
                "" => Tri::No,
                key => match rfc3339(key) {
                    Some(at) => Tri::of(span.meets(at, at)),
                    None => Tri::Unknown,
                },
            },
            Table::Event | Table::Task | Table::Journal => match corpus.occurrences(item) {
                Some(readings) => Tri::agree(readings.iter().map(|occurrences| {
                    Tri::of(
                        occurrences
                            .iter()
                            .any(|(start, end)| span.meets(*start, *end)),
                    )
                })),
                None => Tri::Unknown,
            },
            Table::Contact | Table::File => match self.undated_named {
                true => Tri::Unknown,
                false => Tri::No,
            },
        })
    }

    /// `thread:` (SEARCH §9): the value is the link id of any node, member
    /// or phantom, naming its thread, a connected set over the nodes of the
    /// whole store, whose present members it answers in every account
    /// holding one. A member bears its link id, or its link id and its
    /// `Message-ID`, the two readings §9 leaves open.
    fn thread(&self, id: &str) -> Vec<Tri> {
        let corpus = self.corpus;
        let members: Vec<&Item> = self
            .candidates
            .iter()
            .flat_map(|candidate| candidate.placements.iter().copied())
            .filter(|item| item.summary.table == Table::Mail)
            .collect();

        let parent = |item: &Item| -> Option<Option<String>> {
            if corpus.is_opaque(item) {
                return None;
            }
            if let Some(first) = item.summary.in_reply_to.first() {
                return Some(Some(first.clone()));
            }
            match corpus.bodied(item) {
                true => corpus
                    .model
                    .words
                    .contains_key(item.object.as_ref()?)
                    .then_some(None),
                false => Some(None),
            }
        };

        let mut answers: BTreeMap<String, Vec<Tri>> = BTreeMap::new();
        for both in [false, true] {
            let mut sets = Sets::default();
            let mut open = BTreeSet::new();
            for item in &members {
                let mut ids = vec![item.link_id.clone()];
                if both && let Value::Text(message_id) = item.summary.get("message_id") {
                    ids.push(message_id);
                }
                for other in &ids[1..] {
                    sets.join(&ids[0], other);
                }
                sets.add(&ids[0]);
                match parent(item) {
                    Some(Some(parent)) => sets.join(&ids[0], &parent),
                    Some(None) => {}
                    None => {
                        open.insert(item.link_id.clone());
                    }
                }
            }

            let named = sets.holds(id).then(|| sets.find(id));
            let open_roots: BTreeSet<String> = open.iter().map(|link| sets.find(link)).collect();
            for item in &members {
                let root = sets.find(&item.link_id);
                let truth = match &named {
                    Some(named) if *named == root => Tri::Yes,
                    Some(named) if open_roots.contains(named) || open_roots.contains(&root) => {
                        Tri::Unknown
                    }
                    Some(_) => Tri::No,
                    None if open.is_empty() => Tri::No,
                    None => Tri::Unknown,
                };
                answers.entry(item.link_id.clone()).or_default().push(truth);
            }
        }

        self.each(|candidate| {
            Tri::any(
                candidate
                    .placements
                    .iter()
                    .map(|item| match answers.get(&item.link_id) {
                        Some(truths) => Tri::agree(truths.iter().copied()),
                        None => Tri::No,
                    }),
            )
        })
    }

    /// `linked:` (SEARCH §8): the items at the other end of the named
    /// item's references, either direction, read from the store.
    fn linked(&self, value: &str) -> Vec<Tri> {
        let model = &self.corpus.model;
        let (role, id) = match value.split_once(':') {
            Some((role, id)) if reference_role(role) => (Some(role), id),
            _ => (None, value),
        };

        let endpoint = |item: &Item| {
            model.collections[&item.collection]
                .kind
                .map(|kind| (kind, item.link_id.clone()))
        };
        let named: BTreeSet<(Kind, String)> = self
            .candidates
            .iter()
            .filter(|candidate| names(candidate, id))
            .flat_map(|candidate| {
                candidate
                    .placements
                    .iter()
                    .filter_map(|item| endpoint(item))
            })
            .collect();

        let mut ends = BTreeSet::new();
        for reference in &model.references {
            if role.is_some_and(|role| reference.role != role) {
                continue;
            }
            let from = (reference.from_kind, reference.from_link_id.clone());
            let to = (reference.to_kind, reference.to_link_id.clone());
            if named.contains(&from) {
                ends.insert(to.clone());
            }
            if named.contains(&to) {
                ends.insert(from);
            }
        }
        self.any_placement(|item| Tri::of(endpoint(item).is_some_and(|end| ends.contains(&end))))
    }
}

/// What a query's terms read (SEARCH §8.2).
struct Reads {
    kinds: BTreeSet<Kind>,
    content: bool,
    mark: bool,
}

impl Reads {
    fn of(terms: &[&Term]) -> Self {
        let mut reads = Self {
            kinds: BTreeSet::new(),
            content: false,
            mark: false,
        };
        for term in terms {
            let field = term.field.as_deref();
            let address = matches!(field, Some(field) if ROLES.contains(&field) || field == "with")
                && term.value.contains('@');
            let text = field.is_none_or(|field| TEXT_FIELDS.contains(&field));
            if text && !address {
                reads
                    .kinds
                    .extend([Kind::Mail, Kind::Contact, Kind::Calendar]);
                reads.content |= matches!(field, None | Some("body" | "attachment" | "mimetype"));
            }
            match (field, term.value.as_str()) {
                (Some("date" | "when"), _) => {
                    reads.kinds.insert(Kind::Calendar);
                }
                (Some("has"), "attachment") => {
                    reads.kinds.insert(Kind::Calendar);
                    reads.mark = true;
                }
                (Some("thread"), _) | (Some("is"), "encrypted") => {
                    reads.kinds.insert(Kind::Mail);
                }
                (Some("has"), "phone" | "address" | "birthday" | "photo")
                | (Some("category"), _) => {
                    reads.kinds.insert(Kind::Contact);
                }
                _ => {}
            }
        }
        reads
    }
}

/// Disjoint sets over node ids, each named by its first id.
#[derive(Default)]
struct Sets {
    parents: BTreeMap<String, String>,
}

impl Sets {
    fn add(&mut self, id: &str) {
        self.parents
            .entry(String::from(id))
            .or_insert_with(|| String::from(id));
    }

    fn holds(&self, id: &str) -> bool {
        self.parents.contains_key(id)
    }

    fn find(&self, id: &str) -> String {
        let mut at = String::from(id);
        while let Some(parent) = self.parents.get(&at) {
            if *parent == at {
                break;
            }
            at = parent.clone();
        }
        at
    }

    fn join(&mut self, left: &str, right: &str) {
        self.add(left);
        self.add(right);
        let (left, right) = (self.find(left), self.find(right));
        if left != right {
            let (low, high) = match left < right {
                true => (left, right),
                false => (right, left),
            };
            self.parents.insert(high, low);
        }
    }
}

fn every_term<'n>(node: &'n Node, terms: &mut Vec<&'n Term>) {
    match node {
        Node::Or(nodes) | Node::And(nodes) => nodes.iter().for_each(|node| every_term(node, terms)),
        Node::Not(node) | Node::Group(node) => every_term(node, terms),
        Node::Term(term) => terms.push(term),
    }
}

/// Gathers the text terms of `node` (SEARCH §8.5) with the fields each
/// matches in, false when one stands under a negation or is `person:`.
fn positive_texts<'n>(
    node: &'n Node,
    negated: bool,
    texts: &mut Vec<(&'n Term, Vec<&'static str>)>,
) -> bool {
    match node {
        Node::Or(nodes) | Node::And(nodes) => nodes
            .iter()
            .all(|node| positive_texts(node, negated, texts)),
        Node::Group(node) => positive_texts(node, negated, texts),
        Node::Not(node) => positive_texts(node, true, texts),
        Node::Term(term) => {
            let field = term.field.as_deref();
            let address = field.is_some_and(|field| ROLES.contains(&field) || field == "with")
                && term.value.contains('@');
            let fields = match field {
                _ if address => return true,
                None => FREE.to_vec(),
                Some("person") => return false,
                Some("subject" | "title" | "summary") => vec!["title"],
                Some("body") => vec!["body"],
                Some("attachment" | "mimetype") => vec!["attachment"],
                Some("location") => vec!["place"],
                Some("with") => vec!["people"],
                Some(field) => match ROLES.iter().find(|role| **role == field) {
                    Some(role) => vec![*role],
                    None => return true,
                },
            };
            if negated {
                return false;
            }
            texts.push((term, fields));
            true
        }
    }
}

/// The conjuncts of the whole query (SEARCH §8.2): a group that is itself
/// one lends its own, at any depth.
fn conjuncts_of<'n>(node: &'n Node, terms: &mut Vec<&'n Term>) {
    match node {
        Node::And(nodes) => nodes.iter().for_each(|node| conjuncts_of(node, terms)),
        Node::Group(node) => conjuncts_of(node, terms),
        Node::Term(term) => terms.push(term),
        Node::Or(_) | Node::Not(_) => {}
    }
}

/// Whether `id:` names the candidate (SEARCH §8): its `seq` for a value of
/// ASCII digits, and any placement's link id.
fn names(candidate: &Candidate, id: &str) -> bool {
    let seq = !id.is_empty()
        && id.bytes().all(|byte| byte.is_ascii_digit())
        && id.parse() == Ok(candidate.hit.seq);
    seq || candidate.placements.iter().any(|item| item.link_id == id)
}

fn flags(item: &Item) -> BTreeSet<String> {
    item.flags.clone().unwrap_or_default()
}

/// An item's title (SEARCH §8.2): a message's subject, a card's `fn`, a
/// calendar component's summary, a file's name.
fn title(item: &Item) -> Option<String> {
    let column = match item.summary.table {
        Table::Mail => "subject",
        Table::Contact => "fn",
        Table::Event | Table::Task | Table::Journal => "summary",
        Table::File => "name",
    };
    match item.summary.get(column) {
        Value::Text(text) => Some(text),
        _ => None,
    }
}

fn kind_value(table: Table) -> &'static str {
    match table {
        Table::Mail => "mail",
        Table::Contact => "contact",
        Table::Event => "event",
        Table::Task => "task",
        Table::Journal => "journal",
        Table::File => "file",
    }
}

fn kind_named(value: &str) -> Kind {
    match value {
        "mail" => Kind::Mail,
        "contact" => Kind::Contact,
        "file" => Kind::File,
        _ => Kind::Calendar,
    }
}

fn kind_of_media(media_type: &str) -> Kind {
    match media_type {
        "message/rfc822" => Kind::Mail,
        "text/vcard" => Kind::Contact,
        "text/calendar" => Kind::Calendar,
        _ => Kind::File,
    }
}

/// An RFC 3339 instant in UTC as a sort key states it.
fn rfc3339(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    let shaped = bytes.len() == 20
        && bytes.iter().enumerate().all(|(at, byte)| match at {
            4 | 7 => *byte == b'-',
            10 => *byte == b'T',
            13 | 16 => *byte == b':',
            19 => *byte == b'Z',
            _ => byte.is_ascii_digit(),
        });
    shaped.then(|| Instant::from_rfc3339(text).0)
}
