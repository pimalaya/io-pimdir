//! # Compiler
//!
//! A [`PimdirQuery`] compiled to one SQL statement over index.db with the
//! store attached (SEARCH §8): every text term an FTS5 `MATCH` built from
//! escaped tokens, through the canonical `match_objects` and
//! `match_summaries`; every structured term a set of placements, through
//! the canonical statements where one exists. A hit is `(account, seq)`,
//! so each term is lifted to the hits any live placement of which it
//! holds for, and `not` is the hits none does: the union rule SEARCH §8.2
//! states for tags, kept for every term.

use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use rusqlite::types::Value;

use crate::{
    client::search::PimdirSearchSort,
    search::query::{
        PimdirQuery, PimdirQueryDate, PimdirQueryDateRange, PimdirQueryDateUnit, PimdirQueryId,
        PimdirQueryKind, PimdirQueryPredicate, PimdirQueryPriority, PimdirQueryTerm,
        PimdirQueryText, PimdirQueryTextField,
    },
    sql::{self, search},
    summary::{file, time},
};

/// The SQL function folding a stored value as [`fold`] folds a query's,
/// registered on every search connection.
pub(crate) const FOLD: &str = "pimdir_fold";

/// A value folded as STORAGE Annex A.2 folds `fn`: the Unicode default
/// full lowercase mapping, locale-independent, then trimmed.
pub(crate) fn fold(value: &str) -> String {
    value.to_lowercase().trim().to_string()
}

/// The items each `linked:` term names, as `(kind, link id)` pairs per
/// named id (STORAGE §14.2).
pub(crate) type Ends = Vec<(PimdirQueryId, Vec<(String, String)>)>;

/// A compiled query: its common table expressions, the condition a live
/// hit meets, the parameters both bind, and what an answer reports
/// beside its hits.
pub(crate) struct Compiled {
    /// One `t<n>(akey, seq, src, rank)` per text term.
    ctes: Vec<String>,
    /// The text terms no `not` negates, whose matches say what a hit
    /// matched on and rank it.
    positive: Vec<usize>,
    /// The condition over `live(akey, account, seq)`.
    condition: String,
    /// The parameters, bound by position.
    pub(crate) params: Vec<Value>,
    /// The words a snippet looks for: the positive text terms'.
    pub(crate) words: Vec<String>,
    /// The items each `linked:` term names, as `(kind, link id)` pairs
    /// (STORAGE §14.2), resolved against the store by the caller.
    ends: Ends,
    /// The instant relative dates resolve against, Unix seconds.
    now: i64,
}

impl Compiled {
    /// Compiles a query, `ends` holding the items each `linked:` term
    /// names, its relative dates resolved against `now` (Unix seconds).
    pub(crate) fn new(query: &PimdirQuery, ends: Ends, now: i64) -> Self {
        let undated: Vec<&str> = query
            .terms()
            .into_iter()
            .filter_map(|(term, _)| match &term.predicate {
                PimdirQueryPredicate::Kind(kind) if !kind.is_dated() => Some(media_type(*kind)),
                _ => None,
            })
            .collect();

        let mut compiled = Self {
            ctes: Vec::new(),
            positive: Vec::new(),
            condition: String::new(),
            params: Vec::new(),
            words: Vec::new(),
            ends,
            now,
        };
        compiled.condition = compiled.node(query, false, &undated);
        compiled
    }

    /// The hits in `sort` order (SEARCH §8.2), `limit` of them after
    /// `offset`, every one with `None`: account, seq, what they matched
    /// on, rank, sort key, whether undated, title. The `name` order folds
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
  (SELECT CASE WHEN max(m.src = 'body') = 1 THEN 'body' WHEN count(*) > 0 THEN 'summary' END
   FROM matched m WHERE m.akey = h.akey AND m.seq = h.seq) AS matched,
  (SELECT min(m.rank) FROM matched m WHERE m.akey = h.akey AND m.seq = h.seq) AS rank,
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

    /// The common table expressions every statement starts with: the text
    /// terms, the live hits, the hits meeting the condition, and the
    /// matches of the positive text terms.
    fn with(&self) -> String {
        let mut ctes = self.ctes.clone();
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

        let matched = match self.positive.is_empty() {
            true => "SELECT NULL, NULL, NULL, NULL WHERE 0".to_string(),
            false => self
                .positive
                .iter()
                .map(|at| format!("SELECT akey, seq, src, rank FROM t{at}"))
                .collect::<Vec<_>>()
                .join(" UNION ALL "),
        };
        ctes.push(format!("matched(akey, seq, src, rank) AS ({matched})"));

        format!("WITH {}", ctes.join(",\n"))
    }

    /// Binds one parameter, answering its position.
    fn bind(&mut self, value: Value) -> String {
        self.params.push(value);
        format!("?{}", self.params.len())
    }

    /// Embeds a canonical statement as a subquery: its comments and final
    /// semicolon stripped, each named parameter bound by position.
    fn embed(&mut self, statement: &str, binds: Vec<(&str, Value)>) -> String {
        let mut text: String = statement
            .lines()
            .filter(|line| !line.trim_start().starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n");
        text = text.trim().trim_end_matches(';').to_string();

        for (name, value) in binds {
            let position = self.bind(value);
            text = replace_param(&text, name, &position);
        }
        text
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
                    None => self.embed(
                        sql::LIST_ADDRESS_PLACEMENTS,
                        vec![(":address", address.into()), (":role", role)],
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
                     WHERE m.collection = i.collection AND m.link_id = i.link_id AND m.attachment = 1) \
                     OR (c.kind = 'text/calendar' AND (i.collection, i.seq) IN ({attached})))"
                )
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
                         WHERE s.collection = i.collection AND s.link_id = i.link_id)"
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
                     WHERE s.collection = i.collection AND s.link_id = i.link_id \
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
                         WHERE s.collection = i.collection AND s.link_id = i.link_id AND {band})"
                )
            }
            PimdirQueryPredicate::Category(category) => {
                // NOTE: both sides folded as STORAGE Annex A.2 folds `fn`,
                // the stored side by the function the connection registers.
                let category = self.bind(fold(category).into());
                format!(
                    "SELECT i.collection, i.seq {live} AND EXISTS (SELECT 1 FROM store.task_summary s \
                         JOIN json_each(s.categories) j \
                         WHERE s.collection = i.collection AND s.link_id = i.link_id \
                         AND {FOLD}(j.value) = {category})"
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
                let (since, until) = bounds(range, self.now);
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
                let undated = self.bind(serde_json::to_string(undated).unwrap_or_default().into());
                format!(
                    "SELECT i.collection, i.seq {live} AND ((c.kind = 'message/rfc822' \
                     OR (c.kind = 'text/calendar' AND i.object_hash IS NULL)) \
                     AND i.sort_key != '' AND ({since} IS NULL OR i.sort_key >= {since}) \
                     AND ({until} IS NULL OR i.sort_key < {until})) \
                     UNION SELECT collection, seq FROM ({occurring}) \
                     UNION SELECT i.collection, i.seq {live} \
                     AND c.kind IN (SELECT value FROM json_each({undated}))"
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
        let at = self.ctes.len();
        let contacts = match fts(Some("title"), text) {
            None => "SELECT NULL, NULL WHERE 0".to_string(),
            Some(expression) => {
                let objects = self.embed(
                    search::MATCH_OBJECTS,
                    vec![(":match", expression.clone().into())],
                );
                let summaries =
                    self.embed(search::MATCH_SUMMARIES, vec![(":match", expression.into())]);
                format!(
                    "SELECT m.collection, m.seq FROM ({objects}) m \
                     JOIN store.collections c ON c.id = m.collection WHERE c.kind = 'text/vcard' \
                     UNION SELECT m.collection, m.seq FROM ({summaries}) m \
                     JOIN store.collections c ON c.id = m.collection WHERE c.kind = 'text/vcard'"
                )
            }
        };
        self.ctes.push(format!(
            "p{at}(collection, seq) AS MATERIALIZED ({contacts})"
        ));

        format!(
            "SELECT collection, seq FROM p{at} UNION \
             SELECT i.collection, i.seq FROM store.item_address a \
             JOIN store.items i ON i.collection = a.collection AND i.link_id = a.link_id \
             WHERE i.deleted = 0 AND i.retained_at IS NULL AND a.address IN ( \
               SELECT e.address FROM store.item_address e \
               JOIN store.items ei ON ei.collection = e.collection AND ei.link_id = e.link_id \
               WHERE e.role = 'email' AND (ei.collection, ei.seq) IN (SELECT collection, seq FROM p{at}))"
        )
    }

    /// Compiles a text term to a `t<n>` expression of its matches over
    /// bodies and, when the field exists there, over summaries, answering
    /// the condition over `live`.
    fn text(
        &mut self,
        column: Option<&str>,
        summary: bool,
        text: &PimdirQueryText,
        negated: bool,
    ) -> String {
        let at = self.ctes.len();
        let cte = match fts(column, text) {
            None => "SELECT NULL, NULL, NULL, NULL WHERE 0".to_string(),
            Some(expression) => {
                let objects = self.embed(
                    search::MATCH_OBJECTS,
                    vec![(":match", expression.clone().into())],
                );
                let mut union = format!(
                    "SELECT ifnull(c.account, ''), m.seq, 'body', m.rank FROM ({objects}) m \
                     JOIN store.collections c ON c.id = m.collection"
                );
                if summary {
                    let summaries =
                        self.embed(search::MATCH_SUMMARIES, vec![(":match", expression.into())]);
                    union += &format!(
                        " UNION ALL SELECT ifnull(c.account, ''), m.seq, 'summary', m.rank \
                         FROM ({summaries}) m JOIN store.collections c ON c.id = m.collection"
                    );
                }
                union
            }
        };
        // NOTE: materialised, so `bm25` runs in the full-text query that
        // ranks; flattened into the statement reading `rank`, a lone
        // `match_objects` would call it outside one, which FTS5 refuses.
        self.ctes.push(format!(
            "t{at}(akey, seq, src, rank) AS MATERIALIZED ({cte})"
        ));

        if !negated {
            self.positive.push(at);
            self.words.push(text.text.clone());
        }

        member(&format!("SELECT akey, seq FROM t{at}"))
    }
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
   LEFT JOIN store.mail_summary ms ON ms.collection = i.collection AND ms.link_id = i.link_id
   LEFT JOIN store.contact_summary cs ON cs.collection = i.collection AND cs.link_id = i.link_id
   LEFT JOIN store.event_summary es ON es.collection = i.collection AND es.link_id = i.link_id
   LEFT JOIN store.task_summary ts ON ts.collection = i.collection AND ts.link_id = i.link_id
   LEFT JOIN store.journal_summary js ON js.collection = i.collection AND js.link_id = i.link_id
   LEFT JOIN store.file_summary fs ON fs.collection = i.collection AND fs.link_id = i.link_id";

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
/// month or year in UTC, `today` and `yesterday` theirs around `now`
/// (Unix seconds), and a relative amount the instant that long before
/// `now`, its span alone running from that instant to `now`.
///
/// A single point is parsed as a range from it to itself, so `2w..2w`
/// reads as `2w` alone.
pub(crate) fn bounds(range: &PimdirQueryDateRange, now: i64) -> (Option<String>, Option<String>) {
    let single = range.start.is_some() && range.start == range.end;
    let since = range.start.as_ref().map(|point| span(point, now).0);
    let until = range.end.as_ref().map(|point| match point {
        PimdirQueryDate::Relative { .. } if single => now,
        point => span(point, now).1,
    });
    (since.map(time::rfc3339), until.map(time::rfc3339))
}

/// The span of one date point, `[start, end)` in Unix seconds, a
/// relative amount spanning its instant alone; nothing before the first
/// instant of year 0, so an amount reaching past it is no wider.
fn span(point: &PimdirQueryDate, now: i64) -> (i64, i64) {
    let day = |year: u16, month: u8, day: u8| {
        time::days_from_civil(i32::from(year), u32::from(month), u32::from(day)) * DAY
    };
    let midnight = now.div_euclid(DAY) * DAY;

    match *point {
        PimdirQueryDate::Year(year) => (day(year, 1, 1), day(year + 1, 1, 1)),
        PimdirQueryDate::Month { year, month } => {
            let next = match month {
                12 => day(year + 1, 1, 1),
                month => day(year, month + 1, 1),
            };
            (day(year, month, 1), next)
        }
        PimdirQueryDate::Day {
            year,
            month,
            day: date,
        } => {
            let start = day(year, month, date);
            (start, start + DAY)
        }
        PimdirQueryDate::Today => (midnight, midnight + DAY),
        PimdirQueryDate::Yesterday => (midnight - DAY, midnight),
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

/// The FTS5 expression of a text term: the text as one escaped string, so
/// the tokenizer reads its words as a phrase and nothing as syntax, a
/// prefix starred, filtered to a column. `None` for a text holding no
/// word, which matches nothing.
fn fts(column: Option<&str>, text: &PimdirQueryText) -> Option<String> {
    if !text.text.chars().any(char::is_alphanumeric) {
        return None;
    }

    let mut expression = format!("\"{}\"", text.text.replace('"', "\"\""));
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

    #[test]
    fn a_category_folds_by_the_full_unicode_lowercase_mapping() {
        assert_eq!(fold(" ÉCOLE "), "école");
        assert_eq!(fold("İ"), "i\u{307}");
    }

    fn bounds_of(value: &str, now: i64) -> (Option<String>, Option<String>) {
        match PimdirQuery::parse(&format!("date:{value}")).unwrap() {
            PimdirQuery::Term(PimdirQueryTerm {
                predicate: PimdirQueryPredicate::Date(range),
                ..
            }) => bounds(&range, now),
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
}
