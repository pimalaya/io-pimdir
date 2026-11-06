//! # Compiler
//!
//! A [`PimdirQuery`] compiled to SQL over index.db with the store attached
//! (SEARCH §8): every text term an FTS5 `MATCH` built from escaped tokens,
//! read once through the canonical `match_objects` and `match_summaries`
//! into `temp.term_match` before the query runs, so a query of many terms
//! stays one shallow statement; every structured term a set of
//! placements, through the canonical statements where one exists. A hit
//! is `(account, seq)`, so each term is lifted to the hits any live
//! placement of which it holds for, and `not` is the hits none does: the
//! union rule SEARCH §8.2 states for tags, kept for every term.

use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use jiff::{Timestamp, civil::Date, tz::TimeZone};
use rusqlite::types::Value;

use crate::{
    client::search::PimdirSearchSort,
    search::{
        fold,
        query::{
            PimdirQuery, PimdirQueryDate, PimdirQueryDateRange, PimdirQueryDateUnit, PimdirQueryId,
            PimdirQueryKind, PimdirQueryPredicate, PimdirQueryPriority, PimdirQueryTerm,
            PimdirQueryText, PimdirQueryTextField,
        },
    },
    sql::{self, search},
    summary::{file, time},
};

/// The items each `linked:` term names, as `(kind, link id)` pairs per
/// named id (STORAGE §14.2).
pub(crate) type Ends = Vec<(PimdirQueryId, Vec<(String, String)>)>;

/// The task categories each `category:` term names: its value folded as
/// STORAGE Annex A.2 folds `fn`, and every value a task states that folds
/// to it, as stated.
pub(crate) type Categories = Vec<(String, Vec<String>)>;

/// A compiled query: the statements filling `temp.term_match` with its
/// text terms' matches, the condition a live hit meets, the parameters it
/// binds, and what an answer reports beside its hits.
pub(crate) struct Compiled {
    /// The statements reading each text term's matches into
    /// `temp.term_match`, run before the query, in order.
    pub(crate) matches: Vec<Matches>,
    /// The text terms read so far, each numbered by its position.
    terms: usize,
    /// The text terms no `not` negates, whose matches say what a hit
    /// matched on and rank it.
    positive: Vec<usize>,
    /// The condition over `live(akey, account, seq)`.
    condition: String,
    /// The parameters, bound by position.
    pub(crate) params: Vec<Value>,
    /// The positive text terms' expressions, each probed against a page's
    /// hits to tell what they matched (`match_object`, `match_summary`).
    pub(crate) probes: Vec<Probe>,
    /// The words a snippet looks for: the positive text terms'.
    pub(crate) words: Vec<String>,
    /// The items each `linked:` term names, as `(kind, link id)` pairs
    /// (STORAGE §14.2), resolved against the store by the caller.
    ends: Ends,
    /// The task categories each `category:` term names, resolved against
    /// the store by the caller.
    categories: Categories,
    /// The instant relative dates resolve against, Unix seconds.
    now: i64,
    /// The zone calendar spans are read in.
    zone: TimeZone,
}

impl Compiled {
    /// Compiles a query, `ends` holding the items each `linked:` term
    /// names and `categories` the task categories each `category:` term
    /// names, its dates resolved against `now` (Unix seconds) in `zone`.
    pub(crate) fn new(
        query: &PimdirQuery,
        ends: Ends,
        categories: Categories,
        now: i64,
        zone: TimeZone,
    ) -> Self {
        let undated: Vec<&str> = query
            .terms()
            .into_iter()
            .filter_map(|(term, _)| match &term.predicate {
                PimdirQueryPredicate::Kind(kind) if !kind.is_dated() => Some(media_type(*kind)),
                _ => None,
            })
            .collect();

        let mut compiled = Self {
            matches: Vec::new(),
            terms: 0,
            positive: Vec::new(),
            probes: Vec::new(),
            condition: String::new(),
            params: Vec::new(),
            words: Vec::new(),
            ends,
            categories,
            now,
            zone,
        };
        compiled.condition = compiled.node(query, false, &undated);
        compiled
    }

    /// The hits in `sort` order (SEARCH §8.2), `limit` of them after
    /// `offset`, every one with `None`: account, seq, rank, sort key,
    /// whether undated, title. The `name` order folds
    /// titles as SQL cannot, so its caller sorts the whole set itself;
    /// another order leaves the title empty, read for no hit.
    pub(crate) fn select(
        &mut self,
        sort: PimdirSearchSort,
        page: Option<(usize, usize)>,
    ) -> String {
        let order = match sort {
            PimdirSearchSort::Date => {
                "undated ASC, CASE WHEN undated = 0 THEN sort_key END DESC, \
                 CASE WHEN undated = 1 THEN sort_key END ASC, h.seq ASC, h.akey ASC"
            }
            PimdirSearchSort::Name => "h.seq ASC, h.akey ASC",
            PimdirSearchSort::Relevance => "rank ASC NULLS LAST, h.seq ASC, h.akey ASC",
        };
        let (limit, offset) = page.unwrap_or((i64::MAX as usize, 0));
        let limit = self.bind(Value::Integer(limit.min(i64::MAX as usize) as i64));
        let offset = self.bind(Value::Integer(offset as i64));
        let placements = "FROM store.items i JOIN store.collections c ON c.id = i.collection \
            WHERE i.seq = h.seq AND ifnull(c.account, '') = h.akey \
            AND i.deleted = 0 AND i.retained_at IS NULL";
        // NOTE: the title joins six summary tables per hit, before the order
        // and the page cut: only the `name` order reads it.
        let rank = match sort {
            PimdirSearchSort::Relevance => {
                "(SELECT min(m.rank) FROM matched m WHERE m.akey = h.akey AND m.seq = h.seq)"
            }
            _ => "NULL",
        };
        let title = match sort {
            PimdirSearchSort::Name => format!(
                "(SELECT coalesce(ms.subject, cs.fn, es.summary, ts.summary, js.summary, fs.name, '') \
                 {TITLES} WHERE i.seq = h.seq AND ifnull(c.account, '') = h.akey AND i.deleted = 0 \
                 AND i.retained_at IS NULL ORDER BY i.collection LIMIT 1)"
            ),
            _ => String::from("''"),
        };

        format!(
            "{}
SELECT h.account, h.seq,
  {rank} AS rank,
  (SELECT max(i.sort_key) {placements}) AS sort_key,
  (SELECT max(c.kind IN ('text/vcard', 'application/octet-stream')) {placements}) AS undated,
  {title} AS title
FROM hits h ORDER BY {order} LIMIT {limit} OFFSET {offset};",
            self.with(),
        )
    }

    /// How many hits the query has.
    pub(crate) fn count(&self) -> String {
        format!("{}\nSELECT count(*) FROM hits;", self.with())
    }

    /// The common table expressions every statement starts with: the live
    /// hits, the hits meeting the condition, and the matches of the
    /// positive text terms.
    fn with(&self) -> String {
        let mut ctes = Vec::new();
        ctes.push(
            "live(akey, account, seq) AS (SELECT DISTINCT ifnull(c.account, ''), c.account, i.seq \
             FROM store.items i JOIN store.collections c ON c.id = i.collection \
             WHERE i.deleted = 0 AND i.retained_at IS NULL)"
                .into(),
        );
        ctes.push(format!(
            "hits(akey, account, seq) AS (SELECT akey, account, seq FROM live WHERE {})",
            self.condition
        ));

        let positive: Vec<String> = self.positive.iter().map(ToString::to_string).collect();
        let matched = format!(
            "SELECT akey, seq, src, rank FROM temp.term_match WHERE term IN ({})",
            positive.join(", ")
        );
        ctes.push(format!("matched(akey, seq, src, rank) AS ({matched})"));

        format!("WITH {}", ctes.join(",\n"))
    }

    /// Binds one parameter, answering its position.
    fn bind(&mut self, value: Value) -> String {
        self.params.push(value);
        format!("?{}", self.params.len())
    }

    /// Embeds a canonical statement as a subquery of the query, each named
    /// parameter bound by position.
    fn embed(&mut self, statement: &str, binds: Vec<(&str, Value)>) -> String {
        embed(statement, binds, &mut self.params)
    }

    /// Reads a text term's matches into `temp.term_match` before the query
    /// runs, through `statement` (`match_objects` or `match_summaries`),
    /// what matched named `src`, those of collections of another kind than
    /// `kind` left out.
    fn matches(
        &mut self,
        term: usize,
        statement: &str,
        src: &str,
        expression: String,
        kind: Option<&str>,
    ) {
        let mut params = Vec::new();
        let matched = embed(statement, vec![(":match", expression.into())], &mut params);
        let kind = match kind {
            Some(kind) => format!(" WHERE c.kind = '{kind}'"),
            None => String::new(),
        };
        let sql = format!(
            "INSERT INTO temp.term_match(term, collection, akey, seq, src, rank) \
             SELECT {term}, m.collection, ifnull(c.account, ''), m.seq, '{src}', m.rank \
             FROM ({matched}) m JOIN store.collections c ON c.id = m.collection{kind}"
        );
        self.matches.push(Matches { sql, params });
    }

    /// Compiles one node to a condition over `live`.
    fn node(&mut self, node: &PimdirQuery, negated: bool, undated: &[&str]) -> String {
        match node {
            PimdirQuery::And(operands) | PimdirQuery::Or(operands) => {
                let operator = match node {
                    PimdirQuery::And(_) => " AND ",
                    _ => " OR ",
                };
                let mut conditions = Vec::new();
                for operand in operands {
                    conditions.push(self.node(operand, negated, undated));
                }
                format!("({})", conditions.join(operator))
            }
            PimdirQuery::Not(operand) => {
                format!("(NOT {})", self.node(operand, !negated, undated))
            }
            PimdirQuery::Term(term) => self.term(term, negated, undated),
        }
    }

    /// Compiles one term to a condition over `live`.
    fn term(&mut self, term: &PimdirQueryTerm, negated: bool, undated: &[&str]) -> String {
        let live = "FROM store.items i JOIN store.collections c ON c.id = i.collection \
                    WHERE i.deleted = 0 AND i.retained_at IS NULL";

        let placements = match &term.predicate {
            PimdirQueryPredicate::Text { field, text } => {
                let (column, summary) = match field {
                    PimdirQueryTextField::Any => (None, true),
                    PimdirQueryTextField::Title => (Some("title"), true),
                    PimdirQueryTextField::Body => (Some("body"), false),
                    PimdirQueryTextField::Attachment => (Some("attachment"), false),
                    PimdirQueryTextField::Place => (Some("place"), false),
                };
                return self.text(column, summary, text, negated);
            }
            PimdirQueryPredicate::Address { role, text } if text.text.contains('@') => {
                let address = text.text.to_lowercase();
                let role = Value::from(role.map(|role| role.as_str().to_string()));
                match address.strip_prefix('@') {
                    Some(domain) => self.embed(
                        sql::LIST_DOMAIN_PLACEMENTS,
                        vec![(":domain", domain.to_string().into()), (":role", role)],
                    ),
                    // NOTE: one page holding every placement, a `LIMIT` of
                    // -1 being SQLite's unbounded one: the term is a set,
                    // which the query's own order pages.
                    None => self.embed(
                        sql::LIST_ADDRESS_PLACEMENTS,
                        vec![
                            (":address", address.into()),
                            (":role", role),
                            (":after_key", Value::Null),
                            (":after_seq", Value::Null),
                            (":after_collection", Value::Null),
                            (":after_role", Value::Null),
                            (":limit", Value::Integer(-1)),
                        ],
                    ),
                }
            }
            PimdirQueryPredicate::Address { role, text } => {
                let column = role.map(|role| role.as_str()).unwrap_or("people");
                return self.text(Some(column), true, text, negated);
            }
            PimdirQueryPredicate::Person(text) => self.person(text),
            PimdirQueryPredicate::Thread(id) => {
                // NOTE: the thread any message of it names, so a thread
                // id still reaches its members after a join renamed it,
                // the old id being one of theirs (SEARCH §9).
                let thread = self.embed(search::THREAD_OF, vec![(":link_id", id.clone().into())]);
                format!(
                    "SELECT p.collection, p.seq FROM message m \
                     JOIN placement p ON p.link_id = m.link_id \
                     JOIN store.collections c ON c.id = p.collection \
                     WHERE c.kind = 'message/rfc822' AND m.thread = ({thread})"
                )
            }
            PimdirQueryPredicate::Linked { role, id } => {
                let ends = self
                    .ends
                    .iter()
                    .find(|(named, _)| named == id)
                    .map(|(_, ends)| ends.clone())
                    .unwrap_or_default();
                let role = Value::from(role.as_ref().map(|role| role.as_str().to_string()));
                let mut linked = vec!["SELECT NULL AS collection, NULL AS seq WHERE 0".to_string()];
                for (kind, link_id) in ends {
                    linked.push(self.embed(
                        search::LINKED,
                        vec![
                            (":kind", kind.into()),
                            (":link_id", link_id.into()),
                            (":role", role.clone()),
                        ],
                    ));
                }
                linked.join(" UNION ")
            }
            PimdirQueryPredicate::Flag(flag) => {
                self.embed(search::FLAGGED, vec![(":flag", flag.clone().into())])
            }
            PimdirQueryPredicate::Encrypted => {
                "SELECT p.collection, p.seq FROM placement p JOIN object o ON o.hash = p.hash \
                 WHERE o.status = 'encrypted'"
                    .into()
            }
            PimdirQueryPredicate::HasBody => {
                format!("SELECT i.collection, i.seq {live} AND i.object_hash IS NOT NULL")
            }
            PimdirQueryPredicate::HasAttachment => {
                // NOTE: a calendar item by the mark its body's extraction
                // recorded (SEARCH §6), the FTS rows keeping no text.
                let attached = self.embed(search::ATTACHED, Vec::new());
                format!(
                    "SELECT i.collection, i.seq {live} AND (EXISTS (SELECT 1 FROM store.mail_summary m \
                     WHERE m.item = i.id AND m.attachment = 1) \
                     OR (c.kind = 'text/calendar' AND (i.collection, i.seq) IN ({attached})))"
                )
            }
            PimdirQueryPredicate::HasProperty(property) => {
                // NOTE: a card by the mark its body's extraction recorded
                // (SEARCH §6); every other kind records none.
                let held = self.embed(
                    search::HOLDING,
                    vec![(":property", property.as_str().to_string().into())],
                );
                format!("SELECT i.collection, i.seq {live} AND (i.collection, i.seq) IN ({held})")
            }
            PimdirQueryPredicate::Kind(kind) => match kind {
                PimdirQueryKind::Event | PimdirQueryKind::Task | PimdirQueryKind::Journal => {
                    let table = match kind {
                        PimdirQueryKind::Event => "event_summary",
                        PimdirQueryKind::Task => "task_summary",
                        _ => "journal_summary",
                    };
                    format!(
                        "SELECT i.collection, i.seq {live} AND EXISTS (SELECT 1 FROM store.{table} s \
                         WHERE s.item = i.id)"
                    )
                }
                kind => {
                    let kind = self.bind(media_type(*kind).to_string().into());
                    format!("SELECT i.collection, i.seq {live} AND c.kind = {kind}")
                }
            },
            PimdirQueryPredicate::Status(status) => {
                let status = self.bind(status.clone().into());
                format!(
                    "SELECT i.collection, i.seq {live} AND EXISTS (SELECT 1 FROM store.task_summary s \
                     WHERE s.item = i.id \
                     AND s.status = {status})"
                )
            }
            PimdirQueryPredicate::Priority(priority) => {
                let band = match priority {
                    PimdirQueryPriority::High => "s.priority BETWEEN 1 AND 4",
                    PimdirQueryPriority::Medium => "s.priority = 5",
                    PimdirQueryPriority::Low => "s.priority BETWEEN 6 AND 9",
                    PimdirQueryPriority::None => "ifnull(s.priority, 0) = 0",
                };
                // NOTE: a store whose owner predates the column reads it as
                // NULL (STORAGE §6), every task then of no priority.
                format!(
                    "SELECT i.collection, i.seq {live} AND EXISTS (SELECT 1 FROM store.task_summary s \
                         WHERE s.item = i.id AND {band})"
                )
            }
            PimdirQueryPredicate::Category(category) => {
                // NOTE: both sides folded as STORAGE Annex A.2 folds `fn`,
                // in Rust: a task by the `task_category` values the caller
                // found folding to the term's, compared as stated.
                // NOTE: a card by the values its body's extraction recorded
                // (SEARCH §6), folded already, contact_summary keeping none.
                let folded = fold::value(category);
                let stated: Vec<String> = self
                    .categories
                    .iter()
                    .find(|(term, _)| *term == folded)
                    .map(|(_, values)| values.clone())
                    .unwrap_or_default();
                let carded =
                    self.embed(search::CATEGORISED, vec![(":value", folded.clone().into())]);
                let stated = match stated.is_empty() {
                    true => String::from("0"),
                    false => {
                        let values: Vec<String> = stated
                            .into_iter()
                            .map(|value| self.bind(value.into()))
                            .collect();
                        format!("s.value IN ({})", values.join(", "))
                    }
                };
                format!(
                    "SELECT i.collection, i.seq {live} AND (EXISTS (SELECT 1 FROM store.task_category s \
                         WHERE s.item = i.id \
                         AND {stated}) OR (i.collection, i.seq) IN ({carded}))"
                )
            }
            PimdirQueryPredicate::Account(account) => {
                return format!("(akey = {})", self.bind(account.clone().into()));
            }
            PimdirQueryPredicate::Collection(collection) => {
                let collection = self.bind(collection.clone().into());
                format!("SELECT i.collection, i.seq {live} AND i.collection = {collection}")
            }
            PimdirQueryPredicate::Date(range) => {
                let (since, until) = bounds(range, self.now, &self.zone);
                // NOTE: an open side is the earliest or the latest instant an
                // RFC 3339 string can spell.
                let occurring = self.embed(
                    search::OCCURRING_BETWEEN,
                    vec![
                        (":start", since.clone().unwrap_or_default().into()),
                        (
                            ":end",
                            until
                                .clone()
                                .unwrap_or_else(|| "9999-12-31T23:59:59Z".into())
                                .into(),
                        ),
                    ],
                );
                let since = self.bind(since.into());
                let until = self.bind(until.into());
                // NOTE: the undated kinds a `kind:` term names, each bound,
                // `NULL` alone for none, which no kind equals.
                let mut kinds = Vec::new();
                for kind in undated {
                    kinds.push(self.bind(kind.to_string().into()));
                }
                let undated = match kinds.is_empty() {
                    true => String::from("NULL"),
                    false => kinds.join(", "),
                };
                format!(
                    "SELECT i.collection, i.seq {live} AND ((c.kind = 'message/rfc822' \
                     OR (c.kind = 'text/calendar' AND i.object_hash IS NULL)) \
                     AND i.sort_key != '' AND ({since} IS NULL OR i.sort_key >= {since}) \
                     AND ({until} IS NULL OR i.sort_key < {until})) \
                     UNION SELECT collection, seq FROM ({occurring}) \
                     UNION SELECT i.collection, i.seq {live} \
                     AND c.kind IN ({undated})"
                )
            }
            PimdirQueryPredicate::Changed { since, until } => {
                let since = self.bind((*since).into());
                let until = self.bind((*until).into());
                format!(
                    "SELECT i.collection, i.seq {live} AND ({since} IS NULL OR i.changed >= {since}) \
                     AND ({until} IS NULL OR i.changed <= {until})"
                )
            }
            PimdirQueryPredicate::Id(id) => {
                // NOTE: a value of digits names the item of that seq and
                // every item whose link id it is (SEARCH §8).
                let link = self.bind(id.as_str().to_string().into());
                let linked = member(&format!(
                    "SELECT ifnull(c.account, ''), i.seq {live} AND i.link_id = {link}"
                ));
                return match id.seq() {
                    Some(seq) => format!("((seq = {}) OR {linked})", self.bind(seq.into())),
                    None => linked,
                };
            }
        };

        member(&format!(
            "SELECT ifnull(c.account, ''), x.seq FROM ({placements}) x \
             JOIN store.collections c ON c.id = x.collection"
        ))
    }

    /// The placements of `person:` (SEARCH §8): the contacts whose title
    /// matches, and every item naming one of their addresses in any role.
    fn person(&mut self, text: &PimdirQueryText) -> String {
        let at = self.terms;
        self.terms += 1;
        if let Some(expression) = fts(Some("title"), text) {
            let card = Some("text/vcard");
            self.matches(at, search::MATCH_OBJECTS, "body", expression.clone(), card);
            self.matches(at, search::MATCH_SUMMARIES, "summary", expression, card);
        }
        let contacts = format!("SELECT collection, seq FROM temp.term_match WHERE term = {at}");

        format!(
            "{contacts} UNION \
             SELECT i.collection, i.seq FROM store.item_address a \
             JOIN store.items i ON i.id = a.item \
             WHERE i.deleted = 0 AND i.retained_at IS NULL AND a.address IN ( \
               SELECT e.address FROM store.item_address e \
               JOIN store.items ei ON ei.id = e.item \
               WHERE e.role = 'email' AND (ei.collection, ei.seq) IN ({contacts}))"
        )
    }

    /// Compiles a text term to its matches over bodies and, when the field
    /// exists there, over summaries, read into `temp.term_match` under the
    /// term's number, answering the condition over `live`.
    fn text(
        &mut self,
        column: Option<&str>,
        summary: bool,
        text: &PimdirQueryText,
        negated: bool,
    ) -> String {
        let at = self.terms;
        self.terms += 1;
        if let Some(expression) = fts(column, text) {
            self.matches(at, search::MATCH_OBJECTS, "body", expression.clone(), None);
            if summary {
                self.matches(at, search::MATCH_SUMMARIES, "summary", expression, None);
            }
        }

        if !negated {
            self.positive.push(at);
            self.words.push(text.text.clone());
            if let Some(expression) = fts(column, text) {
                self.probes.push(Probe {
                    expression,
                    summary,
                });
            }
        }

        member(&format!(
            "SELECT akey, seq FROM temp.term_match WHERE term = {at}"
        ))
    }
}

/// A positive text term, probed against one hit's body or summary.
pub(crate) struct Probe {
    /// Its FTS5 expression (SEARCH §8.5).
    pub(crate) expression: String,
    /// Whether its field exists in a summary, which it then matches too.
    pub(crate) summary: bool,
}

/// A statement filling `temp.term_match`, run before the query.
pub(crate) struct Matches {
    /// The statement.
    pub(crate) sql: String,
    /// Its parameters, bound by position.
    pub(crate) params: Vec<Value>,
}

/// Embeds a canonical statement as a subquery: its comments and final
/// semicolon stripped, each named parameter bound by position after those
/// `params` holds.
fn embed(statement: &str, binds: Vec<(&str, Value)>, params: &mut Vec<Value>) -> String {
    let mut text: String = statement
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    text = text.trim().trim_end_matches(';').to_string();

    for (name, value) in binds {
        params.push(value);
        let position = format!("?{}", params.len());
        text = replace_param(&text, name, &position);
    }
    text
}

/// The condition that a hit of `live` is among `hits`, a statement
/// answering `(akey, seq)` pairs, as 1 or 0 and never `NULL`.
///
/// A bare `(akey, seq) IN (...)` may be `NULL` as far as SQLite knows, so
/// under `not`, where `NULL` and false part ways, it walks every row of the
/// set for each hit it does not hold, to tell a partial `NULL` match from
/// none: `is:unread`, no `\Seen`, took 24 s over 50,000 messages. Read as
/// the test of a `CASE`, where `NULL` is false, it is one probe of the set.
fn member(hits: &str) -> String {
    format!("(CASE WHEN (akey, seq) IN ({hits}) THEN 1 ELSE 0 END)")
}

/// The titles a hit's placements carry, joined for the `name` order.
const TITLES: &str = "FROM store.items i JOIN store.collections c ON c.id = i.collection
   LEFT JOIN store.mail_summary ms ON ms.item = i.id
   LEFT JOIN store.contact_summary cs ON cs.item = i.id
   LEFT JOIN store.event_summary es ON es.item = i.id
   LEFT JOIN store.task_summary ts ON ts.item = i.id
   LEFT JOIN store.journal_summary js ON js.item = i.id
   LEFT JOIN store.file_summary fs ON fs.item = i.id";

/// Seconds in a day.
const DAY: i64 = 86_400;

/// The first instant of year 0, the earliest an RFC 3339 instant spells:
/// `0000-01-01T00:00:00Z`.
const YEAR_ZERO: i64 = -62_167_219_200;

/// The collection media type of a kind.
pub(crate) fn media_type(kind: PimdirQueryKind) -> &'static str {
    match kind {
        PimdirQueryKind::Mail => "message/rfc822",
        PimdirQueryKind::Contact => "text/vcard",
        PimdirQueryKind::Event | PimdirQueryKind::Task | PimdirQueryKind::Journal => {
            "text/calendar"
        }
        PimdirQueryKind::File => file::KIND,
    }
}

/// The instants a range spans (SEARCH §8.1), `[since, until)` as RFC 3339
/// in UTC, either side `None` open: from the start of its first point's
/// span to the end of its last's, a calendar point spanning its day,
/// month or year in `zone`, `today` and `yesterday` theirs around `now`
/// (Unix seconds), and a relative amount the instant that long before
/// `now`, its span alone running from that instant to `now`.
///
/// A single point is parsed as a range from it to itself, so `2w..2w`
/// reads as `2w` alone.
pub(crate) fn bounds(
    range: &PimdirQueryDateRange,
    now: i64,
    zone: &TimeZone,
) -> (Option<String>, Option<String>) {
    let single = range.start.is_some() && range.start == range.end;
    let since = range.start.as_ref().map(|point| span(point, now, zone).0);
    let until = range.end.as_ref().map(|point| match point {
        PimdirQueryDate::Relative { .. } if single => now,
        point => span(point, now, zone).1,
    });
    (since.map(time::rfc3339), until.map(time::rfc3339))
}

/// The span of one date point, `[start, end)` in Unix seconds, its days
/// read in `zone`; a relative amount spans its instant, never before year 0.
fn span(point: &PimdirQueryDate, now: i64, zone: &TimeZone) -> (i64, i64) {
    let day = |year: u16, month: u8, day: u8| {
        time::days_from_civil(i32::from(year), u32::from(month), u32::from(day))
    };
    let start = |days: i64| start_of_day(days, zone);
    let today = Timestamp::from_second(now).map_or(now.div_euclid(DAY), |instant| {
        let date = instant.to_zoned(zone.clone()).date();
        time::days_from_civil(
            i32::from(date.year()),
            date.month() as u32,
            date.day() as u32,
        )
    });

    match *point {
        PimdirQueryDate::Year(year) => (start(day(year, 1, 1)), start(day(year + 1, 1, 1))),
        PimdirQueryDate::Month { year, month } => {
            let next = match month {
                12 => day(year + 1, 1, 1),
                month => day(year, month + 1, 1),
            };
            (start(day(year, month, 1)), start(next))
        }
        PimdirQueryDate::Day {
            year,
            month,
            day: date,
        } => {
            let first = day(year, month, date);
            (start(first), start(first + 1))
        }
        PimdirQueryDate::Today => (start(today), start(today + 1)),
        PimdirQueryDate::Yesterday => (start(today - 1), start(today)),
        PimdirQueryDate::Relative { amount, unit } => {
            let amount = i64::from(amount);
            let instant = match unit {
                PimdirQueryDateUnit::Day => now - amount * DAY,
                PimdirQueryDateUnit::Week => now - amount * 7 * DAY,
                PimdirQueryDateUnit::Year => {
                    let (year, month, date) = time::civil_from_days(now.div_euclid(DAY));
                    let year = i64::from(year) - amount;
                    match i32::try_from(year) {
                        Ok(year) if year >= 0 => {
                            let date = date.min(time::days_in_month(year, month));
                            time::days_from_civil(year, month, date) * DAY + now.rem_euclid(DAY)
                        }
                        _ => YEAR_ZERO,
                    }
                }
            };
            let instant = instant.max(YEAR_ZERO);
            (instant, instant)
        }
    }
}

/// The first instant of a day counted from 1970-01-01 in `zone`, its UTC
/// midnight past the years jiff places.
fn start_of_day(days: i64, zone: &TimeZone) -> i64 {
    let (year, month, day) = time::civil_from_days(days);
    i16::try_from(year)
        .ok()
        .and_then(|year| Date::new(year, month as i8, day as i8).ok())
        .and_then(|date| date.to_zoned(zone.clone()).ok())
        .and_then(|zoned| zoned.start_of_day().ok())
        .map_or(days * DAY, |zoned| zoned.timestamp().as_second())
}

/// The FTS5 expression of a text term: the text folded as the indexer
/// folds it (SEARCH §5) and written as one escaped string, so the
/// tokenizer reads its words as a phrase and nothing as syntax, a prefix
/// starred, filtered to a column. `None` for a text holding no word,
/// which matches nothing.
fn fts(column: Option<&str>, text: &PimdirQueryText) -> Option<String> {
    if !text.text.chars().any(char::is_alphanumeric) {
        return None;
    }

    let folded = fold::text(&text.text);
    let mut expression = format!("\"{}\"", folded.replace('"', "\"\""));
    if text.prefix {
        expression += " *";
    }
    Some(match column {
        Some(column) => format!("{{{column}}} : {expression}"),
        None => expression,
    })
}

/// Replaces a named parameter by a positional one wherever it stands
/// whole, so `:role` leaves `:roles` alone.
fn replace_param(text: &str, name: &str, position: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find(name) {
        let after = &rest[at + name.len()..];
        let whole = !after
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        out.push_str(&rest[..at]);
        out.push_str(if whole { position } else { name });
        rest = after;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use crate::{
        client::search::compile::*,
        search::query::{PimdirQueryPredicate, PimdirQueryTerm},
        summary::time,
    };

    fn bounds_of(value: &str, now: i64) -> (Option<String>, Option<String>) {
        bounds_in(value, now, "UTC")
    }

    fn bounds_in(value: &str, now: i64, zone: &str) -> (Option<String>, Option<String>) {
        let zone = TimeZone::get(zone).unwrap();
        match PimdirQuery::parse(&format!("date:{value}")).unwrap() {
            PimdirQuery::Term(PimdirQueryTerm {
                predicate: PimdirQueryPredicate::Date(range),
                ..
            }) => bounds(&range, now, &zone),
            _ => panic!("a date term"),
        }
    }

    #[test]
    fn relative_dates_resolve_against_now() {
        let now = time::unix(2028, 2, 29, 15, 30, 0);
        let span = |since: &str, until: &str| (Some(since.into()), Some(until.into()));

        assert_eq!(
            bounds_of("1y", now),
            span("2027-02-28T15:30:00Z", "2028-02-29T15:30:00Z"),
            "a calendar year back from 29 February lands on the 28th"
        );
        assert_eq!(
            bounds_of("3d..1d", now),
            span("2028-02-26T15:30:00Z", "2028-02-28T15:30:00Z"),
            "a relative side is its instant, not a span to now"
        );
        assert_eq!(
            bounds_of("yesterday..today", now),
            span("2028-02-28T00:00:00Z", "2028-03-01T00:00:00Z")
        );
        assert_eq!(
            bounds_of("..2w", now),
            (None, Some("2028-02-15T15:30:00Z".into()))
        );
        assert_eq!(
            bounds_of("99999y", now).0.as_deref(),
            Some("0000-01-01T00:00:00Z"),
            "nothing before year 0"
        );
    }

    #[test]
    fn calendar_spans_start_at_the_zone_midnight() {
        let span = |since: &str, until: &str| (Some(since.into()), Some(until.into()));
        let after = time::unix(2026, 12, 31, 23, 30, 0);
        let before = time::unix(2026, 12, 31, 22, 30, 0);

        assert_eq!(
            bounds_in("today", after, "Europe/Paris"),
            span("2026-12-31T23:00:00Z", "2027-01-01T23:00:00Z"),
            "00:30 in Paris is already the new year's first day"
        );
        assert_eq!(
            bounds_of("today", after),
            span("2026-12-31T00:00:00Z", "2027-01-01T00:00:00Z")
        );
        assert_eq!(
            bounds_in("today", before, "Europe/Paris"),
            span("2026-12-30T23:00:00Z", "2026-12-31T23:00:00Z"),
            "23:30 in Paris is still the old year's last day"
        );
        assert_eq!(
            bounds_in(
                "yesterday",
                time::unix(2027, 1, 1, 23, 30, 0),
                "Europe/Paris"
            ),
            span("2026-12-31T23:00:00Z", "2027-01-01T23:00:00Z")
        );
        assert_eq!(
            bounds_in("2027", after, "Europe/Paris"),
            span("2026-12-31T23:00:00Z", "2027-12-31T23:00:00Z")
        );
        assert_eq!(
            bounds_in("2027-01", after, "Europe/Paris"),
            span("2026-12-31T23:00:00Z", "2027-01-31T23:00:00Z")
        );
        assert_eq!(
            bounds_in("2026-12-31", after, "Europe/Paris"),
            span("2026-12-30T23:00:00Z", "2026-12-31T23:00:00Z")
        );
        assert_eq!(
            bounds_in("1d", after, "Europe/Paris"),
            span("2026-12-30T23:30:00Z", "2026-12-31T23:30:00Z"),
            "a relative amount is an instant, whatever the zone"
        );
    }

    #[test]
    fn a_day_a_transition_moves_is_23_or_25_hours() {
        let span = |since: &str, until: &str| (Some(since.into()), Some(until.into()));
        let now = time::unix(2026, 8, 20, 12, 0, 0);

        assert_eq!(
            bounds_in("2026-10-25", now, "Europe/Paris"),
            span("2026-10-24T22:00:00Z", "2026-10-25T23:00:00Z"),
            "the clocks go back: 25 hours"
        );
        assert_eq!(
            bounds_in("2027-03-28", now, "Europe/Paris"),
            span("2027-03-27T23:00:00Z", "2027-03-28T22:00:00Z"),
            "the clocks go forward: 23 hours"
        );
        assert_eq!(
            bounds_in("2026-03-08", now, "America/Havana"),
            span("2026-03-08T05:00:00Z", "2026-03-09T04:00:00Z"),
            "a transition skipping midnight: the day starts at 01:00"
        );
    }
}
