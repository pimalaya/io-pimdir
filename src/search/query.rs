//! # Query
//!
//! The query language of SEARCH §8.3, parsed to an AST a consumer may
//! also build by hand (a client compiling its own filter into it), and
//! the structured refusal of SEARCH §8.4.
//!
//! Terms are whitespace-separated and conjoined; `and`, `or` and `not`
//! are operators in any case, and a `-` starting a term is `not`. `not`
//! binds tighter than `and`, explicit or by juxtaposition, which binds
//! tighter than `or`; parentheses group. A term is free text, a phrase or
//! `field:value`, the field being the text before the first colon. Outside
//! quotes every word holding a colon is a field term, so an unknown name
//! is refused rather than searched as text. A phrase is literal: nothing
//! inside it is an operator, a field or a wildcard. A trailing `*` after
//! two characters or more is a prefix wildcard on free text and on a text
//! field's value; anywhere else it is refused. Nothing here is FTS5
//! syntax: the compiler escapes every token.
//!
//! A refused query answers one fault, found in three passes: the phrases,
//! the parentheses, then the terms and operators in reading order. Offsets
//! are Unicode scalar values into the query, half open, so an agent can
//! point at the part it has to correct.

use alloc::{
    boxed::Box,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use core::fmt;

use crate::{reference::PimdirReferenceRole, summary::PimdirAddressRole};

/// The most parentheses and negations a query nests: this
/// implementation's bound, refused as `unbalanced-parenthesis` beyond.
pub const NESTING_MAX: usize = 64;

/// Every field of SEARCH §8's table, in its order.
pub const FIELDS: &[&str] = &[
    "subject",
    "title",
    "summary",
    "body",
    "from",
    "to",
    "cc",
    "bcc",
    "attendee",
    "organizer",
    "email",
    "with",
    "person",
    "tag",
    "is",
    "flag",
    "has",
    "kind",
    "status",
    "priority",
    "category",
    "account",
    "collection",
    "folder",
    "date",
    "when",
    "thread",
    "id",
    "linked",
    "attachment",
    "mimetype",
    "location",
    "changed",
];

/// The fields whose value is text, so takes a trailing wildcard unless it
/// holds an `@` (SEARCH §8.3).
const TEXT_FIELDS: &[&str] = &[
    "subject",
    "title",
    "summary",
    "body",
    "from",
    "to",
    "cc",
    "bcc",
    "attendee",
    "organizer",
    "email",
    "with",
    "person",
    "attachment",
    "mimetype",
    "location",
];

/// `is-value` (SEARCH §8.3).
const IS_VALUES: &[&str] = &[
    "unread",
    "read",
    "flagged",
    "answered",
    "draft",
    "encrypted",
];

/// `has-value` (SEARCH §8.3).
const HAS_VALUES: &[&str] = &["body", "attachment"];

/// `status-value` (SEARCH §8.3): RFC 5545's `VTODO` set, lowercased.
const STATUS_VALUES: &[&str] = &["needs-action", "in-process", "completed", "cancelled"];

/// `priority-value` (SEARCH §8.3).
const PRIORITY_VALUES: &[&str] = &["high", "medium", "low", "none"];

/// The forms of `date-value` (SEARCH §8.3).
const DATE_FORMS: &[&str] = &[
    "YYYY",
    "YYYY-MM",
    "YYYY-MM-DD",
    "today",
    "yesterday",
    "<n>d",
    "<n>w",
    "<n>y",
    "<point>..<point>",
    "<point>..",
    "..<point>",
];

/// The forms of `stamp-value` (SEARCH §8.3).
const STAMP_FORMS: &[&str] = &["<stamp>", "<stamp>..<stamp>", "<stamp>..", "..<stamp>"];

/// What starts a term.
const TERM_START: &[&str] = &["term", "\"phrase\"", "field:value", "(", "not", "-"];

/// A parsed query (SEARCH §8).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirQuery {
    /// Every operand matches.
    And(Vec<PimdirQuery>),
    /// Any operand matches.
    Or(Vec<PimdirQuery>),
    /// The operand does not match.
    Not(Box<PimdirQuery>),
    /// One term.
    Term(PimdirQueryTerm),
}

impl PimdirQuery {
    /// Parses a query, or refuses it with what to correct (SEARCH §8.4).
    pub fn parse(query: &str) -> Result<Self, PimdirQueryError> {
        let chars: Vec<char> = query.chars().collect();

        let tokens = lex(&chars)?;
        if tokens.is_empty() {
            return Err(PimdirQueryError::new(
                PimdirQueryErrorCode::EmptyQuery,
                "The query holds no term",
                0,
                chars.len(),
                TERM_START,
            ));
        }
        balance(&tokens)?;

        let mut parser = Parser {
            chars: &chars,
            tokens,
            at: 0,
        };
        parser.disjunction(None, 0)
    }

    /// Every term of the query, depth first, with whether an odd number
    /// of negations applies to it.
    pub fn terms(&self) -> Vec<(&PimdirQueryTerm, bool)> {
        let mut terms = Vec::new();
        let mut pending = vec![(self, false)];
        while let Some((node, negated)) = pending.pop() {
            match node {
                Self::And(operands) | Self::Or(operands) => {
                    pending.extend(operands.iter().rev().map(|op| (op, negated)));
                }
                Self::Not(operand) => pending.push((operand, !negated)),
                Self::Term(term) => terms.push((term, negated)),
            }
        }
        terms
    }
}

/// One term and where the query states it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirQueryTerm {
    /// What the term asks.
    pub predicate: PimdirQueryPredicate,
    /// The term's first offset, in Unicode scalar values.
    pub start: usize,
    /// The offset after its last.
    pub end: usize,
}

/// What one term asks (SEARCH §8).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirQueryPredicate {
    /// Text in a field: free text, `subject:`, `body:`, `attachment:`,
    /// `location:`.
    Text {
        /// The field searched.
        field: PimdirQueryTextField,
        /// The text.
        text: PimdirQueryText,
    },
    /// A person in one address role (`from:` and the others), or in any
    /// with `None` (`with:`): a seek on the address when the value holds
    /// an `@`, the role's text otherwise.
    Address {
        /// The role, `None` for any.
        role: Option<PimdirAddressRole>,
        /// The address, domain or name.
        text: PimdirQueryText,
    },
    /// Contacts whose name matches, expanded to their addresses
    /// (`person:`).
    Person(PimdirQueryText),
    /// A store flag one placement carries (`tag:`, `flag:`, `is:`).
    Flag(String),
    /// A body recorded `encrypted` (`is:encrypted`).
    Encrypted,
    /// A stored body (`has:body`).
    HasBody,
    /// An attachment (`has:attachment`).
    HasAttachment,
    /// One kind (`kind:`).
    Kind(PimdirQueryKind),
    /// A task's `STATUS`, uppercased as the summary holds it (`status:`).
    Status(String),
    /// A task's `PRIORITY` band (`priority:`).
    Priority(PimdirQueryPriority),
    /// One of a task's `CATEGORIES`, whole, as written (`category:`).
    Category(String),
    /// One account (`account:`).
    Account(String),
    /// One collection, by id or by name (`collection:`, `folder:`).
    Collection(String),
    /// A date range (`date:`, `when:`).
    Date(PimdirQueryDateRange),
    /// A range of change stamps (`changed:`), both ends included.
    Changed {
        /// The lowest stamp, `None` open.
        since: Option<i64>,
        /// The highest stamp, `None` open.
        until: Option<i64>,
    },
    /// One thread (`thread:`).
    Thread(String),
    /// One item (`id:`).
    Id(PimdirQueryId),
    /// The items at the other end of an item's references (`linked:`).
    Linked {
        /// The role narrowed to, `None` for any.
        role: Option<PimdirReferenceRole>,
        /// The item at this end.
        id: PimdirQueryId,
    },
}

/// The index field a text term searches (SEARCH §6).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirQueryTextField {
    /// Every field: free text.
    Any,
    /// The `title` field: `subject:`, `title:`, `summary:`.
    Title,
    /// The `body` field: `body:`.
    Body,
    /// The `attachment` field: `attachment:`, `mimetype:`.
    Attachment,
    /// The `place` field: `location:`.
    Place,
}

/// A text value: a word, maybe a prefix, or a quoted phrase.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirQueryText {
    /// The text, without its quotes or its wildcard.
    pub text: String,
    /// Whether it was quoted.
    pub phrase: bool,
    /// Whether a trailing `*` makes it a prefix.
    pub prefix: bool,
}

/// The bands `priority:` names (SEARCH §8, RFC 5545 §3.8.1.9).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirQueryPriority {
    /// `PRIORITY` 1 to 4.
    High,
    /// `PRIORITY` 5.
    Medium,
    /// `PRIORITY` 6 to 9.
    Low,
    /// No `PRIORITY`, or 0.
    None,
}

impl PimdirQueryPriority {
    /// The band a `priority-value` names.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "high" => Some(Self::High),
            "medium" => Some(Self::Medium),
            "low" => Some(Self::Low),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

/// The kinds `kind:` names (SEARCH §8).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirQueryKind {
    /// `message/rfc822`.
    Mail,
    /// `text/vcard`.
    Contact,
    /// A `text/calendar` `VEVENT`.
    Event,
    /// A `text/calendar` `VTODO`.
    Task,
    /// A `text/calendar` `VJOURNAL`.
    Journal,
    /// `application/octet-stream`.
    File,
}

impl PimdirQueryKind {
    /// Every kind, in the order `kind-value` lists them.
    pub const ALL: [Self; 6] = [
        Self::Mail,
        Self::Contact,
        Self::Event,
        Self::Task,
        Self::Journal,
        Self::File,
    ];

    /// The name `kind:` takes.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mail => "mail",
            Self::Contact => "contact",
            Self::Event => "event",
            Self::Task => "task",
            Self::Journal => "journal",
            Self::File => "file",
        }
    }

    /// The inverse of [`as_str`](Self::as_str).
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }

    /// Whether items of the kind carry a date a range compares: contacts
    /// and files do not (SEARCH §8.1).
    pub fn is_dated(&self) -> bool {
        !matches!(self, Self::Contact | Self::File)
    }
}

/// A date range (SEARCH §8.1): from the start of `start`'s span to the
/// end of `end`'s, so a single point is the range covering it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirQueryDateRange {
    /// The first point, `None` open.
    pub start: Option<PimdirQueryDate>,
    /// The last point, `None` open.
    pub end: Option<PimdirQueryDate>,
}

/// One point of a date range (SEARCH §8.1).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirQueryDate {
    /// A calendar year.
    Year(u16),
    /// A calendar month.
    Month {
        /// The year.
        year: u16,
        /// The month, 1 to 12.
        month: u8,
    },
    /// A calendar day.
    Day {
        /// The year.
        year: u16,
        /// The month, 1 to 12.
        month: u8,
        /// The day, 1 to the month's last.
        day: u8,
    },
    /// `today`: the calendar day holding the query's now.
    Today,
    /// `yesterday`: the calendar day before it.
    Yesterday,
    /// An amount back from now, `3d`, `2w`, `1y`: the instant that long
    /// before the query's now.
    Relative {
        /// The amount.
        amount: u32,
        /// The unit.
        unit: PimdirQueryDateUnit,
    },
}

/// The unit of a relative date (SEARCH §8.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirQueryDateUnit {
    /// `d`: days.
    Day,
    /// `w`: weeks of seven days.
    Week,
    /// `y`: calendar years, a 29 February landing on the 28th.
    Year,
}

impl PimdirQueryDate {
    /// Whether the point resolves against a now (keywords and relative
    /// amounts), rather than standing on its own.
    pub fn is_relative(&self) -> bool {
        matches!(self, Self::Today | Self::Yesterday | Self::Relative { .. })
    }

    /// The first day of an absolute point's span, `YYYY-MM-DD`.
    pub fn first_day(&self) -> Option<String> {
        match self {
            Self::Year(year) => Some(format!("{year:04}-01-01")),
            Self::Month { year, month } => Some(format!("{year:04}-{month:02}-01")),
            Self::Day { year, month, day } => Some(format!("{year:04}-{month:02}-{day:02}")),
            _ => None,
        }
    }

    /// The day after the last of an absolute point's span, `YYYY-MM-DD`.
    pub fn day_after(&self) -> Option<String> {
        match *self {
            Self::Year(year) => Some(format!("{:04}-01-01", year + 1)),
            Self::Day { year, month, day } if day < days_in(year, month) => {
                Some(format!("{year:04}-{month:02}-{:02}", day + 1))
            }
            Self::Day { year, month, .. } | Self::Month { year, month } => {
                let (year, month) = if month == 12 {
                    (year + 1, 1)
                } else {
                    (year, month + 1)
                };
                Some(format!("{year:04}-{month:02}-01"))
            }
            _ => None,
        }
    }
}

/// An item named by `id:` or `linked:` (SEARCH §8): a value of digits,
/// naming a public id and a link id at once, or any other link id.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirQueryId {
    /// A value of ASCII digits alone, as written: the item whose public
    /// id `seq` (STORAGE §9.1) it reads as, and every item whose link id
    /// it is, a hit set rather than a guess.
    Digits(String),
    /// Any other value: a link id.
    Link(String),
}

impl PimdirQueryId {
    fn parse(value: &str) -> Self {
        match !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
            true => Self::Digits(value.into()),
            false => Self::Link(value.into()),
        }
    }

    /// The public id a value of digits reads as, `None` for a link id or
    /// digits past the range of a `seq`.
    pub fn seq(&self) -> Option<i64> {
        match self {
            Self::Digits(digits) => digits.parse().ok(),
            Self::Link(_) => None,
        }
    }

    /// The value as written, the link id it names.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Digits(value) | Self::Link(value) => value,
        }
    }
}

/// A refused query (SEARCH §8.4): what is wrong, where, and what would be
/// accepted there.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirQueryError {
    /// What kind of fault it is.
    pub code: PimdirQueryErrorCode,
    /// The fault, as a sentence for a person.
    pub message: String,
    /// The first offset at fault, in Unicode scalar values.
    pub start: usize,
    /// The offset after the last at fault.
    pub end: usize,
    /// What would have been valid at `start`.
    pub expected: Vec<String>,
    /// Every field name, for a field refused.
    pub fields: Vec<String>,
}

impl PimdirQueryError {
    /// A refusal of `code` over `start..end`, expecting `expected`,
    /// listing every field when a field is what is refused.
    pub fn new(
        code: PimdirQueryErrorCode,
        message: impl Into<String>,
        start: usize,
        end: usize,
        expected: &[&str],
    ) -> Self {
        let fields = match code {
            PimdirQueryErrorCode::UnknownField => FIELDS.iter().map(ToString::to_string).collect(),
            _ => Vec::new(),
        };

        Self {
            code,
            message: message.into(),
            start,
            end,
            expected: expected.iter().map(ToString::to_string).collect(),
            fields,
        }
    }
}

impl fmt::Display for PimdirQueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at {}..{}: {}",
            self.code.as_str(),
            self.start,
            self.end,
            self.message
        )
    }
}

impl core::error::Error for PimdirQueryError {}

/// The faults a refusal names (SEARCH §8.4), each spelled as its code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PimdirQueryErrorCode {
    /// No term in the query, or in a group: `empty-query`.
    EmptyQuery,
    /// A field not in SEARCH §8's table: `unknown-field`.
    UnknownField,
    /// An empty value, or one out of the field's set: `invalid-value`.
    InvalidValue,
    /// A date value no `date-value` reads: `invalid-date`.
    InvalidDate,
    /// A quote never closed: `unterminated-phrase`.
    UnterminatedPhrase,
    /// A parenthesis never closed, or closing nothing:
    /// `unbalanced-parenthesis`.
    UnbalancedParenthesis,
    /// An operator missing a side: `dangling-operator`.
    DanglingOperator,
    /// A `*` other than a trailing prefix wildcard: `invalid-wildcard`.
    InvalidWildcard,
}

impl PimdirQueryErrorCode {
    /// The code as the refusal object spells it.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EmptyQuery => "empty-query",
            Self::UnknownField => "unknown-field",
            Self::InvalidValue => "invalid-value",
            Self::InvalidDate => "invalid-date",
            Self::UnterminatedPhrase => "unterminated-phrase",
            Self::UnbalancedParenthesis => "unbalanced-parenthesis",
            Self::DanglingOperator => "dangling-operator",
            Self::InvalidWildcard => "invalid-wildcard",
        }
    }
}

/// One lexed token and its span.
#[derive(Clone, Debug)]
struct Token {
    kind: TokenKind,
    start: usize,
    end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TokenKind {
    Open,
    Close,
    And,
    Or,
    /// `not`, or a `-` starting a term; `bare` for a `-` followed by
    /// whitespace, a `)` or the end, which negates nothing.
    Not {
        bare: bool,
    },
    /// A quoted phrase, its text between the quotes.
    Phrase(String),
    /// A word holding no colon.
    Text(String),
    /// A word holding a colon: the offset of the first one, and whether
    /// the value is the phrase right after it.
    Field {
        colon: usize,
        phrase: bool,
    },
}

/// The first pass: splits the query into tokens, longest first, refusing
/// a phrase never closed.
fn lex(chars: &[char]) -> Result<Vec<Token>, PimdirQueryError> {
    let unterminated = |start: usize| {
        PimdirQueryError::new(
            PimdirQueryErrorCode::UnterminatedPhrase,
            "This quote is never closed",
            start,
            chars.len(),
            &["\""],
        )
    };
    let close = |open: usize| {
        chars[open + 1..]
            .iter()
            .position(|c| *c == '"')
            .map(|len| open + 1 + len)
    };

    let mut tokens = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let start = at;
        let kind = match chars[at] {
            c if is_space(c) => {
                at += 1;
                continue;
            }
            '(' => TokenKind::Open,
            ')' => TokenKind::Close,
            '-' => TokenKind::Not {
                bare: chars
                    .get(at + 1)
                    .is_none_or(|next| is_space(*next) || *next == ')'),
            },
            '"' => {
                let end = close(at).ok_or_else(|| unterminated(at))?;
                at = end;
                TokenKind::Phrase(chars[start + 1..end].iter().collect())
            }
            _ => {
                while at + 1 < chars.len() && !is_separator(chars[at + 1]) {
                    at += 1;
                }
                let word: String = chars[start..=at].iter().collect();
                match chars[start..=at].iter().position(|c| *c == ':') {
                    Some(colon) => {
                        let colon = start + colon;
                        let phrase = colon == at && chars.get(at + 1) == Some(&'"');
                        if phrase {
                            at = close(at + 1).ok_or_else(|| unterminated(at + 1))?;
                        }
                        TokenKind::Field { colon, phrase }
                    }
                    None => match word.to_lowercase().as_str() {
                        "and" => TokenKind::And,
                        "or" => TokenKind::Or,
                        "not" => TokenKind::Not { bare: false },
                        _ => TokenKind::Text(word),
                    },
                }
            }
        };
        at += 1;
        tokens.push(Token {
            kind,
            start,
            end: at,
        });
    }

    Ok(tokens)
}

/// `ws` of SEARCH §8.3.
fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

/// Whether a character ends a word.
fn is_separator(c: char) -> bool {
    is_space(c) || matches!(c, '(' | ')' | '"')
}

/// The second pass: refuses the leftmost parenthesis closing nothing or
/// never closed.
fn balance(tokens: &[Token]) -> Result<(), PimdirQueryError> {
    let mut open = Vec::new();
    let mut faults = Vec::new();
    for token in tokens {
        match token.kind.clone() {
            TokenKind::Open => open.push(token.start),
            TokenKind::Close if open.pop().is_none() => faults.push(token.start),
            _ => (),
        }
    }

    let Some(first) = faults.into_iter().chain(open).min() else {
        return Ok(());
    };
    Err(PimdirQueryError::new(
        PimdirQueryErrorCode::UnbalancedParenthesis,
        "This parenthesis is never closed, or closes no group",
        first,
        first + 1,
        &[],
    ))
}

/// The third pass: a recursive descent, `or` over `and` over `not`, each
/// term checked as it is read.
struct Parser<'a> {
    chars: &'a [char],
    tokens: Vec<Token>,
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&TokenKind> {
        self.tokens.get(self.at).map(|token| &token.kind)
    }

    fn dangling(&self, at: usize) -> PimdirQueryError {
        let token = &self.tokens[at];
        PimdirQueryError::new(
            PimdirQueryErrorCode::DanglingOperator,
            "This operator is missing an operand",
            token.start,
            token.end,
            TERM_START,
        )
    }

    /// `disjunction`; `expected_by` is the operator a missing operand
    /// leaves dangling.
    fn disjunction(
        &mut self,
        expected_by: Option<usize>,
        depth: usize,
    ) -> Result<PimdirQuery, PimdirQueryError> {
        let mut operands = vec![self.conjunction(expected_by, depth)?];
        while let Some(TokenKind::Or) = self.peek() {
            self.at += 1;
            operands.push(self.conjunction(Some(self.at - 1), depth)?);
        }

        Ok(match operands.len() {
            1 => operands.pop().unwrap(),
            _ => PimdirQuery::Or(operands),
        })
    }

    fn conjunction(
        &mut self,
        expected_by: Option<usize>,
        depth: usize,
    ) -> Result<PimdirQuery, PimdirQueryError> {
        let mut operands = vec![self.negation(expected_by, depth)?];
        loop {
            match self.peek() {
                Some(TokenKind::And) => {
                    self.at += 1;
                    operands.push(self.negation(Some(self.at - 1), depth)?);
                }
                Some(
                    TokenKind::Open
                    | TokenKind::Phrase(_)
                    | TokenKind::Text(_)
                    | TokenKind::Field { .. }
                    | TokenKind::Not { .. },
                ) => operands.push(self.negation(None, depth)?),
                _ => break,
            }
        }

        Ok(match operands.len() {
            1 => operands.pop().unwrap(),
            _ => PimdirQuery::And(operands),
        })
    }

    fn negation(
        &mut self,
        expected_by: Option<usize>,
        depth: usize,
    ) -> Result<PimdirQuery, PimdirQueryError> {
        let mut first = None;
        let mut count = 0;
        while let Some(TokenKind::Not { bare }) = self.peek() {
            if *bare {
                return Err(self.dangling(self.at));
            }
            first = first.or(Some(self.at));
            count += 1;
            self.at += 1;
        }

        let mut node = self.primary(first.or(expected_by), depth + count)?;
        for _ in 0..count {
            node = PimdirQuery::Not(Box::new(node));
        }
        Ok(node)
    }

    fn primary(
        &mut self,
        expected_by: Option<usize>,
        depth: usize,
    ) -> Result<PimdirQuery, PimdirQueryError> {
        match self.peek() {
            None | Some(TokenKind::Close | TokenKind::And | TokenKind::Or) => {
                // NOTE: an empty query is refused before, so a missing
                // operand at the start of the query is an operator token.
                return Err(self.dangling(expected_by.unwrap_or(self.at)));
            }
            _ => (),
        }

        let token = self.tokens[self.at].clone();
        self.at += 1;
        if depth >= NESTING_MAX {
            return Err(PimdirQueryError::new(
                PimdirQueryErrorCode::UnbalancedParenthesis,
                format!("The query nests deeper than {NESTING_MAX} groups or negations"),
                token.start,
                token.end,
                &[],
            ));
        }

        match token.kind.clone() {
            TokenKind::Open => {
                if let Some(TokenKind::Close) = self.peek() {
                    let end = self.tokens[self.at].end;
                    return Err(PimdirQueryError::new(
                        PimdirQueryErrorCode::EmptyQuery,
                        "This group holds no term",
                        token.start,
                        end,
                        TERM_START,
                    ));
                }
                let inner = self.disjunction(None, depth + 1)?;
                // NOTE: the parentheses balance, so this is the `)`.
                self.at += 1;
                Ok(inner)
            }
            TokenKind::Phrase(text) => Ok(term(
                PimdirQueryPredicate::Text {
                    field: PimdirQueryTextField::Any,
                    text: PimdirQueryText {
                        text,
                        phrase: true,
                        prefix: false,
                    },
                },
                &token,
            )),
            TokenKind::Text(word) => {
                let (text, prefix) = wildcard(&word, token.start, true)?;
                Ok(term(
                    PimdirQueryPredicate::Text {
                        field: PimdirQueryTextField::Any,
                        text: PimdirQueryText {
                            text: text.into(),
                            phrase: false,
                            prefix,
                        },
                    },
                    &token,
                ))
            }
            TokenKind::Field { colon, phrase } => field(self.chars, &token, colon, phrase),
            // NOTE: operators are consumed above, never read as a primary.
            _ => unreachable!("a term token"),
        }
    }
}

/// A term over a token's span.
fn term(predicate: PimdirQueryPredicate, token: &Token) -> PimdirQuery {
    PimdirQuery::Term(PimdirQueryTerm {
        predicate,
        start: token.start,
        end: token.end,
    })
}

/// Checks a word's `*` (SEARCH §8.3): only as its last character after
/// two or more, and only where `allowed`. Answers the word without it and
/// whether it was there.
fn wildcard(word: &str, offset: usize, allowed: bool) -> Result<(&str, bool), PimdirQueryError> {
    let Some(star) = word.chars().position(|c| c == '*') else {
        return Ok((word, false));
    };

    let last = star + 1 == word.chars().count();
    if allowed && last && star >= 2 {
        return Ok((&word[..word.len() - 1], true));
    }
    Err(PimdirQueryError::new(
        PimdirQueryErrorCode::InvalidWildcard,
        match allowed {
            true => "A wildcard is the last character of a word, after two or more",
            false => "This value takes no wildcard",
        },
        offset + star,
        offset + star + 1,
        &[],
    ))
}

/// Reads a field term, checking its field, then an empty value, then the
/// wildcard, then the value (SEARCH §8.4).
fn field(
    chars: &[char],
    token: &Token,
    colon: usize,
    phrase: bool,
) -> Result<PimdirQuery, PimdirQueryError> {
    let name: String = chars[token.start..colon].iter().collect();
    let name = name.as_str();
    if !FIELDS.contains(&name) {
        let lower = name.to_lowercase();
        let message = match FIELDS.contains(&lower.as_str()) {
            true => format!("{name:?} is no field: did you mean {lower}:? Fields are lowercase"),
            false => format!("{name:?} is no field: quote the word to search it as text"),
        };
        return Err(PimdirQueryError::new(
            PimdirQueryErrorCode::UnknownField,
            message,
            token.start,
            colon + 1,
            &[],
        ));
    }

    let value_start = colon + 1;
    let value: String = match phrase {
        true => chars[colon + 2..token.end - 1].iter().collect(),
        false => chars[value_start..token.end].iter().collect(),
    };
    if value_start == token.end {
        return Err(PimdirQueryError::new(
            PimdirQueryErrorCode::InvalidValue,
            format!("{name}: takes a value right after its colon"),
            token.start,
            token.end,
            &["value"],
        ));
    }

    let text_field = TEXT_FIELDS.contains(&name);
    let (stem, prefix) = match phrase {
        true => (value.as_str(), false),
        false => wildcard(&value, value_start, text_field && !value.contains('@'))?,
    };
    let text = PimdirQueryText {
        text: stem.into(),
        phrase,
        prefix,
    };
    let invalid = |code, expected: &[&str]| {
        let message = match code {
            PimdirQueryErrorCode::InvalidDate => format!("{value:?} is no date or date range"),
            _ => format!("{name}: does not take {value:?}"),
        };
        PimdirQueryError::new(code, message, value_start, token.end, expected)
    };

    let predicate = match name {
        "subject" | "title" | "summary" => PimdirQueryPredicate::Text {
            field: PimdirQueryTextField::Title,
            text,
        },
        "body" => PimdirQueryPredicate::Text {
            field: PimdirQueryTextField::Body,
            text,
        },
        "attachment" | "mimetype" => PimdirQueryPredicate::Text {
            field: PimdirQueryTextField::Attachment,
            text,
        },
        "location" => PimdirQueryPredicate::Text {
            field: PimdirQueryTextField::Place,
            text,
        },
        "from" | "to" | "cc" | "bcc" | "email" | "organizer" | "attendee" => {
            PimdirQueryPredicate::Address {
                role: PimdirAddressRole::parse(name),
                text,
            }
        }
        "with" => PimdirQueryPredicate::Address { role: None, text },
        "person" => PimdirQueryPredicate::Person(text),
        "tag" | "flag" => PimdirQueryPredicate::Flag(value),
        "is" => match value.as_str() {
            "unread" => {
                let seen = term(PimdirQueryPredicate::Flag("\\Seen".into()), token);
                return Ok(PimdirQuery::Not(Box::new(seen)));
            }
            "read" => PimdirQueryPredicate::Flag("\\Seen".into()),
            "flagged" => PimdirQueryPredicate::Flag("\\Flagged".into()),
            "answered" => PimdirQueryPredicate::Flag("\\Answered".into()),
            "draft" => PimdirQueryPredicate::Flag("\\Draft".into()),
            "encrypted" => PimdirQueryPredicate::Encrypted,
            _ => return Err(invalid(PimdirQueryErrorCode::InvalidValue, IS_VALUES)),
        },
        "has" => match value.as_str() {
            "body" => PimdirQueryPredicate::HasBody,
            "attachment" => PimdirQueryPredicate::HasAttachment,
            _ => return Err(invalid(PimdirQueryErrorCode::InvalidValue, HAS_VALUES)),
        },
        "kind" => match PimdirQueryKind::parse(&value) {
            Some(kind) => PimdirQueryPredicate::Kind(kind),
            None => {
                let kinds: Vec<&str> = PimdirQueryKind::ALL.iter().map(|k| k.as_str()).collect();
                return Err(invalid(PimdirQueryErrorCode::InvalidValue, &kinds));
            }
        },
        "status" => match STATUS_VALUES.contains(&value.as_str()) {
            true => PimdirQueryPredicate::Status(value.to_uppercase()),
            false => return Err(invalid(PimdirQueryErrorCode::InvalidValue, STATUS_VALUES)),
        },
        "priority" => match PimdirQueryPriority::parse(&value) {
            Some(priority) => PimdirQueryPredicate::Priority(priority),
            None => return Err(invalid(PimdirQueryErrorCode::InvalidValue, PRIORITY_VALUES)),
        },
        "category" => PimdirQueryPredicate::Category(value),
        "account" => PimdirQueryPredicate::Account(value),
        "collection" | "folder" => PimdirQueryPredicate::Collection(value),
        "date" | "when" => match date_range(&value) {
            Some(range) => PimdirQueryPredicate::Date(range),
            None => return Err(invalid(PimdirQueryErrorCode::InvalidDate, DATE_FORMS)),
        },
        "changed" => match stamp_range(&value) {
            Some((since, until)) => PimdirQueryPredicate::Changed { since, until },
            None => return Err(invalid(PimdirQueryErrorCode::InvalidValue, STAMP_FORMS)),
        },
        "thread" => PimdirQueryPredicate::Thread(value),
        "id" => PimdirQueryPredicate::Id(PimdirQueryId::parse(&value)),
        "linked" => {
            let (role, id) = match value.split_once(':') {
                Some((role, id)) if !id.is_empty() && is_reference_role(role) => {
                    (Some(PimdirReferenceRole::parse(role)), id)
                }
                _ => (None, value.as_str()),
            };
            PimdirQueryPredicate::Linked {
                role,
                id: PimdirQueryId::parse(id),
            }
        }
        // NOTE: FIELDS lists exactly the names matched above.
        _ => unreachable!("{name} is a known field"),
    };

    Ok(term(predicate, token))
}

/// Whether `linked:` reads the text before a colon as a role: one pimdir
/// gives, or an application's `x-` one.
fn is_reference_role(role: &str) -> bool {
    matches!(
        role,
        "attachment" | "invitation" | "sender" | "parent" | "source" | "related"
    ) || role.strip_prefix("x-").is_some_and(|name| !name.is_empty())
}

/// Reads a range of `point`s: one, or two around `..` with either side
/// open but not both.
fn range<T>(value: &str, point: impl Fn(&str) -> Option<T>) -> Option<(Option<T>, Option<T>)>
where
    T: Clone,
{
    match value.split_once("..") {
        None => {
            let point = point(value)?;
            Some((Some(point.clone()), Some(point)))
        }
        Some(("", "")) => None,
        Some((start, end)) => {
            let side = |side: &str| match side {
                "" => Some(None),
                side => point(side).map(Some),
            };
            Some((side(start)?, side(end)?))
        }
    }
}

/// Reads a `date-value`.
fn date_range(value: &str) -> Option<PimdirQueryDateRange> {
    let (start, end) = range(value, date)?;
    Some(PimdirQueryDateRange { start, end })
}

/// Reads a `stamp-value`.
fn stamp_range(value: &str) -> Option<(Option<i64>, Option<i64>)> {
    range(value, |side| {
        side.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| side.parse().ok())
            .flatten()
    })
}

/// Reads a `point`: a year, a month, a day, a keyword or an amount.
fn date(value: &str) -> Option<PimdirQueryDate> {
    match value {
        "today" => return Some(PimdirQueryDate::Today),
        "yesterday" => return Some(PimdirQueryDate::Yesterday),
        _ => (),
    }

    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    let unit = match value.bytes().last() {
        Some(b'd') => Some(PimdirQueryDateUnit::Day),
        Some(b'w') => Some(PimdirQueryDateUnit::Week),
        Some(b'y') => Some(PimdirQueryDateUnit::Year),
        _ => None,
    };
    if let Some(unit) = unit {
        let amount = &value[..value.len() - 1];
        if digits(amount) {
            return Some(PimdirQueryDate::Relative {
                amount: amount.parse().ok()?,
                unit,
            });
        }
    }

    let number = |part: &str, len: usize| {
        (part.len() == len && digits(part))
            .then(|| part.parse::<u16>().ok())
            .flatten()
    };
    let mut parts = value.split('-');
    let year = number(parts.next()?, 4)?;
    let Some(month) = parts.next() else {
        return Some(PimdirQueryDate::Year(year));
    };
    let month = number(month, 2).filter(|m| (1..=12).contains(m))? as u8;
    match (parts.next(), parts.next()) {
        (None, _) => Some(PimdirQueryDate::Month { year, month }),
        (Some(day), None) => {
            let day = number(day, 2).filter(|d| *d >= 1 && *d <= days_in(year, month) as u16)?;
            Some(PimdirQueryDate::Day {
                year,
                month,
                day: day as u8,
            })
        }
        _ => None,
    }
}

/// The days of a month in the proleptic Gregorian calendar.
fn days_in(year: u16, month: u8) -> u8 {
    match month {
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use crate::search::query::*;

    fn refusal(query: &str) -> (&'static str, usize, usize) {
        let err = PimdirQuery::parse(query).unwrap_err();
        (err.code.as_str(), err.start, err.end)
    }

    #[test]
    fn precedence_is_not_then_and_then_or() {
        let parsed = PimdirQuery::parse("a b or not c and -d").unwrap();
        let PimdirQuery::Or(operands) = parsed else {
            panic!("or at the root");
        };
        assert!(matches!(&operands[0], PimdirQuery::And(and) if and.len() == 2));
        let PimdirQuery::And(right) = &operands[1] else {
            panic!("and under or");
        };
        assert!(
            right
                .iter()
                .all(|operand| matches!(operand, PimdirQuery::Not(_)))
        );
    }

    #[test]
    fn a_dash_starts_a_negation_only_at_a_term_start() {
        let parsed = PimdirQuery::parse("stand-up from:-x").unwrap();
        let terms = parsed.terms();
        assert!(terms.iter().all(|(_, negated)| !negated));
        assert_eq!(refusal("not -"), ("dangling-operator", 4, 5));
        assert_eq!(refusal("not not"), ("dangling-operator", 0, 3));
    }

    #[test]
    fn a_field_in_another_case_is_pointed_at_its_lowercase_name() {
        let err = PimdirQuery::parse("Subject:boiler").unwrap_err();
        assert_eq!(
            (err.code.as_str(), err.start, err.end),
            ("unknown-field", 0, 8)
        );
        assert!(err.message.contains("did you mean subject:?"));
        assert!(err.expected.is_empty() && err.fields.len() == FIELDS.len());

        let err = PimdirQuery::parse("foo:bar").unwrap_err();
        assert!(err.message.contains("quote the word"));
    }

    #[test]
    fn nesting_is_bounded() {
        let deep = format!(
            "{}a{}",
            "(".repeat(NESTING_MAX + 1),
            ")".repeat(NESTING_MAX + 1)
        );
        assert_eq!(refusal(&deep).0, "unbalanced-parenthesis");
        assert!(PimdirQuery::parse(&"not ".repeat(NESTING_MAX + 1)).is_err());
    }

    #[test]
    fn dates_and_stamps_read_their_forms() {
        let range = |query: &str| match PimdirQuery::parse(query).unwrap() {
            PimdirQuery::Term(PimdirQueryTerm {
                predicate: PimdirQueryPredicate::Date(range),
                ..
            }) => (
                range.start.as_ref().and_then(PimdirQueryDate::first_day),
                range.end.as_ref().and_then(PimdirQueryDate::day_after),
            ),
            _ => panic!("a date term"),
        };
        assert_eq!(
            range("date:2024-02-29"),
            (Some("2024-02-29".into()), Some("2024-03-01".into()))
        );
        assert_eq!(range("date:..2026-12"), (None, Some("2027-01-01".into())));
        assert_eq!(
            range("date:2026"),
            (Some("2026-01-01".into()), Some("2027-01-01".into()))
        );
        assert_eq!(refusal("date:2026-02-29"), ("invalid-date", 5, 15));
        assert_eq!(refusal("date:3m"), ("invalid-date", 5, 7));
        assert_eq!(refusal("changed:1..x"), ("invalid-value", 8, 12));
        assert_eq!(refusal("kind:\"mail\" is:\"x\""), ("invalid-value", 15, 18));
    }

    #[test]
    fn a_linked_value_reads_a_role_only_when_it_names_one() {
        let linked = |query: &str| match &PimdirQuery::parse(query).unwrap() {
            PimdirQuery::Term(term) => match &term.predicate {
                PimdirQueryPredicate::Linked { role, id } => (
                    role.as_ref().map(|role| role.as_str().to_string()),
                    id.clone(),
                ),
                _ => panic!("a linked term"),
            },
            _ => panic!("one term"),
        };
        assert_eq!(
            linked("linked:x-cal:a"),
            (Some("x-cal".into()), PimdirQueryId::Link("a".into()))
        );
        assert_eq!(
            linked("linked:x-:a"),
            (None, PimdirQueryId::Link("x-:a".into()))
        );
        assert_eq!(
            linked("linked:sender:007"),
            (Some("sender".into()), PimdirQueryId::Digits("007".into()))
        );
        assert_eq!(PimdirQueryId::Digits("007".into()).seq(), Some(7));
        assert_eq!(
            PimdirQueryId::Digits("99999999999999999999".into()).seq(),
            None
        );
    }
}
