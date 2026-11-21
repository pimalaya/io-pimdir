//! # Compiler
//!
//! A [`PimdirPlan`] rendered as SQL over index.db with the store attached
//! (SEARCH §8). A hit is `(account, seq)`; each term is a condition over
//! a hit, holding when any live placement of it in the account does (the
//! union rule SEARCH §8.2 states for tags, kept for every term), and
//! `not` when none does. A term whose set a driver reads also renders
//! that set, its hits as `(akey, seq)` pairs, the account empty for none.
//!
//! A text term is an FTS5 `MATCH` built from escaped tokens, either read
//! once through the canonical `match_objects` and `match_summaries` into
//! `temp.term_match`, each hit then probed by a seek, or probed against
//! each candidate placement's body and summary, as the plan says.
//!
//! Every value is bound by name, the n-th `:vn`, so one fragment renders
//! into any statement; a statement's own values take other names.

use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use jiff::tz::TimeZone;
use rusqlite::types::Value;

#[cfg(test)]
use crate::search::query::PimdirQuery;
use crate::{
    search::{
        fold,
        plan::{
            self, CAP, PimdirPlan, PimdirPlanAccess, PimdirPlanEstimate, PimdirPlanNode,
            PimdirSearchSort, PimdirWalkStep,
        },
        query::{
            PimdirQueryDateRange, PimdirQueryId, PimdirQueryKind, PimdirQueryPredicate,
            PimdirQueryPriority, PimdirQueryProperty, PimdirQueryText, PimdirQueryTextField,
        },
    },
    sql::search,
    summary::time,
};

/// The items each `linked:` term names, as `(kind, link id)` pairs per
/// named id (STORAGE §14.2).
pub(crate) type Ends = Vec<(PimdirQueryId, Vec<(String, String)>)>;

/// The task categories each `category:` term names: its value folded as
/// STORAGE Annex A.2 folds `fn`, and every value a task states that folds
/// to it, as stated.
pub(crate) type Categories = Vec<(String, Vec<String>)>;

/// The media types of the undated kinds, as SQL.
const UNDATED: &str = "('text/vcard', 'application/octet-stream')";

/// The titles a hit's placements carry, joined for the `name` order.
const TITLES: &str = "FROM store.items i JOIN store.collections c ON c.id = i.collection
   LEFT JOIN store.mail_summary ms ON ms.item = i.id
   LEFT JOIN store.contact_summary cs ON cs.item = i.id
   LEFT JOIN store.event_summary es ON es.item = i.id
   LEFT JOIN store.task_summary ts ON ts.item = i.id
   LEFT JOIN store.journal_summary js ON js.item = i.id
   LEFT JOIN store.file_summary fs ON fs.item = i.id";

/// A compiled query: its plan, the fragments each of its terms renders
/// to, the values they bind, and what an answer reports beside its hits.
pub(crate) struct Compiled {
    /// The plan rendered.
    pub(crate) plan: PimdirPlan,
    /// How each leaf's text is tested, as the plan chose.
    pub(crate) access: Vec<PimdirPlanAccess>,
    /// Every value a fragment binds, the n-th named `:vn`.
    pub(crate) values: Vec<Value>,
    /// The positive text terms' expressions, each probed against a page's
    /// hits to tell what they matched (`match_object`, `match_summary`).
    pub(crate) probes: Vec<Probe>,
    /// The words a snippet looks for: the positive text terms'.
    pub(crate) words: Vec<String>,
    /// Each leaf rendered.
    leaves: Vec<Leaf>,
    /// The condition a collection `{c}` of the scope meets.
    scope: String,
}

/// One leaf rendered.
struct Leaf {
    /// The condition over the hit `({akey}, {seq})`, a text term's read
    /// from its matches in `temp.term_match`.
    hit: String,
    /// A text term's condition probing its expression against each live
    /// placement of the hit.
    probe: Option<String>,
    /// Its set, `SELECT akey, seq`, when a driver may read it.
    set: Option<String>,
    /// The statement counting its set up to [`CAP`].
    estimate: Option<String>,
    /// The statements reading its matches into `temp.term_match`.
    matches: Vec<String>,
}

/// A positive text term, probed against one hit's body or summary.
pub(crate) struct Probe {
    /// Its FTS5 expression (SEARCH §8.5).
    pub(crate) expression: String,
    /// Whether its field exists in a summary, which it then matches too.
    pub(crate) summary: bool,
}

impl Compiled {
    /// Renders a plan, `ends` holding the items each `linked:` term names
    /// and `categories` the task categories each `category:` term names.
    pub(crate) fn new(plan: PimdirPlan, ends: &Ends, categories: &Categories) -> Self {
        let mut compiled = Self {
            access: vec![PimdirPlanAccess::Materialize; plan.leaves.len()],
            values: Vec::new(),
            probes: Vec::new(),
            words: Vec::new(),
            leaves: Vec::new(),
            scope: String::new(),
            plan,
        };
        compiled.scope = compiled.render_scope();
        for at in 0..compiled.plan.leaves.len() {
            let leaf = compiled.leaf(at, ends, categories);
            compiled.leaves.push(leaf);
        }
        compiled
    }

    /// The statements reading leaf `at`'s matches into `temp.term_match`.
    pub(crate) fn matches(&self, at: usize) -> &[String] {
        &self.leaves[at].matches
    }

    /// The leaves whose matches a statement reads from `temp.term_match`
    /// under the current access: every text term read into a set, and
    /// every `person:`, whose contacts are.
    pub(crate) fn materialized(&self) -> Vec<usize> {
        (0..self.leaves.len())
            .filter(|at| {
                !self.leaves[*at].matches.is_empty()
                    && (self.leaves[*at].probe.is_none()
                        || self.access[*at] == PimdirPlanAccess::Materialize)
            })
            .collect()
    }

    /// The statement measuring leaf `at`'s set, `None` for a leaf of no
    /// set.
    pub(crate) fn estimate(&self, at: usize) -> Option<&str> {
        self.leaves[at].estimate.as_deref()
    }

    /// How many hits the union of the sets of `set` holds that have a
    /// live placement in the scope and meet the query.
    pub(crate) fn count_set(&self, set: &[usize]) -> String {
        format!(
            "SELECT count(*) FROM ({}) h WHERE {} AND {}",
            self.driven(set),
            self.placed("h.akey", "h.seq"),
            self.condition("h.akey", "h.seq", &self.skip(set)),
        )
    }

    /// How many hits the scope's live placements hold that meet the
    /// query, each counted at its representative placement.
    pub(crate) fn count_scope(&self) -> String {
        let rep = match self.plan.scope.is_one_collection() {
            true => String::new(),
            false => format!(" AND {}", self.representative("y", "c.account")),
        };
        format!(
            "SELECT count(*) FROM store.collections c \
             JOIN store.items y ON y.collection = c.id AND y.deleted = 0 \
             WHERE {}{rep} AND {}",
            self.at_scope("c"),
            self.condition("ifnull(c.account, '')", "y.seq", &self.skip(&[])),
        )
    }

    /// The hits of the union of the sets of `set`, `SELECT akey, seq`,
    /// each once.
    pub(crate) fn driven(&self, set: &[usize]) -> String {
        let sets: Vec<&str> = set
            .iter()
            .filter_map(|at| self.leaves[*at].set.as_deref())
            .collect();
        match sets.as_slice() {
            [one] => format!("SELECT DISTINCT akey, seq FROM ({one})"),
            many => many.join(" UNION "),
        }
    }

    /// Every hit with a live placement in the scope, `SELECT akey, seq`,
    /// each once.
    pub(crate) fn scoped(&self) -> String {
        format!(
            "SELECT DISTINCT ifnull(c.account, '') AS akey, i.seq AS seq FROM store.collections c \
             JOIN store.items i ON i.collection = c.id AND i.deleted = 0 WHERE {}",
            self.at_scope("c")
        )
    }

    /// The hits among `hits` (`SELECT akey, seq`, read from the sets of
    /// `set`) meeting the query, each with its place in `sort` (SEARCH
    /// §8.2): akey, seq, the greatest sort key of its live placements in
    /// the scope, whether undated, its rank and its title. With `page`,
    /// a dated order's page: those after the cursor named
    /// `:after_undated`, `:after_key`, `:after_seq` and `:after_account`
    /// when `page` holds true, ordered and cut at `:limit`.
    pub(crate) fn rows(
        &self,
        hits: &str,
        set: &[usize],
        sort: PimdirSearchSort,
        page: Option<bool>,
    ) -> String {
        let rank = match sort {
            PimdirSearchSort::Relevance => {
                let positive: Vec<String> = self
                    .plan
                    .leaves
                    .iter()
                    .enumerate()
                    .filter(|(_, leaf)| leaf.is_text() && !leaf.negated)
                    .map(|(at, _)| at.to_string())
                    .collect();
                format!(
                    "(SELECT min(m.rank) FROM temp.term_match m WHERE m.term IN ({}) \
                     AND m.akey = h.akey AND m.seq = h.seq)",
                    positive.join(", ")
                )
            }
            _ => String::from("NULL"),
        };
        let title = match sort {
            PimdirSearchSort::Title => format!(
                "(SELECT coalesce(ms.subject, cs.fn, es.summary, ts.summary, js.summary, fs.name, '') \
                 {TITLES} WHERE i.seq = h.seq AND ifnull(c.account, '') = h.akey AND i.deleted = 0 \
                 ORDER BY i.collection LIMIT 1)"
            ),
            PimdirSearchSort::Sender => String::from(
                "(SELECT coalesce(ms.sender, '') FROM store.items i \
                 JOIN store.collections c ON c.id = i.collection \
                 LEFT JOIN store.mail_summary ms ON ms.item = i.id \
                 WHERE i.seq = h.seq AND ifnull(c.account, '') = h.akey AND i.deleted = 0 \
                 ORDER BY i.collection LIMIT 1)",
            ),
            _ => String::from("''"),
        };
        let mut sql = format!(
            "SELECT h.akey, h.seq, max(s.sort_key) AS k, max(sc.kind IN {UNDATED}) AS u, \
             {rank} AS rank, {title} AS title \
             FROM ({hits}) h \
             JOIN store.items s ON s.seq = h.seq AND s.deleted = 0 \
             JOIN store.collections sc ON sc.id = s.collection \
             AND ifnull(sc.account, '') = h.akey AND {} \
             WHERE {} GROUP BY h.akey, h.seq",
            self.at_scope("sc"),
            self.condition("h.akey", "h.seq", &self.skip(set)),
        );
        if let Some(after) = page {
            let newest = sort == PimdirSearchSort::NewestFirst;
            if after {
                let beyond = match newest {
                    true => "CASE WHEN u = 0 THEN k < :after_key ELSE k > :after_key END",
                    false => "k > :after_key",
                };
                sql += &format!(
                    " HAVING u > :after_undated OR (u = :after_undated AND ({beyond} \
                     OR (k = :after_key AND (h.seq > :after_seq \
                     OR (h.seq = :after_seq AND h.akey > :after_account)))))"
                );
            }
            sql += match newest {
                true => {
                    " ORDER BY u, CASE WHEN u = 0 THEN k END DESC, CASE WHEN u = 1 THEN k END, \
                     h.seq, h.akey LIMIT :limit"
                }
                false => " ORDER BY u, k, h.seq, h.akey LIMIT :limit",
            };
        }
        sql
    }

    /// Every collection of the scope with the head a walk starts it
    /// from: id, account (empty for none), whether undated, and the key
    /// of its first live row in its phase's direction, at or beyond
    /// `:dated_key` or `:undated_key` where the phase is bounded. Each
    /// phase is `Some((ascending, bounded))`, `None` for one the walk
    /// skips.
    pub(crate) fn heads(
        &self,
        dated: Option<(bool, bool)>,
        undated: Option<(bool, bool)>,
    ) -> String {
        let head = |phase: Option<(bool, bool)>, bound: &str| match phase {
            None => String::from("NULL"),
            Some((ascending, bounded)) => {
                let (order, compare) = match ascending {
                    true => ("ASC", ">="),
                    false => ("DESC", "<="),
                };
                let bound = match bounded {
                    true => format!(" AND y.sort_key {compare} {bound}"),
                    false => String::new(),
                };
                format!(
                    "(SELECT y.sort_key FROM store.items y WHERE y.collection = c.id \
                     AND y.deleted = 0{bound} ORDER BY y.sort_key {order} LIMIT 1)"
                )
            }
        };
        format!(
            "SELECT c.id, ifnull(c.account, ''), c.kind IN {UNDATED}, \
             CASE WHEN c.kind IN {UNDATED} THEN {} ELSE {} END \
             FROM store.collections c WHERE {} ORDER BY c.id",
            head(undated, ":undated_key"),
            head(dated, ":dated_key"),
            self.at_scope("c"),
        )
    }

    /// The rows one walk step examines: those of collection
    /// `:collection`, of account `:account` (`NULL` for none) and akey
    /// `:akey`, after `(:key, :seq)` and within `:floor` where `step`
    /// holds them, at most `:rows`, each with whether it places a hit.
    pub(crate) fn step(&self, step: &PimdirWalkStep) -> String {
        let (order, beyond, within) = match step.ascending {
            true => ("ASC", ">", "<="),
            false => ("DESC", "<", ">="),
        };
        let mut bounds = String::new();
        if step.after.is_some() {
            bounds += &format!(
                " AND y.sort_key {beyond}= :key AND (y.sort_key {beyond} :key OR y.seq > :seq)"
            );
        }
        if step.floor.is_some() {
            bounds += &format!(" AND y.sort_key {within} :floor");
        }
        let rep = match self.plan.scope.is_one_collection() {
            true => String::from("1"),
            false => self.representative("y", ":account"),
        };
        format!(
            "SELECT y.sort_key, y.seq, CASE WHEN {rep} THEN {} ELSE 0 END \
             FROM store.items y INDEXED BY items_by_sort \
             WHERE y.collection = :collection AND y.deleted = 0{bounds} \
             ORDER BY y.sort_key {order}, y.seq LIMIT :rows",
            self.condition(":akey", "y.seq", &self.skip(&[])),
        )
    }

    /// How many collections the scope holds.
    pub(crate) fn collections(&self) -> String {
        format!(
            "SELECT count(*) FROM store.collections c WHERE {}",
            self.at_scope("c")
        )
    }

    /// The condition a hit `(akey, seq)` meets, the leaves of `skip`
    /// taken as holding.
    pub(crate) fn condition(&self, akey: &str, seq: &str, skip: &[usize]) -> String {
        self.node(&self.plan.root, akey, seq, skip)
    }

    /// The leaves a statement over the scope's placements, reading the
    /// sets of `set`, need not probe: those the scope implies and those
    /// of the sets read.
    pub(crate) fn skip(&self, set: &[usize]) -> Vec<usize> {
        self.plan.scoped.iter().chain(set).copied().collect()
    }

    /// Renders one node over the hit `(akey, seq)`.
    fn node(&self, node: &PimdirPlanNode, akey: &str, seq: &str, skip: &[usize]) -> String {
        match node {
            PimdirPlanNode::And(operands) | PimdirPlanNode::Or(operands) => {
                let operator = match node {
                    PimdirPlanNode::And(_) => " AND ",
                    _ => " OR ",
                };
                let rendered: Vec<String> = operands
                    .iter()
                    .map(|operand| self.node(operand, akey, seq, skip))
                    .collect();
                match rendered.is_empty() {
                    true => String::from("1"),
                    false => format!("({})", rendered.join(operator)),
                }
            }
            PimdirPlanNode::Not(operand) => {
                format!("(NOT {})", self.node(operand, akey, seq, skip))
            }
            PimdirPlanNode::Leaf(at) if skip.contains(at) => String::from("1"),
            PimdirPlanNode::Leaf(at) => {
                let leaf = &self.leaves[*at];
                let template = match (&leaf.probe, self.access[*at]) {
                    (Some(probe), PimdirPlanAccess::Probe) => probe,
                    _ => &leaf.hit,
                };
                template.replace("{akey}", akey).replace("{seq}", seq)
            }
        }
    }

    /// The condition that the hit `(akey, seq)` has a live placement in
    /// the scope.
    fn placed(&self, akey: &str, seq: &str) -> String {
        format!(
            "EXISTS (SELECT 1 FROM store.items s JOIN store.collections sc ON sc.id = s.collection \
             WHERE s.seq = {seq} AND s.deleted = 0 AND ifnull(sc.account, '') = {akey} AND {})",
            self.at_scope("sc")
        )
    }

    /// The condition that placement `y`, of account `account`, is its
    /// hit's representative in the scope: no other live placement of it
    /// there holds a greater key, nor an equal one in an earlier
    /// collection.
    fn representative(&self, y: &str, account: &str) -> String {
        format!(
            "NOT EXISTS (SELECT 1 FROM store.items r \
             JOIN store.collections rc ON rc.id = r.collection \
             WHERE r.seq = {y}.seq AND r.deleted = 0 AND r.id <> {y}.id \
             AND rc.account IS {account} AND {} AND (r.sort_key > {y}.sort_key \
             OR (r.sort_key = {y}.sort_key AND r.collection < {y}.collection)))",
            self.at_scope("rc")
        )
    }

    /// The scope's condition over collection `c`.
    fn at_scope(&self, c: &str) -> String {
        self.scope.replace("{c}", c)
    }

    /// Binds one value, answering its name.
    fn bind(&mut self, value: impl Into<Value>) -> String {
        self.values.push(value.into());
        format!(":v{}", self.values.len())
    }

    /// Embeds a canonical statement as a subquery, its comments and final
    /// semicolon stripped, each named parameter bound.
    fn embed(&mut self, statement: &str, binds: Vec<(&str, Value)>) -> String {
        let mut text: String = statement
            .lines()
            .filter(|line| !line.trim_start().starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n");
        text = text.trim().trim_end_matches(';').to_string();
        for (name, value) in binds {
            let named = self.bind(value);
            text = replace_param(&text, name, &named);
        }
        text
    }

    /// The scope's condition over a collection `{c}`: its kinds as
    /// constants, its accounts and ids bound.
    fn render_scope(&mut self) -> String {
        let scope = self.plan.scope.clone();
        let mut parts = Vec::new();
        if let Some(kinds) = &scope.kinds {
            let kinds: Vec<String> = kinds.iter().map(|kind| format!("'{kind}'")).collect();
            parts.push(match kinds.is_empty() {
                true => String::from("0"),
                false => format!("{{c}}.kind IN ({})", kinds.join(", ")),
            });
        }
        if let Some(accounts) = scope.accounts {
            let named: Vec<String> = accounts.into_iter().map(|a| self.bind(a)).collect();
            parts.push(match named.is_empty() {
                true => String::from("0"),
                false => format!("{{c}}.account IN ({})", named.join(", ")),
            });
        }
        if let Some(ids) = scope.collections {
            let named: Vec<String> = ids.into_iter().map(|id| self.bind(id)).collect();
            parts.push(match named.is_empty() {
                true => String::from("0"),
                false => format!("{{c}}.id IN ({})", named.join(", ")),
            });
        }
        match parts.is_empty() {
            true => String::from("1"),
            false => format!("({})", parts.join(" AND ")),
        }
    }

    /// Renders one leaf: its condition over a hit, as a placement
    /// predicate lifted to the hit's live placements in its account where
    /// the term is one, and its set and estimate where it has them.
    ///
    /// `thread:` reaches the members of the thread any message of it
    /// names, so a thread id still reaches them after a join renamed it
    /// (SEARCH §9). `has:attachment` reads a calendar item by the mark its
    /// body's extraction recorded, and `has:phone` and the others a card
    /// by theirs (SEARCH §6). `priority:` reads a column a store whose
    /// owner predates it holds `NULL` (STORAGE §6), every task then of no
    /// priority. `category:` folds both sides as STORAGE Annex A.2 folds
    /// `fn`, in Rust: a task by the `task_category` values the caller found
    /// folding to the term's, compared as stated, a card by the folded
    /// values its extraction recorded. A date term's open side is the
    /// earliest or the latest instant an RFC 3339 string spells, and it
    /// holds every item of an undated kind a `kind:` term names (SEARCH
    /// §8.1). An `id:` value of digits names the item of that seq and every
    /// item whose link id it is (SEARCH §8).
    fn leaf(&mut self, at: usize, ends: &Ends, categories: &Categories) -> Leaf {
        let predicate = self.plan.leaves[at].predicate.clone();
        let negated = self.plan.leaves[at].negated;
        let dates = self.plan.leaves[at].dates;
        let mut leaf = Leaf {
            hit: String::new(),
            probe: None,
            set: None,
            estimate: None,
            matches: Vec::new(),
        };
        let (j, c) = (format!("j{at}"), format!("c{at}"));

        let placement: String = match &predicate {
            PimdirQueryPredicate::Text { field, text } => {
                let (column, summary) = match field {
                    PimdirQueryTextField::Any => (None, true),
                    PimdirQueryTextField::Title => (Some("title"), true),
                    PimdirQueryTextField::Body => (Some("body"), false),
                    PimdirQueryTextField::Attachment => (Some("attachment"), false),
                    PimdirQueryTextField::Place => (Some("place"), false),
                };
                self.text(&mut leaf, at, column, summary, text, negated);
                return leaf;
            }
            PimdirQueryPredicate::Address { role, text } if !text.text.contains('@') => {
                let column = role.map(|role| role.as_str()).unwrap_or("people");
                self.text(&mut leaf, at, Some(column), true, text, negated);
                return leaf;
            }
            PimdirQueryPredicate::Address { role, text } => {
                let address = text.text.to_lowercase();
                let role = role.map(|role| self.bind(role.as_str().to_string()));
                let roled = |alias: &str| match &role {
                    Some(role) => format!(" AND {alias}.role = {role}"),
                    None => String::new(),
                };
                match address.strip_prefix('@') {
                    Some(domain) => {
                        let domain = self.bind(domain.to_string());
                        format!(
                            "EXISTS (SELECT 1 FROM store.item_address a{at} \
                             WHERE a{at}.item = {j}.id AND lower(substr(a{at}.address, \
                             -length({domain}) - 1)) = '@' || lower({domain}){})",
                            roled(&format!("a{at}"))
                        )
                    }
                    None => {
                        let address = self.bind(address);
                        leaf.set = Some(format!(
                            "SELECT ifnull(c.account, '') AS akey, i.seq AS seq \
                             FROM store.item_address a \
                             JOIN store.items i ON i.id = a.item AND i.deleted = 0 \
                             JOIN store.collections c ON c.id = i.collection \
                             WHERE a.address = {address}{}",
                            roled("a")
                        ));
                        leaf.estimate = Some(format!(
                            "SELECT count(*) FROM (SELECT 1 FROM store.item_address a \
                             WHERE a.address = {address}{} LIMIT {CAP})",
                            roled("a")
                        ));
                        format!(
                            "EXISTS (SELECT 1 FROM store.item_address a{at} \
                             WHERE a{at}.item = {j}.id AND a{at}.address = {address}{})",
                            roled(&format!("a{at}"))
                        )
                    }
                }
            }
            PimdirQueryPredicate::Person(text) => {
                if let Some(expression) = fts(Some("title"), text) {
                    let card = Some("text/vcard");
                    self.read_matches(
                        &mut leaf,
                        at,
                        search::MATCH_OBJECTS,
                        "body",
                        &expression,
                        card,
                    );
                    self.read_matches(
                        &mut leaf,
                        at,
                        search::MATCH_SUMMARIES,
                        "summary",
                        &expression,
                        card,
                    );
                }
                let named = format!(
                    "EXISTS (SELECT 1 FROM store.item_address a{at} WHERE a{at}.item = {j}.id \
                     AND a{at}.address IN (SELECT e.address FROM store.item_address e \
                     JOIN store.items ei ON ei.id = e.item WHERE e.role = 'email' \
                     AND (ei.collection, ei.seq) IN (SELECT t.collection, t.seq \
                     FROM temp.term_match t WHERE t.term = {at})))"
                );
                leaf.hit = format!("({} OR {})", matched(at), has(at, &named));
                return leaf;
            }
            PimdirQueryPredicate::Thread(id) => {
                let thread = self.embed(search::THREAD_OF, vec![(":link_id", id.clone().into())]);
                let members = format!(
                    "SELECT p.collection, p.seq FROM message m \
                     JOIN placement p ON p.link_id = m.link_id \
                     JOIN store.collections c ON c.id = p.collection \
                     WHERE c.kind = 'message/rfc822' AND m.thread = ({thread})"
                );
                small(&mut leaf, &members, &j)
            }
            PimdirQueryPredicate::Linked { role, id } => {
                let ends = ends
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
                small(&mut leaf, &linked.join(" UNION "), &j)
            }
            PimdirQueryPredicate::Flag(flag) => {
                let flag = self.bind(flag.clone());
                leaf.set = Some(format!(
                    "SELECT ifnull(c.account, '') AS akey, f.seq AS seq FROM flag f \
                     JOIN store.collections c ON c.id = f.collection WHERE f.flag = {flag}"
                ));
                leaf.estimate = Some(format!(
                    "SELECT count(*) FROM (SELECT 1 FROM flag WHERE flag = {flag} LIMIT {CAP})"
                ));
                format!(
                    "EXISTS (SELECT 1 FROM flag f{at} WHERE f{at}.flag = {flag} \
                     AND f{at}.collection = {j}.collection AND f{at}.seq = {j}.seq)"
                )
            }
            PimdirQueryPredicate::Encrypted => {
                leaf.set = Some(String::from(
                    "SELECT ifnull(c.account, '') AS akey, p.seq AS seq FROM object o \
                     JOIN placement p ON p.hash = o.hash \
                     JOIN store.collections c ON c.id = p.collection \
                     WHERE o.status = 'encrypted'",
                ));
                object(at, &j, &format!("o{at}.status = 'encrypted'"))
            }
            PimdirQueryPredicate::HasBody => format!("{j}.object_hash IS NOT NULL"),
            PimdirQueryPredicate::HasAttachment => format!(
                "(EXISTS (SELECT 1 FROM store.mail_summary m{at} WHERE m{at}.item = {j}.id \
                 AND m{at}.attachment = 1) OR ({c}.kind = 'text/calendar' AND {}))",
                object(at, &j, &format!("o{at}.attachment = 1"))
            ),
            PimdirQueryPredicate::HasProperty(property) => {
                let column = match property {
                    PimdirQueryProperty::Phone => "tel",
                    PimdirQueryProperty::Address => "adr",
                    PimdirQueryProperty::Birthday => "bday",
                    PimdirQueryProperty::Photo => "photo",
                };
                object(at, &j, &format!("o{at}.{column} = 1"))
            }
            PimdirQueryPredicate::Kind(kind) => match kind {
                PimdirQueryKind::Event | PimdirQueryKind::Task | PimdirQueryKind::Journal => {
                    let table = match kind {
                        PimdirQueryKind::Event => "event_summary",
                        PimdirQueryKind::Task => "task_summary",
                        _ => "journal_summary",
                    };
                    format!("EXISTS (SELECT 1 FROM store.{table} s{at} WHERE s{at}.item = {j}.id)")
                }
                kind => format!("{c}.kind = '{}'", plan::media_type(*kind)),
            },
            PimdirQueryPredicate::Status(status) => {
                let status = self.bind(status.clone());
                format!(
                    "EXISTS (SELECT 1 FROM store.task_summary s{at} WHERE s{at}.item = {j}.id \
                     AND s{at}.status = {status})"
                )
            }
            PimdirQueryPredicate::Mid(id) => {
                let id = self.bind(id.clone());
                format!(
                    "EXISTS (SELECT 1 FROM store.mail_summary s{at} WHERE s{at}.item = {j}.id \
                     AND s{at}.message_id = {id})"
                )
            }
            PimdirQueryPredicate::Priority(priority) => {
                let band = match priority {
                    PimdirQueryPriority::High => format!("s{at}.priority BETWEEN 1 AND 4"),
                    PimdirQueryPriority::Medium => format!("s{at}.priority = 5"),
                    PimdirQueryPriority::Low => format!("s{at}.priority BETWEEN 6 AND 9"),
                    PimdirQueryPriority::None => format!("ifnull(s{at}.priority, 0) = 0"),
                };
                format!(
                    "EXISTS (SELECT 1 FROM store.task_summary s{at} WHERE s{at}.item = {j}.id \
                     AND {band})"
                )
            }
            PimdirQueryPredicate::Category(category) => {
                let folded = fold::value(category);
                let stated: Vec<String> = categories
                    .iter()
                    .find(|(term, _)| *term == folded)
                    .map(|(_, values)| values.clone())
                    .unwrap_or_default();
                let folded = self.bind(folded);
                let stated = match stated.is_empty() {
                    true => String::from("0"),
                    false => {
                        let values: Vec<String> =
                            stated.into_iter().map(|value| self.bind(value)).collect();
                        format!("s{at}.value IN ({})", values.join(", "))
                    }
                };
                format!(
                    "(EXISTS (SELECT 1 FROM store.task_category s{at} WHERE s{at}.item = {j}.id \
                     AND {stated}) OR EXISTS (SELECT 1 FROM placement p{at} \
                     JOIN object o{at} ON o{at}.hash = p{at}.hash \
                     JOIN object_category k{at} ON k{at}.object = o{at}.id \
                     WHERE p{at}.collection = {j}.collection AND p{at}.seq = {j}.seq \
                     AND k{at}.value = {folded}))"
                )
            }
            PimdirQueryPredicate::Account(account) => {
                leaf.hit = format!("({{akey}} = {})", self.bind(account.clone()));
                return leaf;
            }
            PimdirQueryPredicate::Collection(collection) => {
                let collection = self.bind(collection.clone());
                leaf.set = Some(format!(
                    "SELECT ifnull(c.account, '') AS akey, i.seq AS seq FROM store.items i \
                     JOIN store.collections c ON c.id = i.collection \
                     WHERE i.collection = {collection} AND i.deleted = 0"
                ));
                leaf.estimate = Some(format!(
                    "SELECT count(*) FROM (SELECT 1 FROM store.items \
                     WHERE collection = {collection} AND deleted = 0 LIMIT {CAP})"
                ));
                format!("{j}.collection = {collection}")
            }
            PimdirQueryPredicate::Date(_) => self.date(&mut leaf, at, dates.unwrap_or_default()),
            PimdirQueryPredicate::Changed { since, until } => {
                let mut range = String::new();
                if let Some(since) = since {
                    range += &format!(" AND {{x}}.changed >= {}", self.bind(*since));
                }
                if let Some(until) = until {
                    range += &format!(" AND {{x}}.changed <= {}", self.bind(*until));
                }
                leaf.set = Some(format!(
                    "SELECT ifnull(c.account, '') AS akey, i.seq AS seq FROM store.items i \
                     JOIN store.collections c ON c.id = i.collection WHERE i.deleted = 0{}",
                    range.replace("{x}", "i")
                ));
                format!("1{}", range.replace("{x}", &j))
            }
            PimdirQueryPredicate::Id(id) => {
                let link = self.bind(id.as_str().to_string());
                let linked = has(at, &format!("{j}.link_id = {link}"));
                let seq = id.seq().map(|seq| self.bind(seq));
                let (hit, matching) = match &seq {
                    Some(seq) => (
                        format!("({{seq}} = {seq} OR {linked})"),
                        format!("(i.seq = {seq} OR i.link_id = {link})"),
                    ),
                    None => (linked, format!("i.link_id = {link}")),
                };
                leaf.set = Some(format!(
                    "SELECT ifnull(c.account, '') AS akey, i.seq AS seq FROM store.items i \
                     JOIN store.collections c ON c.id = i.collection \
                     WHERE i.deleted = 0 AND {matching}"
                ));
                leaf.hit = hit;
                self.estimated(&mut leaf, at);
                return leaf;
            }
        };

        leaf.hit = has(at, &placement);
        self.estimated(&mut leaf, at);
        leaf
    }

    /// Renders a date term spanning `[since, until)`: its condition over
    /// placement `j{at}`, and its set when the plan measures one.
    fn date(&mut self, leaf: &mut Leaf, at: usize, dates: (Option<i64>, Option<i64>)) -> String {
        let (j, c) = (format!("j{at}"), format!("c{at}"));
        let since = dates.0.map(time::rfc3339);
        let until = dates.1.map(time::rfc3339);
        let start = self.bind(since.clone().unwrap_or_default());
        let end = self.bind(
            until
                .clone()
                .unwrap_or_else(|| "9999-12-31T23:59:59Z".into()),
        );
        let mut range = String::new();
        if let Some(since) = since {
            range += &format!(" AND {{x}}.sort_key >= {}", self.bind(since));
        }
        if let Some(until) = until {
            range += &format!(" AND {{x}}.sort_key < {}", self.bind(until));
        }
        let dated = |x: &str, c: &str| {
            format!(
                "({c}.kind = 'message/rfc822' OR ({c}.kind = 'text/calendar' \
                 AND {x}.object_hash IS NULL)) AND {x}.sort_key != ''{}",
                range.replace("{x}", x)
            )
        };
        let occurring =
            |o: &str| format!("{o}.start < {end} AND ({o}.end > {start} OR {o}.start >= {start})");
        let undated = match self.plan.undated.is_empty() {
            true => String::new(),
            false => {
                let kinds: Vec<String> = self
                    .plan
                    .undated
                    .iter()
                    .map(|kind| format!("'{kind}'"))
                    .collect();
                format!(" OR {c}.kind IN ({})", kinds.join(", "))
            }
        };
        leaf.set = Some(format!(
            "SELECT ifnull(c.account, '') AS akey, i.seq AS seq FROM store.collections c \
             JOIN store.items i ON i.collection = c.id AND i.deleted = 0 WHERE {} \
             UNION ALL SELECT ifnull(c.account, '') AS akey, o.seq AS seq FROM occurrence o \
             JOIN store.collections c ON c.id = o.collection WHERE {}",
            dated("i", "c"),
            occurring("o"),
        ));
        format!(
            "(({}) OR EXISTS (SELECT 1 FROM occurrence o{at} \
             WHERE o{at}.collection = {j}.collection AND o{at}.seq = {j}.seq AND {}){undated})",
            dated(&j, &c),
            occurring(&format!("o{at}")),
        )
    }

    /// Gives a leaf the estimate of its set, counted up to [`CAP`], when
    /// the plan measures one, and drops its set when the plan reads none.
    fn estimated(&self, leaf: &mut Leaf, at: usize) {
        if self.plan.leaves[at].estimate == PimdirPlanEstimate::None {
            leaf.set = None;
            leaf.estimate = None;
            return;
        }
        if leaf.estimate.is_none()
            && let Some(set) = &leaf.set
        {
            leaf.estimate = Some(format!("SELECT count(*) FROM ({set} LIMIT {CAP})"));
        }
    }

    /// Renders a text term: its matches read into `temp.term_match` under
    /// the leaf's number from bodies and, when its field exists there,
    /// from summaries; the condition reading them, the one probing its
    /// expression per placement, its set and its estimate.
    fn text(
        &mut self,
        leaf: &mut Leaf,
        at: usize,
        column: Option<&str>,
        summary: bool,
        text: &PimdirQueryText,
        negated: bool,
    ) {
        let Some(expression) = fts(column, text) else {
            leaf.hit = String::from("0");
            leaf.probe = Some(String::from("0"));
            leaf.estimate = Some(String::from("SELECT 0"));
            leaf.set = Some(String::from("SELECT '' AS akey, 0 AS seq WHERE 0"));
            return;
        };
        self.read_matches(leaf, at, search::MATCH_OBJECTS, "body", &expression, None);
        if summary {
            self.read_matches(
                leaf,
                at,
                search::MATCH_SUMMARIES,
                "summary",
                &expression,
                None,
            );
        }
        if !negated {
            self.words.push(text.text.clone());
            self.probes.push(Probe {
                expression: expression.clone(),
                summary,
            });
        }

        let named = self.bind(expression);
        let (j, p) = (format!("j{at}"), format!("p{at}"));
        let summarised = match summary {
            true => format!(
                " OR EXISTS (SELECT 1 FROM summary_text WHERE summary_text MATCH {named} \
                 AND rowid = {p}.id)"
            ),
            false => String::new(),
        };
        let probed = format!(
            "EXISTS (SELECT 1 FROM placement {p} WHERE {p}.collection = {j}.collection \
             AND {p}.seq = {j}.seq AND (EXISTS (SELECT 1 FROM object o{at} \
             WHERE o{at}.hash = {p}.hash AND EXISTS (SELECT 1 FROM object_text \
             WHERE object_text MATCH {named} AND rowid = o{at}.id)){summarised}))"
        );
        leaf.hit = matched(at);
        leaf.probe = Some(has(at, &probed));
        leaf.set = Some(format!(
            "SELECT akey, seq FROM temp.term_match WHERE term = {at}"
        ));
        let counted = |table: &str| {
            format!(
                "(SELECT count(*) FROM (SELECT 1 FROM {table} WHERE {table} MATCH {named} \
                 LIMIT {CAP}))"
            )
        };
        leaf.estimate = Some(match summary {
            true => format!(
                "SELECT {} + {}",
                counted("object_text"),
                counted("summary_text")
            ),
            false => format!("SELECT {}", counted("object_text")),
        });
    }

    /// Gives a leaf the statement reading a text term's matches into
    /// `temp.term_match`, through `statement` (`match_objects` or
    /// `match_summaries`), what matched named `src`, those of collections
    /// of another kind than `kind` left out.
    fn read_matches(
        &mut self,
        leaf: &mut Leaf,
        at: usize,
        statement: &str,
        src: &str,
        expression: &str,
        kind: Option<&str>,
    ) {
        let matched = self.embed(statement, vec![(":match", expression.to_string().into())]);
        let kind = match kind {
            Some(kind) => format!(" WHERE c.kind = '{kind}'"),
            None => String::new(),
        };
        leaf.matches.push(format!(
            "INSERT INTO temp.term_match(term, collection, akey, seq, src, rank) \
             SELECT {at}, m.collection, ifnull(c.account, ''), m.seq, '{src}', m.rank \
             FROM ({matched}) m JOIN store.collections c ON c.id = m.collection{kind}"
        ));
    }
}

/// The condition that the hit `({akey}, {seq})` has a live placement
/// meeting `placement`, a condition over placement `j{at}` of collection
/// `c{at}`.
fn has(at: usize, placement: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM store.items j{at} \
         JOIN store.collections c{at} ON c{at}.id = j{at}.collection \
         WHERE j{at}.seq = {{seq}} AND j{at}.deleted = 0 \
         AND ifnull(c{at}.account, '') = {{akey}} AND {placement})"
    )
}

/// The condition that the hit `({akey}, {seq})` is among the matches of
/// text term `at` in `temp.term_match`.
fn matched(at: usize) -> String {
    format!(
        "EXISTS (SELECT 1 FROM temp.term_match t{at} WHERE t{at}.term = {at} \
         AND t{at}.akey = {{akey}} AND t{at}.seq = {{seq}})"
    )
}

/// The condition that placement `j` holds a body whose `object` row
/// `o{at}` meets `condition`.
fn object(at: usize, j: &str, condition: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM placement p{at} JOIN object o{at} ON o{at}.hash = p{at}.hash \
         WHERE p{at}.collection = {j}.collection AND p{at}.seq = {j}.seq AND {condition})"
    )
}

/// A small set of placements, `SELECT collection, seq`, given to a leaf
/// as its set, answering the condition over placement `j` that it holds
/// it.
fn small(leaf: &mut Leaf, placements: &str, j: &str) -> String {
    leaf.set = Some(format!(
        "SELECT ifnull(c.account, '') AS akey, x.seq AS seq FROM ({placements}) x \
         JOIN store.collections c ON c.id = x.collection"
    ));
    format!("({j}.collection, {j}.seq) IN ({placements})")
}

/// The collection media type of a kind.
pub(crate) fn media_type(kind: PimdirQueryKind) -> &'static str {
    plan::media_type(kind)
}

/// The instants a range spans as RFC 3339 in UTC, either side `None` open
/// ([`PimdirQueryDateRange::bounds`]).
pub(crate) fn bounds(
    range: &PimdirQueryDateRange,
    now: i64,
    zone: &TimeZone,
) -> (Option<String>, Option<String>) {
    let (since, until) = range.bounds(now, zone);
    (since.map(time::rfc3339), until.map(time::rfc3339))
}

/// The FTS5 expression of a text term: the text folded as the indexer
/// folds it (SEARCH §5) and written as one escaped string, so the
/// tokenizer reads its words as a phrase and nothing as syntax, a prefix
/// starred, filtered to a column. `None` for a text holding no word,
/// which matches nothing. U+0000 separates tokens as any character the
/// tokenizer keeps in none, and FTS5 reads no expression past one, so it
/// is written as a space (SEARCH §8.5).
fn fts(column: Option<&str>, text: &PimdirQueryText) -> Option<String> {
    if !text.text.chars().any(char::is_alphanumeric) {
        return None;
    }

    let folded = fold::text(&text.text).replace('\0', " ");
    let mut expression = format!("\"{}\"", folded.replace('"', "\"\""));
    if text.prefix {
        expression += " *";
    }
    Some(match column {
        Some(column) => format!("{{{column}}} : {expression}"),
        None => expression,
    })
}

/// Replaces a named parameter by another name wherever it stands whole,
/// so `:role` leaves `:roles` alone.
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
    }

    #[test]
    fn a_bound_no_four_digit_year_states_is_open() {
        let now = time::unix(2026, 8, 20, 12, 0, 0);
        let at = |instant: &str| Some(String::from(instant));

        assert_eq!(
            bounds_of("99999y", now),
            (None, at("2026-08-20T12:00:00Z")),
            "a start before year 0 is open"
        );
        assert_eq!(bounds_of("999999d", now).0, None);
        assert_eq!(bounds_of("999999w..2026", now).0, None);
        assert_eq!(
            bounds_of("2026y", now).0,
            at("0000-08-20T12:00:00Z"),
            "year 0 is stated"
        );
        assert_eq!(
            bounds_of("..999999d", now),
            (None, at("0000-01-01T00:00:00Z")),
            "an end before year 0 holds nothing"
        );

        assert_eq!(
            bounds_of("2026..9999", now),
            (at("2026-01-01T00:00:00Z"), None),
            "the year 9999 ends past the last instant four digits state"
        );
        assert_eq!(bounds_of("..9999-12", now), (None, None));
        assert_eq!(bounds_of("9999-12-31", now).1, None);
        assert_eq!(
            bounds_of("9999-12-30", now),
            (at("9999-12-30T00:00:00Z"), at("9999-12-31T00:00:00Z")),
            "the day before ends on an instant"
        );
        assert_eq!(
            bounds_in("0000-01-01", now, "Europe/Paris").0,
            None,
            "a day begun before year 0 in UTC"
        );
        assert_eq!(
            bounds_in("9999-12-31", now, "America/New_York").1,
            None,
            "a day ending in the year after 9999 in UTC"
        );
    }

    /// FTS5 reads no expression past U+0000, which the grammar admits in a
    /// term (SEARCH §8.5).
    #[test]
    fn a_nul_in_a_text_term_is_written_as_a_space() {
        let text = PimdirQueryText {
            text: "weekly\0sync \"x\"".into(),
            phrase: false,
            prefix: true,
        };
        assert_eq!(
            fts(Some("title"), &text).as_deref(),
            Some("{title} : \"weekly sync \"\"x\"\"\" *")
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

    #[test]
    fn a_local_time_a_transition_skips_or_repeats_reads_as_the_spec_says() {
        let span = |since: &str, until: &str| (Some(since.into()), Some(until.into()));
        let now = time::unix(2026, 8, 20, 12, 0, 0);

        assert_eq!(
            bounds_in("2027-03-28T02:30", now, "Europe/Paris"),
            span("2027-03-28T01:30:00Z", "2027-03-28T01:31:00Z"),
            "skipped: read at the offset before, 03:30 after the transition"
        );
        assert_eq!(
            bounds_in("2026-10-25T02:30", now, "Europe/Paris"),
            span("2026-10-25T00:30:00Z", "2026-10-25T00:31:00Z"),
            "repeated: its first occurrence, at +02:00"
        );
        assert_eq!(
            bounds_in("2026-10-25T02:30:15-05:00", now, "Europe/Paris"),
            span("2026-10-25T07:30:15Z", "2026-10-25T07:30:16Z"),
            "an offset stated wins over the zone, a second spans itself"
        );
    }

    #[test]
    fn a_month_back_lands_on_the_last_day_the_month_has() {
        let now = time::unix(2026, 3, 31, 12, 0, 0);
        assert_eq!(
            bounds_of("1mo", now).0.as_deref(),
            Some("2026-02-28T12:00:00Z")
        );
        assert_eq!(
            bounds_of("13mo", now).0.as_deref(),
            Some("2025-02-28T12:00:00Z")
        );
        assert_eq!(
            bounds_of("90min", now).0.as_deref(),
            Some("2026-03-31T10:30:00Z")
        );
    }
}
