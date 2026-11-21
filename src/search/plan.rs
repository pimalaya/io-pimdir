//! # Plan
//!
//! How a query is answered (SEARCH §8.2), decided I/O-free so every
//! shape renders one plan: the query's scope, its terms as leaves, the
//! sets a positive conjunct reads that may drive it, and the driver a
//! count or a page takes given what the client measured of those sets.
//!
//! A count reads its smallest set and probes the other terms per hit, or
//! every live item of the scope when no positive term has a set. A page
//! in a dated order reads a small set whole, joined to the store, sorted
//! and cut by the cursor; a broad one walks the scope's collections in
//! the store's order (STORAGE §9.3) with [`PimdirWalk`], each row probed,
//! until the page is full, and falls back to the set once the walk has
//! examined as many rows as reading the set would have cost. The `title`,
//! `sender` and `relevance` orders read their whole driving set.
//!
//! A hit stands by the greatest sort key of its live placements in the
//! query's collections, in both dated orders, so a walk over those
//! collections places it where a set read whole does.

use alloc::{boxed::Box, format, string::String, vec, vec::Vec};
use core::cmp::Ordering;

use jiff::tz::TimeZone;

use crate::{
    coroutine::PimdirCoroutineState,
    search::query::{PimdirQuery, PimdirQueryKind, PimdirQueryPredicate},
    summary::file,
};

/// The most rows an estimate counts (`LIMIT CAP`): a set past it is read
/// as large, whatever its size.
pub const CAP: u64 = 100_001;

/// How many times a page's hits a set may hold and still be read whole
/// rather than walked.
const SET_PAGES: u64 = 20;

/// How many rows a walk is expected to examine per hit of its page, what
/// a probed text term is weighed against.
const WALK_ROWS: u64 = 20;

/// The most rows one walk step examines.
const STEP_MAX: usize = 4_096;

/// The order of a hit list (SEARCH §8.2).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PimdirSearchSort {
    /// `newest-first`: `sort_key` descending, the undated last.
    #[default]
    NewestFirst,
    /// `oldest-first`: `sort_key` ascending, the undated last.
    OldestFirst,
    /// `title`: the title folded, A to Z, a title-less item first.
    Title,
    /// `sender`: a message's sender folded, A to Z, the rest last by date.
    Sender,
    /// `relevance`: FTS5 `bm25`, best first, the unranked last by date.
    Relevance,
}

impl PimdirSearchSort {
    /// Every order, in SEARCH §8.2's.
    pub const ALL: [Self; 5] = [
        Self::NewestFirst,
        Self::OldestFirst,
        Self::Title,
        Self::Sender,
        Self::Relevance,
    ];

    /// The order's name in SEARCH §8.2.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NewestFirst => "newest-first",
            Self::OldestFirst => "oldest-first",
            Self::Title => "title",
            Self::Sender => "sender",
            Self::Relevance => "relevance",
        }
    }

    /// The inverse of [`as_str`](Self::as_str).
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|sort| sort.as_str() == name)
    }

    /// Whether the order is the store's, by sort key, which a walk reads
    /// page by page: `newest-first` and `oldest-first`.
    pub fn is_dated(&self) -> bool {
        matches!(self, Self::NewestFirst | Self::OldestFirst)
    }
}

/// The place of the last hit a page answered in its query's order, the
/// cursor the next page starts after (SEARCH §8.2): a hit arriving or
/// leaving between two pages neither repeats nor hides another, every
/// page walked to the end answering the hits one whole answer does as
/// the index stood when each was read.
///
/// Opaque: [`token`](Self::token) writes it as printable ASCII and
/// [`from_token`](Self::from_token) reads it back.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirSearchCursor {
    /// Whether the hit is undated, the dated orders' first key.
    pub(crate) undated: bool,
    /// Its sort key, their second.
    pub(crate) sort_key: Option<String>,
    /// The bits of its rank, the `relevance` order's first, `None` last.
    pub(crate) rank: Option<u64>,
    /// Its title or its sender folded, the `title` or `sender` order's
    /// first.
    pub(crate) title: String,
    /// Its public id, every order's tie-break.
    pub(crate) seq: i64,
    /// Its account, empty for none, the last tie-break.
    pub(crate) account: String,
}

impl PimdirSearchCursor {
    /// The cursor as a token: its six fields, each hex or decimal, after
    /// a `1` naming the form.
    pub fn token(&self) -> String {
        let hex =
            |bytes: &[u8]| -> String { bytes.iter().map(|byte| format!("{byte:02x}")).collect() };
        format!(
            "1.{}.{}.{}.{}.{}.{}",
            u8::from(self.undated),
            self.sort_key
                .as_deref()
                .map_or(String::from("-"), |key| hex(key.as_bytes())),
            self.rank
                .map_or(String::from("-"), |rank| format!("{rank:016x}")),
            hex(self.title.as_bytes()),
            self.seq,
            hex(self.account.as_bytes()),
        )
    }

    /// Reads a token [`token`](Self::token) wrote, `None` for any other
    /// text.
    pub fn from_token(token: &str) -> Option<Self> {
        let unhex = |text: &str| -> Option<String> {
            if !text.len().is_multiple_of(2) {
                return None;
            }
            let bytes = (0..text.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
                .collect::<Option<Vec<u8>>>()?;
            String::from_utf8(bytes).ok()
        };
        let mut fields = token.split('.');
        if fields.next()? != "1" {
            return None;
        }
        let undated = match fields.next()? {
            "0" => false,
            "1" => true,
            _ => return None,
        };
        let sort_key = match fields.next()? {
            "-" => None,
            key => Some(unhex(key)?),
        };
        let rank = match fields.next()? {
            "-" => None,
            rank if rank.len() == 16 => Some(u64::from_str_radix(rank, 16).ok()?),
            _ => return None,
        };
        let title = unhex(fields.next()?)?;
        let seq = fields.next()?.parse().ok()?;
        let account = unhex(fields.next()?)?;
        if fields.next().is_some() {
            return None;
        }
        Some(Self {
            undated,
            sort_key,
            rank,
            title,
            seq,
            account,
        })
    }

    /// The order of two places under `sort`, the one every driver and the
    /// title fold give a whole answer.
    pub fn compare(&self, other: &Self, sort: PimdirSearchSort) -> Ordering {
        let ties = || {
            self.seq
                .cmp(&other.seq)
                .then_with(|| self.account.cmp(&other.account))
        };
        let dated = |newest: bool| {
            self.undated
                .cmp(&other.undated)
                .then_with(|| match (self.undated, newest) {
                    (false, true) => other.sort_key.cmp(&self.sort_key),
                    _ => self.sort_key.cmp(&other.sort_key),
                })
                .then_with(ties)
        };
        match sort {
            PimdirSearchSort::NewestFirst => dated(true),
            PimdirSearchSort::OldestFirst => dated(false),
            PimdirSearchSort::Sender => match (self.title.is_empty(), other.title.is_empty()) {
                (false, false) => self.title.cmp(&other.title).then_with(ties),
                (true, true) => dated(true),
                (left, right) => left.cmp(&right),
            },
            PimdirSearchSort::Relevance => {
                let rank = |rank: Option<u64>| rank.map(f64::from_bits);
                match (rank(self.rank), rank(other.rank)) {
                    (Some(left), Some(right)) => left.total_cmp(&right),
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => Ordering::Equal,
                }
                .then_with(ties)
            }
            PimdirSearchSort::Title => self.title.cmp(&other.title).then_with(ties),
        }
    }

    /// The place of a hit in a dated order, as a walk or a set read whole
    /// answers it.
    pub fn dated(undated: bool, sort_key: String, seq: i64, account: String) -> Self {
        Self {
            undated,
            sort_key: Some(sort_key),
            rank: None,
            title: String::new(),
            seq,
            account,
        }
    }

    /// Whether the place is of an undated hit.
    pub fn is_undated(&self) -> bool {
        self.undated
    }

    /// The sort key of the place, `None` for a hit of none.
    pub fn sort_key(&self) -> Option<&str> {
        self.sort_key.as_deref()
    }

    /// The public id of the place's hit.
    pub fn seq(&self) -> i64 {
        self.seq
    }

    /// The account of the place's hit, empty for none.
    pub fn account(&self) -> &str {
        &self.account
    }
}

/// The query's collections (SEARCH §8.2): every collection of the store
/// less those an `account:`, `collection:`, `folder:` or `kind:` term
/// standing as a conjunct rules out. Each axis is `None` when no term
/// narrows it, and an empty list when its terms leave nothing.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirPlanScope {
    /// The media types a collection of the scope has.
    pub kinds: Option<Vec<&'static str>>,
    /// The accounts a collection of the scope belongs to.
    pub accounts: Option<Vec<String>>,
    /// The collection ids of the scope.
    pub collections: Option<Vec<String>>,
}

impl PimdirPlanScope {
    /// The scope of a query, read from its conjuncts
    /// ([`PimdirQuery::conjuncts`]).
    ///
    /// The first `collection:` conjunct narrows it, never a later one: an
    /// item filed in two collections is a hit of a query naming both
    /// (`folder:a folder:b`), so the scope holds the first and the others
    /// are probed. Two accounts or two kinds hold no hit together, a hit
    /// being one account's and a seq never spanning kinds.
    pub fn of(query: &PimdirQuery) -> Self {
        let mut scope = Self::default();
        for term in query.conjuncts() {
            match &term.predicate {
                PimdirQueryPredicate::Kind(kind) => {
                    narrow(&mut scope.kinds, vec![media_type(*kind)]);
                }
                PimdirQueryPredicate::Account(account) => {
                    narrow(&mut scope.accounts, vec![account.clone()]);
                }
                PimdirQueryPredicate::Collection(id) if scope.collections.is_none() => {
                    scope.collections = Some(vec![id.clone()]);
                }
                _ => (),
            }
        }
        scope
    }

    /// Whether a collection of `kind`, `account` and `id` is of the
    /// scope.
    pub fn admits(&self, kind: &str, account: Option<&str>, id: &str) -> bool {
        self.kinds
            .as_ref()
            .is_none_or(|kinds| kinds.contains(&kind))
            && self
                .accounts
                .as_ref()
                .is_none_or(|accounts| account.is_some_and(|a| accounts.iter().any(|b| b == a)))
            && self
                .collections
                .as_ref()
                .is_none_or(|ids| ids.iter().any(|b| b == id))
    }

    /// Whether the scope names one collection alone, where a hit has one
    /// placement at most.
    pub fn is_one_collection(&self) -> bool {
        self.collections.as_ref().is_some_and(|ids| ids.len() == 1)
    }
}

/// A query's terms as a tree over the plan's leaves.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirPlanNode {
    /// Every operand holds.
    And(Vec<PimdirPlanNode>),
    /// Any operand holds.
    Or(Vec<PimdirPlanNode>),
    /// The operand does not hold.
    Not(Box<PimdirPlanNode>),
    /// One term, by its position in [`PimdirPlan::leaves`].
    Leaf(usize),
}

/// One term of a plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirPlanLeaf {
    /// What the term asks.
    pub predicate: PimdirQueryPredicate,
    /// Whether an odd number of negations applies to it.
    pub negated: bool,
    /// What a client measures of its set, when it has one a driver reads.
    pub estimate: PimdirPlanEstimate,
    /// The instants a date term spans, `[since, until)` in Unix seconds,
    /// either side `None` open (SEARCH §8.1); `None` for another term.
    pub dates: Option<(Option<i64>, Option<i64>)>,
}

impl PimdirPlanLeaf {
    /// Whether the term is matched on the full-text index, read into a
    /// set or probed per hit (SEARCH §8.5): free text, a text field, and
    /// a role field or `with:` whose value holds no `@`.
    pub fn is_text(&self) -> bool {
        match &self.predicate {
            PimdirQueryPredicate::Text { .. } => true,
            PimdirQueryPredicate::Address { text, .. } => !text.text.contains('@'),
            _ => false,
        }
    }
}

/// What a client measures of a leaf's set before a driver is chosen,
/// each count capped at [`CAP`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirPlanEstimate {
    /// The full-text matches of its expression, over bodies and
    /// summaries.
    Text,
    /// The `item_address` rows of one address.
    Address,
    /// The `flag` rows of one flag.
    Flag,
    /// The live items of one collection.
    Collection,
    /// The items stamped within the range.
    Changed,
    /// The dated items and occurrences within the range.
    Date,
    /// A set a few seeks read: `id:`, `thread:`, `linked:`,
    /// `is:encrypted`.
    Seek,
    /// The items holding one value under a key, or one of the spellings
    /// a label folds to: `label:`, `property:`, a seek on
    /// `item_property_by_value` per value.
    Property,
    /// None: the term is probed per hit, never read whole.
    None,
}

/// What a client measured of a set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PimdirPlanBound {
    /// The rows counted.
    pub rows: u64,
    /// Whether the count stopped at [`CAP`], the set being larger.
    pub capped: bool,
}

/// What one row costs each way, in nanoseconds: what weighs a set read
/// whole against a walk, and a text term read into a set against probed.
/// The defaults are FTS5's on SQLite 3.51; an FTS4 shape states its own
/// text probe (86 µs).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PimdirPlanCosts {
    /// A hit of a set read whole, joined to the store and sorted.
    pub set_row: u64,
    /// A full-text match read into a set.
    pub match_row: u64,
    /// A full-text expression probed against one placement.
    pub text_probe: u64,
    /// An index seek: a structured term probed, a placement compared.
    pub seek: u64,
    /// The head of one collection a walk reads before it starts.
    pub head: u64,
}

impl Default for PimdirPlanCosts {
    fn default() -> Self {
        Self {
            set_row: 1_000,
            match_row: 150,
            text_probe: 6_000,
            seek: 1_000,
            head: 1_000,
        }
    }
}

/// How the hits of a count or a page are found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirPlanDriver {
    /// The union of these leaves' sets, read whole, joined to the store
    /// and every other term probed per hit: one leaf, or the operands of
    /// a positive conjunct `or`.
    Set(Vec<usize>),
    /// The scope's collections merged in the store's order, every term
    /// probed per row, until the page is full; past `budget` rows
    /// examined, `fallback`'s set is read instead.
    Walk {
        /// The most rows examined, `None` for no bound.
        budget: Option<u64>,
        /// The set read past the budget.
        fallback: Option<Vec<usize>>,
    },
    /// Every live item of the scope, every term probed.
    Scope,
}

/// How a text term is tested.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirPlanAccess {
    /// Its matches read once into a set, each hit probed by a seek.
    Materialize,
    /// Its expression probed against each candidate's placements.
    Probe,
}

/// A driver forced in place of the one the plan chooses, so a test runs
/// a query under each and sees one answer.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirPlanForce {
    /// The smallest set, text terms tested as the plan chooses.
    Set,
    /// A walk with no budget for a page, the scope for a count, every
    /// text term probed per placement.
    Walk,
    /// A walk with no budget for a page, the scope for a count, every
    /// text term read into a set.
    Scope,
    /// A walk of the smallest set's page that gives up after its first
    /// step and reads the set, every text term probed until then; the
    /// smallest set for a count.
    Fallback,
}

/// A driver and how each leaf's text is tested.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirPlanChoice {
    /// The driver.
    pub driver: PimdirPlanDriver,
    /// Per leaf, how its text is tested: [`PimdirPlanAccess::Probe`] for
    /// a leaf of no text.
    pub access: Vec<PimdirPlanAccess>,
}

/// A query planned: its scope, its leaves and the sets that may drive it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirPlan {
    /// The query's collections.
    pub scope: PimdirPlanScope,
    /// The query's terms as a tree over [`leaves`](Self::leaves).
    pub root: PimdirPlanNode,
    /// The query's terms, depth first.
    pub leaves: Vec<PimdirPlanLeaf>,
    /// The sets that may drive it: a positive conjunct leaf whose set a
    /// client reads whole, or the operands of a positive conjunct `or`
    /// when every one is such a leaf.
    pub sets: Vec<Vec<usize>>,
    /// The undated media types a `kind:` term names anywhere, every item
    /// of which a date term holds (SEARCH §8.1).
    pub undated: Vec<&'static str>,
    /// The leaves the scope implies, a positive conjunct `kind:` naming a
    /// collection kind whole (mail, contacts, files), `account:` or
    /// `collection:`: a hit with a placement in the scope meets them, so
    /// a statement reading the scope's placements need not probe them.
    pub scoped: Vec<usize>,
}

impl PimdirPlan {
    /// Plans a query, its dates resolved against `now` (Unix seconds) in
    /// `zone`.
    pub fn new(query: &PimdirQuery, now: i64, zone: &TimeZone) -> Self {
        let undated: Vec<&'static str> = query
            .terms()
            .into_iter()
            .filter_map(|(term, _)| match &term.predicate {
                PimdirQueryPredicate::Kind(kind) if !kind.is_dated() => Some(media_type(*kind)),
                _ => None,
            })
            .collect();

        let mut plan = Self {
            scope: PimdirPlanScope::of(query),
            root: PimdirPlanNode::And(Vec::new()),
            leaves: Vec::new(),
            sets: Vec::new(),
            undated,
            scoped: Vec::new(),
        };
        plan.root = plan.node(query, false, now, zone);

        let mut sets = Vec::new();
        let mut scoped = Vec::new();
        let mut collection = false;
        for conjunct in conjuncts(&plan.root) {
            if let PimdirPlanNode::Leaf(at) = conjunct {
                let implied = match &plan.leaves[*at].predicate {
                    PimdirQueryPredicate::Kind(kind) => matches!(
                        kind,
                        PimdirQueryKind::Mail | PimdirQueryKind::Contact | PimdirQueryKind::File
                    ),
                    PimdirQueryPredicate::Account(_) => true,
                    PimdirQueryPredicate::Collection(_) => {
                        !core::mem::replace(&mut collection, true)
                    }
                    _ => false,
                };
                if implied {
                    scoped.push(*at);
                }
            }
            match conjunct {
                PimdirPlanNode::Leaf(at) if plan.is_set(*at) => sets.push(vec![*at]),
                PimdirPlanNode::Or(operands) => {
                    let leaves: Option<Vec<usize>> = operands
                        .iter()
                        .map(|operand| match operand {
                            PimdirPlanNode::Leaf(at) if plan.is_set(*at) => Some(*at),
                            _ => None,
                        })
                        .collect();
                    if let Some(leaves) = leaves {
                        sets.push(leaves);
                    }
                }
                _ => (),
            }
        }
        plan.sets = sets;
        plan.scoped = scoped;
        plan
    }

    /// The bound of a set, the sum of its leaves', `None` when one was
    /// not measured.
    pub fn bound(set: &[usize], bounds: &[Option<PimdirPlanBound>]) -> Option<PimdirPlanBound> {
        let mut total = PimdirPlanBound {
            rows: 0,
            capped: false,
        };
        for at in set {
            let bound = bounds.get(*at).copied().flatten()?;
            total.rows = total.rows.saturating_add(bound.rows);
            total.capped |= bound.capped;
        }
        Some(total)
    }

    /// The leaves whose bound a client measures: those of every set, and
    /// every text term, whose bound weighs reading it whole against
    /// probing it.
    pub fn measured(&self) -> Vec<usize> {
        (0..self.leaves.len())
            .filter(|at| self.leaves[*at].is_text() || self.sets.iter().any(|set| set.contains(at)))
            .collect()
    }

    /// The driver of a count: the smallest set, else the scope, which a
    /// set the scope implies (a `collection:` term) counts without
    /// reading it whole.
    pub fn count(
        &self,
        bounds: &[Option<PimdirPlanBound>],
        costs: &PimdirPlanCosts,
        force: Option<PimdirPlanForce>,
    ) -> PimdirPlanChoice {
        let driver = match (force, self.smallest(bounds)) {
            (Some(PimdirPlanForce::Walk | PimdirPlanForce::Scope), _) | (_, None) => {
                PimdirPlanDriver::Scope
            }
            (None, Some((set, _))) if set.iter().all(|at| self.scoped.contains(at)) => {
                PimdirPlanDriver::Scope
            }
            (_, Some((set, _))) => PimdirPlanDriver::Set(set),
        };
        let access = self.access(&driver, bounds, costs, force, None);
        PimdirPlanChoice { driver, access }
    }

    /// The driver of a page of `limit` hits in `sort` over a scope of
    /// `collections` collections: in a dated order, the smallest set when
    /// it holds few pages or costs less than a walk, else a walk falling
    /// back to it; in another, the smallest set or the scope, read whole.
    pub fn page(
        &self,
        sort: PimdirSearchSort,
        limit: usize,
        collections: u64,
        bounds: &[Option<PimdirPlanBound>],
        costs: &PimdirPlanCosts,
        force: Option<PimdirPlanForce>,
    ) -> PimdirPlanChoice {
        let relevance = sort == PimdirSearchSort::Relevance;
        let smallest = self.smallest(bounds);
        if !sort.is_dated() {
            let driver = match (force, smallest) {
                (Some(PimdirPlanForce::Walk | PimdirPlanForce::Scope), _) | (_, None) => {
                    PimdirPlanDriver::Scope
                }
                (_, Some((set, _))) => PimdirPlanDriver::Set(set),
            };
            let mut access = self.access(&driver, bounds, costs, force, None);
            if relevance {
                for (at, leaf) in self.leaves.iter().enumerate() {
                    if leaf.is_text() && !leaf.negated {
                        access[at] = PimdirPlanAccess::Materialize;
                    }
                }
            }
            return PimdirPlanChoice { driver, access };
        }

        let page = (limit as u64).saturating_add(1);
        let walk = PimdirPlanDriver::Walk {
            budget: None,
            fallback: None,
        };
        let driver = match (force, smallest) {
            (Some(PimdirPlanForce::Walk | PimdirPlanForce::Scope), _) | (_, None) => walk,
            (Some(PimdirPlanForce::Set), Some((set, _))) => PimdirPlanDriver::Set(set),
            (Some(PimdirPlanForce::Fallback), Some((set, _))) => PimdirPlanDriver::Walk {
                budget: Some(0),
                fallback: Some(set),
            },
            (None, Some((set, bound))) => {
                let walking = PimdirPlanDriver::Walk {
                    budget: None,
                    fallback: Some(set.clone()),
                };
                let access = self.access(&walking, bounds, costs, None, Some(page));
                let row = self.probe_row(&access, costs);
                let rows = match self.is_scoped() {
                    true => page,
                    false => page.saturating_mul(WALK_ROWS),
                };
                let walked = collections
                    .saturating_mul(costs.head)
                    .saturating_add(rows.saturating_mul(row));
                let read = bound.rows.saturating_mul(costs.set_row);
                match !bound.capped
                    && (bound.rows <= page.saturating_mul(SET_PAGES) || read <= walked)
                {
                    true => PimdirPlanDriver::Set(set),
                    false => PimdirPlanDriver::Walk {
                        budget: Some((read / row.max(1)).max(page)),
                        fallback: Some(set),
                    },
                }
            }
        };
        let access = self.access(&driver, bounds, costs, force, Some(page));
        PimdirPlanChoice { driver, access }
    }

    /// The smallest set by its bound, a set not measured never chosen.
    fn smallest(
        &self,
        bounds: &[Option<PimdirPlanBound>],
    ) -> Option<(Vec<usize>, PimdirPlanBound)> {
        self.sets
            .iter()
            .filter_map(|set| Some((set.clone(), Self::bound(set, bounds)?)))
            .min_by_key(|(_, bound)| (bound.capped, bound.rows))
    }

    /// How each leaf's text is tested under `driver`: a leaf of the
    /// driving set read into one, another read into one when reading its
    /// matches costs less than probing the candidates (a set's bound, a
    /// walk's expected rows for a page of `page`, the scope's unknown
    /// many), probed otherwise.
    fn access(
        &self,
        driver: &PimdirPlanDriver,
        bounds: &[Option<PimdirPlanBound>],
        costs: &PimdirPlanCosts,
        force: Option<PimdirPlanForce>,
        page: Option<u64>,
    ) -> Vec<PimdirPlanAccess> {
        let candidates = match driver {
            PimdirPlanDriver::Set(set) => Self::bound(set, bounds).map(|bound| bound.rows),
            PimdirPlanDriver::Walk { .. } => page.map(|page| page.saturating_mul(WALK_ROWS)),
            PimdirPlanDriver::Scope => None,
        };
        self.leaves
            .iter()
            .enumerate()
            .map(|(at, leaf)| {
                if !leaf.is_text() {
                    return PimdirPlanAccess::Probe;
                }
                if matches!(driver, PimdirPlanDriver::Set(set) if set.contains(&at)) {
                    return PimdirPlanAccess::Materialize;
                }
                match force {
                    Some(PimdirPlanForce::Walk | PimdirPlanForce::Fallback) => {
                        return PimdirPlanAccess::Probe;
                    }
                    Some(PimdirPlanForce::Scope) => return PimdirPlanAccess::Materialize,
                    _ => (),
                }
                match (bounds.get(at).copied().flatten(), candidates) {
                    (Some(bound), Some(candidates))
                        if bound.capped
                            || bound.rows.saturating_mul(costs.match_row)
                                > candidates.saturating_mul(costs.text_probe) =>
                    {
                        PimdirPlanAccess::Probe
                    }
                    _ => PimdirPlanAccess::Materialize,
                }
            })
            .collect()
    }

    /// What a walk pays per row it examines: a seek placing it, one per
    /// structured term and per text term read into a set, a text probe
    /// per one probed, none for a term the scope implies.
    fn probe_row(&self, access: &[PimdirPlanAccess], costs: &PimdirPlanCosts) -> u64 {
        let mut row = costs.seek;
        for (at, (leaf, access)) in self.leaves.iter().zip(access).enumerate() {
            if self.scoped.contains(&at) {
                continue;
            }
            row = row.saturating_add(match (leaf.is_text(), access) {
                (true, PimdirPlanAccess::Probe) => costs.text_probe,
                _ => costs.seek,
            });
        }
        row
    }

    /// Whether every term is one the scope implies, so every live row a
    /// walk examines places a hit or another placement of one.
    fn is_scoped(&self) -> bool {
        (0..self.leaves.len()).all(|at| self.scoped.contains(&at))
    }

    /// Whether a leaf's set may drive the query: a positive term a
    /// client measures.
    fn is_set(&self, at: usize) -> bool {
        let leaf = &self.leaves[at];
        !leaf.negated && leaf.estimate != PimdirPlanEstimate::None
    }

    /// Lifts one node of the query to the plan, its terms appended to
    /// the leaves.
    fn node(
        &mut self,
        node: &PimdirQuery,
        negated: bool,
        now: i64,
        zone: &TimeZone,
    ) -> PimdirPlanNode {
        match node {
            PimdirQuery::And(operands) => PimdirPlanNode::And(
                operands
                    .iter()
                    .map(|operand| self.node(operand, negated, now, zone))
                    .collect(),
            ),
            PimdirQuery::Or(operands) => PimdirPlanNode::Or(
                operands
                    .iter()
                    .map(|operand| self.node(operand, negated, now, zone))
                    .collect(),
            ),
            PimdirQuery::Not(operand) => {
                PimdirPlanNode::Not(Box::new(self.node(operand, !negated, now, zone)))
            }
            PimdirQuery::Group(operand) => self.node(operand, negated, now, zone),
            PimdirQuery::Term(term) => {
                let dates = match &term.predicate {
                    PimdirQueryPredicate::Date(range) => Some(range.bounds(now, zone)),
                    _ => None,
                };
                let estimate = estimate(&term.predicate, &self.undated);
                self.leaves.push(PimdirPlanLeaf {
                    predicate: term.predicate.clone(),
                    negated,
                    estimate,
                    dates,
                });
                PimdirPlanNode::Leaf(self.leaves.len() - 1)
            }
        }
    }
}

/// One collection a walk merges, with where its rows start.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirWalkCollection {
    /// Its id.
    pub id: String,
    /// Its account, empty for none.
    pub account: String,
    /// Whether its kind is undated (contacts, files).
    pub undated: bool,
    /// The key of its first row after the cursor in its walk's direction
    /// ([`PimdirWalk::head_bound`]), or a key no row after it passes;
    /// `None` when no row follows.
    pub head: Option<String>,
}

/// What a walk asks for: the next rows of one collection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirWalkStep {
    /// The collection, by its position among those the walk was given.
    pub collection: usize,
    /// Whether its rows are read by ascending sort key, else descending,
    /// ties by ascending `seq` either way.
    pub ascending: bool,
    /// The row the rows follow, `(sort_key, seq)`, exclusive; `None`
    /// from the collection's first.
    pub after: Option<(String, i64)>,
    /// The key past which no row can enter the page, inclusive: no row
    /// beyond it is asked for. `None` while the page is not full.
    pub floor: Option<String>,
    /// The most rows answered.
    pub rows: usize,
}

/// One row a step examined: its key, its public id and whether it is a
/// hit placed by it (its representative placement, every term holding).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirWalkRow {
    /// Its sort key.
    pub sort_key: String,
    /// Its public id.
    pub seq: i64,
    /// Whether it places a hit.
    pub pass: bool,
}

/// What a walk answers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirWalkOutput {
    /// The places of the first hits after the cursor, at most the number
    /// asked, in order.
    Page(Vec<PimdirSearchCursor>),
    /// The walk examined more rows than its budget before the page was
    /// full: the caller reads the fallback set instead.
    Exceeded,
}

/// A page in a dated order, merged from the scope's collections in the
/// store's order (SEARCH §8.2, STORAGE §9.3): an I/O-free coroutine
/// yielding [`PimdirWalkStep`]s, each answered with the rows the step
/// examined.
///
/// Each collection's rows come in the page's order, a hit placed by one
/// row alone (its representative placement). The walk keeps the best
/// hits so far, and steps next the collection whose next row ranks best;
/// once the page is full, the key of its last hit is the floor below
/// which no row is asked for, and it stops when no collection's next row
/// can reach it. The dated collections are walked first, in the order's
/// direction, then the undated by ascending key.
#[derive(Clone, Debug)]
pub struct PimdirWalk {
    sort: PimdirSearchSort,
    need: usize,
    collections: Vec<PimdirWalkCollection>,
    after: Vec<Option<(String, i64)>>,
    next: Vec<Option<String>>,
    rows: Vec<usize>,
    held: Vec<PimdirSearchCursor>,
    undated: bool,
    examined: u64,
    budget: Option<u64>,
    pending: Option<usize>,
    done: bool,
}

impl PimdirWalk {
    /// A walk for the first `need` hits after `cursor` in `sort`, a dated
    /// order, over `collections`, giving up past `budget` rows examined.
    pub fn new(
        sort: PimdirSearchSort,
        cursor: Option<&PimdirSearchCursor>,
        need: usize,
        collections: Vec<PimdirWalkCollection>,
        budget: Option<u64>,
    ) -> Self {
        let after = collections
            .iter()
            .map(|collection| Self::start(cursor, collection))
            .collect();
        let next = collections
            .iter()
            .map(
                |collection| match Self::head_bound(sort, cursor, collection.undated) {
                    Some(_) => collection.head.clone(),
                    None => None,
                },
            )
            .collect();
        Self {
            sort,
            need,
            rows: vec![need.clamp(1, STEP_MAX); collections.len()],
            collections,
            after,
            next,
            held: Vec::new(),
            undated: cursor.is_some_and(|cursor| cursor.undated),
            examined: 0,
            budget,
            pending: None,
            done: false,
        }
    }

    /// The bound of a collection's head: `Some((ascending, key))` when
    /// its rows are walked, by ascending or descending sort key, from the
    /// first at or beyond `key` (`None` from its first row), and `None`
    /// when the cursor is past them all, an undated cursor leaving no
    /// dated row.
    pub fn head_bound(
        sort: PimdirSearchSort,
        cursor: Option<&PimdirSearchCursor>,
        undated: bool,
    ) -> Option<(bool, Option<&str>)> {
        let ascending = undated || sort == PimdirSearchSort::OldestFirst;
        match cursor {
            Some(cursor) if cursor.undated && !undated => None,
            Some(cursor) if cursor.undated == undated => {
                Some((ascending, cursor.sort_key.as_deref()))
            }
            _ => Some((ascending, None)),
        }
    }

    /// Resumes the walk with the rows its last step examined, `None` on
    /// the first call.
    pub fn resume(
        &mut self,
        rows: Option<Vec<PimdirWalkRow>>,
    ) -> PimdirCoroutineState<PimdirWalkStep, PimdirWalkOutput> {
        if let (Some(at), Some(rows)) = (self.pending.take(), rows) {
            self.absorb(at, rows);
        }
        if self.done {
            return PimdirCoroutineState::Complete(PimdirWalkOutput::Page(Vec::new()));
        }
        if self.budget.is_some_and(|budget| self.examined > budget) {
            self.done = true;
            return PimdirCoroutineState::Complete(PimdirWalkOutput::Exceeded);
        }

        loop {
            let floor = match self.held.len() >= self.need {
                true => self.held.last().and_then(|hit| hit.sort_key.clone()),
                false => None,
            };
            let full = self.held.len() >= self.need;
            let ascending = self.undated || self.sort == PimdirSearchSort::OldestFirst;
            let best = (0..self.collections.len())
                .filter(|at| self.collections[*at].undated == self.undated)
                .filter_map(|at| Some((at, self.next[at].as_ref()?)))
                .reduce(|left, right| match (ascending, right.1.cmp(left.1)) {
                    (true, Ordering::Less) | (false, Ordering::Greater) => right,
                    _ => left,
                });
            let reaches = |key: &String| match (full, &floor) {
                (false, _) => true,
                (true, None) => false,
                (true, Some(floor)) => match ascending {
                    true => key <= floor,
                    false => key >= floor,
                },
            };
            match best {
                Some((at, key)) if reaches(key) => {
                    self.pending = Some(at);
                    return PimdirCoroutineState::Yielded(PimdirWalkStep {
                        collection: at,
                        ascending,
                        after: self.after[at].clone(),
                        floor: if full { floor } else { None },
                        rows: self.rows[at],
                    });
                }
                _ if !self.undated && !full => self.undated = true,
                _ => {
                    self.done = true;
                    let held = core::mem::take(&mut self.held);
                    return PimdirCoroutineState::Complete(PimdirWalkOutput::Page(held));
                }
            }
        }
    }

    /// The row a collection's walk starts after: the cursor's place as a
    /// keyset of its own, a collection of a later account than the
    /// cursor's taking its row of equal key and `seq` too.
    fn start(
        cursor: Option<&PimdirSearchCursor>,
        collection: &PimdirWalkCollection,
    ) -> Option<(String, i64)> {
        let cursor = cursor?;
        if cursor.undated != collection.undated {
            return None;
        }
        let key = cursor.sort_key.clone()?;
        match collection.account.as_str() > cursor.account.as_str() {
            true => Some((key, cursor.seq.saturating_sub(1))),
            false => Some((key, cursor.seq)),
        }
    }

    /// Takes in the rows one step of collection `at` examined.
    fn absorb(&mut self, at: usize, rows: Vec<PimdirWalkRow>) {
        self.examined = self.examined.saturating_add(rows.len() as u64);
        let exhausted = rows.len() < self.rows[at];
        let collection = &self.collections[at];
        for row in &rows {
            if !row.pass {
                continue;
            }
            let place = PimdirSearchCursor::dated(
                collection.undated,
                row.sort_key.clone(),
                row.seq,
                collection.account.clone(),
            );
            let sort = self.sort;
            let slot = self
                .held
                .partition_point(|held| held.compare(&place, sort) == Ordering::Less);
            if slot < self.need {
                self.held.insert(slot, place);
                self.held.truncate(self.need);
            }
        }
        if let Some(last) = rows.last() {
            self.after[at] = Some((last.sort_key.clone(), last.seq));
        }
        self.next[at] = match exhausted {
            true => None,
            false => rows.last().map(|row| row.sort_key.clone()),
        };
        self.rows[at] = (self.rows[at] * 2).min(STEP_MAX);
    }
}

/// The collection media type of a kind.
pub fn media_type(kind: PimdirQueryKind) -> &'static str {
    match kind {
        PimdirQueryKind::Mail => "message/rfc822",
        PimdirQueryKind::Contact => "text/vcard",
        PimdirQueryKind::Event | PimdirQueryKind::Task | PimdirQueryKind::Journal => {
            "text/calendar"
        }
        PimdirQueryKind::File => file::KIND,
    }
}

/// Whether a collection's kind is undated (SEARCH §8.2): contacts and
/// files, last in both dated orders.
pub fn is_undated(media_type: &str) -> bool {
    media_type == "text/vcard" || media_type == file::KIND
}

/// What a client measures of a term's set, `undated` the undated kinds
/// the query names, every item of which a date term holds.
fn estimate(predicate: &PimdirQueryPredicate, undated: &[&str]) -> PimdirPlanEstimate {
    match predicate {
        PimdirQueryPredicate::Text { .. } => PimdirPlanEstimate::Text,
        PimdirQueryPredicate::Address { text, .. } => match text.text.contains('@') {
            true if text.text.starts_with('@') => PimdirPlanEstimate::None,
            true => PimdirPlanEstimate::Address,
            false => PimdirPlanEstimate::Text,
        },
        PimdirQueryPredicate::Flag(_) => PimdirPlanEstimate::Flag,
        PimdirQueryPredicate::Collection(_) => PimdirPlanEstimate::Collection,
        PimdirQueryPredicate::Changed { .. } => PimdirPlanEstimate::Changed,
        PimdirQueryPredicate::Date(_) if undated.is_empty() => PimdirPlanEstimate::Date,
        PimdirQueryPredicate::Encrypted
        | PimdirQueryPredicate::Thread(_)
        | PimdirQueryPredicate::Id(_)
        | PimdirQueryPredicate::Linked { .. } => PimdirPlanEstimate::Seek,
        PimdirQueryPredicate::Label(_) | PimdirQueryPredicate::Property { .. } => {
            PimdirPlanEstimate::Property
        }
        _ => PimdirPlanEstimate::None,
    }
}

/// The conjuncts of a plan's root: the operands its top-level `and`
/// joins, at any depth, or the root alone.
fn conjuncts(node: &PimdirPlanNode) -> Vec<&PimdirPlanNode> {
    match node {
        PimdirPlanNode::And(operands) => operands.iter().flat_map(conjuncts).collect(),
        node => vec![node],
    }
}

/// Narrows one axis of a scope to `values`: the values both hold.
fn narrow<T: PartialEq>(axis: &mut Option<Vec<T>>, values: Vec<T>) {
    *axis = Some(match axis.take() {
        None => values,
        Some(held) => held
            .into_iter()
            .filter(|value| values.contains(value))
            .collect(),
    });
}

#[cfg(test)]
mod tests {
    use alloc::{collections::BTreeSet, string::ToString};

    use proptest::prelude::*;

    use crate::search::plan::*;

    fn plan(query: &str) -> PimdirPlan {
        PimdirPlan::new(&PimdirQuery::parse(query).unwrap(), 0, &TimeZone::UTC)
    }

    fn bound(rows: u64) -> Option<PimdirPlanBound> {
        Some(PimdirPlanBound {
            rows,
            capped: rows >= CAP,
        })
    }

    #[test]
    fn the_scope_is_what_the_conjuncts_leave() {
        let scope = plan("kind:mail account:home (collection:home/INBOX) x").scope;
        assert_eq!(scope.kinds, Some(vec!["message/rfc822"]));
        assert_eq!(scope.accounts, Some(vec!["home".to_string()]));
        assert_eq!(scope.collections, Some(vec!["home/INBOX".to_string()]));
        assert!(scope.admits("message/rfc822", Some("home"), "home/INBOX"));
        assert!(!scope.admits("message/rfc822", None, "home/INBOX"));

        let none = plan("kind:mail kind:contact").scope;
        assert_eq!(none.kinds, Some(Vec::new()), "two kinds leave none");
        assert!(!none.admits("message/rfc822", None, "a"));

        let open = plan("kind:mail or account:home").scope;
        assert_eq!(open, PimdirPlanScope::default(), "an or rules none out");
        assert_eq!(plan("-kind:mail").scope, PimdirPlanScope::default());
    }

    #[test]
    fn a_set_is_a_positive_conjunct_a_client_reads_whole() {
        assert_eq!(plan("kiwi").sets, vec![vec![0]]);
        assert_eq!(plan("kind:mail is:unread").sets, Vec::<Vec<usize>>::new());
        assert_eq!(plan("kind:mail is:read").sets, vec![vec![1]]);
        assert_eq!(plan("california not gas").sets, vec![vec![0]]);
        assert_eq!(
            plan("from:a@x.org (from:b@x.org or tag:t) kiwi").sets,
            vec![vec![0], vec![1, 2], vec![3]]
        );
        assert_eq!(plan("kiwi or kind:mail").sets, Vec::<Vec<usize>>::new());
        assert_eq!(plan("from:@x.org").sets, Vec::<Vec<usize>>::new());
        assert_eq!(
            plan("date:2026 kind:contact").sets,
            Vec::<Vec<usize>>::new()
        );
        assert_eq!(plan("date:2026 kind:mail").sets, vec![vec![0]]);
        assert_eq!(plan("folder:a").sets, vec![vec![0]]);
        assert_eq!(
            plan("kind:contact label:x -property:a/b=c").sets,
            vec![vec![1]],
            "a label or a property is a set, read by seek"
        );
    }

    #[test]
    fn a_count_reads_its_smallest_set_or_the_scope() {
        let costs = PimdirPlanCosts::default();
        let kiwi = plan("kiwi");
        let choice = kiwi.count(&[bound(35)], &costs, None);
        assert_eq!(choice.driver, PimdirPlanDriver::Set(vec![0]));
        assert_eq!(choice.access, vec![PimdirPlanAccess::Materialize]);

        let both = plan("from:a@x.org california");
        let choice = both.count(&[bound(11_000), bound(40_000)], &costs, None);
        assert_eq!(choice.driver, PimdirPlanDriver::Set(vec![0]));
        assert_eq!(
            choice.access[1],
            PimdirPlanAccess::Materialize,
            "40k matches read cost less than 11k probes"
        );
        let choice = both.count(&[bound(20), bound(CAP)], &costs, None);
        assert_eq!(choice.access[1], PimdirPlanAccess::Probe, "20 probes");

        let unread = plan("is:unread");
        assert_eq!(
            unread.count(&[None], &costs, None).driver,
            PimdirPlanDriver::Scope
        );
        assert_eq!(
            kiwi.count(&[bound(35)], &costs, Some(PimdirPlanForce::Walk)),
            PimdirPlanChoice {
                driver: PimdirPlanDriver::Scope,
                access: vec![PimdirPlanAccess::Probe]
            }
        );
    }

    #[test]
    fn a_dated_page_reads_a_small_set_and_walks_a_broad_one() {
        let costs = PimdirPlanCosts::default();
        let sort = PimdirSearchSort::NewestFirst;
        let kiwi = plan("kiwi");
        assert_eq!(
            kiwi.page(sort, 50, 3_311, &[bound(35)], &costs, None)
                .driver,
            PimdirPlanDriver::Set(vec![0])
        );
        let choice = kiwi.page(sort, 50, 3_311, &[bound(CAP)], &costs, None);
        assert!(
            matches!(&choice.driver, PimdirPlanDriver::Walk { budget: Some(_), fallback: Some(set) } if *set == vec![0])
        );
        assert_eq!(choice.access, vec![PimdirPlanAccess::Probe]);

        assert_eq!(
            kiwi.page(
                sort,
                50,
                1,
                &[bound(35)],
                &costs,
                Some(PimdirPlanForce::Fallback)
            ),
            PimdirPlanChoice {
                driver: PimdirPlanDriver::Walk {
                    budget: Some(0),
                    fallback: Some(vec![0])
                },
                access: vec![PimdirPlanAccess::Probe],
            },
            "a walk giving up at once, its text probed until it does"
        );

        let mail = plan("kind:mail");
        assert_eq!(
            mail.page(sort, 50, 3_311, &[None], &costs, None).driver,
            PimdirPlanDriver::Walk {
                budget: None,
                fallback: None
            }
        );
        let folder = plan("folder:a");
        assert!(
            matches!(
                folder
                    .page(sort, 50, 1, &[bound(1_500)], &costs, None)
                    .driver,
                PimdirPlanDriver::Walk { .. }
            ),
            "a walk of one collection examines one row per hit"
        );
        assert_eq!(
            folder.count(&[bound(1_500)], &costs, None).driver,
            PimdirPlanDriver::Scope,
            "a collection counted on its index"
        );
        let from = plan("from:a@x.org");
        assert_eq!(
            from.page(sort, 50, 10, &[bound(900)], &costs, None).driver,
            PimdirPlanDriver::Set(vec![0]),
            "under 20 pages"
        );
        assert_eq!(
            from.page(sort, 50, 100_000, &[bound(50_000)], &costs, None)
                .driver,
            PimdirPlanDriver::Set(vec![0]),
            "fewer rows than heads to read"
        );
        assert!(matches!(
            from.page(sort, 50, 10, &[bound(50_000)], &costs, None)
                .driver,
            PimdirPlanDriver::Walk { .. }
        ));

        let relevance = plan("kiwi -gas");
        let choice = relevance.page(
            PimdirSearchSort::Relevance,
            50,
            1,
            &[bound(CAP), bound(CAP)],
            &costs,
            Some(PimdirPlanForce::Walk),
        );
        assert_eq!(choice.driver, PimdirPlanDriver::Scope);
        assert_eq!(
            choice.access,
            vec![PimdirPlanAccess::Materialize, PimdirPlanAccess::Probe],
            "a positive text term ranks"
        );
    }

    /// A collection as a test states it: its account, whether undated,
    /// and its rows, `(sort_key, seq, pass)`.
    type Rows = (String, bool, Vec<(String, i64, bool)>);

    /// Answers one step from the rows of a collection.
    fn step(collection: &Rows, step: &PimdirWalkStep) -> Vec<PimdirWalkRow> {
        let mut rows: Vec<&(String, i64, bool)> = collection.2.iter().collect();
        rows.sort_by(|a, b| match step.ascending {
            true => a.0.cmp(&b.0).then(a.1.cmp(&b.1)),
            false => b.0.cmp(&a.0).then(a.1.cmp(&b.1)),
        });
        rows.into_iter()
            .filter(|row| match &step.after {
                None => true,
                Some((key, seq)) => match step.ascending {
                    true => row.0 > *key || (row.0 == *key && row.1 > *seq),
                    false => row.0 < *key || (row.0 == *key && row.1 > *seq),
                },
            })
            .filter(|row| match &step.floor {
                None => true,
                Some(floor) => match step.ascending {
                    true => row.0 <= *floor,
                    false => row.0 >= *floor,
                },
            })
            .take(step.rows)
            .map(|row| PimdirWalkRow {
                sort_key: row.0.clone(),
                seq: row.1,
                pass: row.2,
            })
            .collect()
    }

    /// Runs a walk over `collections` to its end, every head its first
    /// row after the cursor.
    fn walk(
        sort: PimdirSearchSort,
        cursor: Option<&PimdirSearchCursor>,
        need: usize,
        collections: &[Rows],
        budget: Option<u64>,
    ) -> PimdirWalkOutput {
        let given = collections
            .iter()
            .enumerate()
            .map(|(at, (account, undated, _))| {
                let mut collection = PimdirWalkCollection {
                    id: at.to_string(),
                    account: account.clone(),
                    undated: *undated,
                    head: None,
                };
                let after = PimdirWalk::start(cursor, &collection);
                let first = step(
                    &collections[at],
                    &PimdirWalkStep {
                        collection: at,
                        ascending: *undated || sort == PimdirSearchSort::OldestFirst,
                        after,
                        floor: None,
                        rows: 1,
                    },
                );
                collection.head = first.first().map(|row| row.sort_key.clone());
                collection
            })
            .collect();
        let mut walk = PimdirWalk::new(sort, cursor, need, given, budget);
        let mut arg = None;
        loop {
            match walk.resume(arg.take()) {
                PimdirCoroutineState::Yielded(want) => {
                    arg = Some(step(&collections[want.collection], &want));
                }
                PimdirCoroutineState::Complete(output) => return output,
            }
        }
    }

    /// Every hit of `collections`, sorted: the answer a whole set gives.
    fn whole(sort: PimdirSearchSort, collections: &[Rows]) -> Vec<PimdirSearchCursor> {
        let mut hits: Vec<PimdirSearchCursor> = collections
            .iter()
            .flat_map(|(account, undated, rows)| {
                rows.iter().filter(|row| row.2).map(|row| {
                    PimdirSearchCursor::dated(*undated, row.0.clone(), row.1, account.clone())
                })
            })
            .collect();
        hits.sort_by(|a, b| a.compare(b, sort));
        hits
    }

    fn collections() -> impl Strategy<Value = Vec<Rows>> {
        let row = (0u8..6, 1i64..12, prop::bool::weighted(0.4));
        let collection = (
            0u8..3,
            prop::bool::weighted(0.25),
            prop::collection::vec(row, 0..12),
        );
        prop::collection::vec(collection, 0..6).prop_map(|collections| {
            let mut placed = BTreeSet::new();
            collections
                .into_iter()
                .map(|(account, undated, rows)| {
                    let account = ["", "home", "work"][account as usize].to_string();
                    let mut seqs = BTreeSet::new();
                    let rows = rows
                        .into_iter()
                        .filter(|(_, seq, _)| seqs.insert(*seq))
                        .map(|(key, seq, pass)| {
                            let pass = pass && placed.insert((account.clone(), seq));
                            (format!("k{key}"), seq, pass)
                        })
                        .collect();
                    (account, undated, rows)
                })
                .collect()
        })
    }

    proptest! {
        /// A walk answers, at every cursor and page size, the slice of
        /// the whole sorted answer after the cursor.
        #[test]
        fn a_walk_answers_the_slice_of_the_whole_answer(
            collections in collections(),
            need in 1usize..5,
            newest in any::<bool>(),
        ) {
            let sort = match newest {
                true => PimdirSearchSort::NewestFirst,
                false => PimdirSearchSort::OldestFirst,
            };
            let all = whole(sort, &collections);
            let cursors = core::iter::once(None).chain(all.iter().map(Some));
            for (at, cursor) in cursors.enumerate() {
                let want: Vec<PimdirSearchCursor> = all[at..].iter().take(need).cloned().collect();
                let got = walk(sort, cursor, need, &collections, None);
                prop_assert_eq!(got, PimdirWalkOutput::Page(want), "after {}", at);
            }
        }

        /// A walk past its budget gives up, one within it answers.
        #[test]
        fn a_walk_past_its_budget_gives_up(collections in collections()) {
            let sort = PimdirSearchSort::NewestFirst;
            let all = whole(sort, &collections);
            let rows: usize = collections.iter().map(|c| c.2.len()).sum();
            match walk(sort, None, 3, &collections, Some(0)) {
                PimdirWalkOutput::Exceeded => prop_assert!(rows > 0),
                PimdirWalkOutput::Page(page) => prop_assert_eq!(page, all.into_iter().take(3).collect::<Vec<_>>()),
            }
            let within = walk(sort, None, 3, &collections, Some(rows as u64 * 2));
            prop_assert_eq!(within, PimdirWalkOutput::Page(whole(sort, &collections).into_iter().take(3).collect()));
        }
    }
}
