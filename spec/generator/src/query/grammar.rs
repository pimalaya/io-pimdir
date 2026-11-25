//! # The grammar
//!
//! SEARCH §8.3 read as written: the tokens, the operators and their
//! precedence, the fields of §8's table and the values each takes, and the
//! refusals of §8.4, each by its code and its span. The oracle answers no
//! query it refuses.

use crate::query::date;

/// A query as §8.3's grammar reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// A disjunction.
    Or(Vec<Node>),
    /// A conjunction, by `and` or juxtaposition.
    And(Vec<Node>),
    /// A negation, by `not` or `-`.
    Not(Box<Node>),
    /// A group, which is its disjunction.
    Group(Box<Node>),
    /// A term.
    Term(Term),
}

/// One term: free text, a phrase or `field:value`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Term {
    /// Its field, none for free text and a phrase.
    pub field: Option<String>,
    /// Its text or value as written, quotes and a trailing wildcard aside.
    pub value: String,
    /// Whether it was a phrase.
    pub phrase: bool,
    /// Whether it ended with the wildcard.
    pub wildcard: bool,
}

/// A query §8.4 refuses: its code and the span at fault, half-open offsets
/// in Unicode scalar values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// The code.
    pub code: &'static str,
    /// Where the span starts.
    pub start: usize,
    /// Where it ends.
    pub end: usize,
}

/// Every field of §8's table.
pub const FIELDS: [&str; 36] = [
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
    "flag",
    "is",
    "has",
    "kind",
    "status",
    "priority",
    "category",
    "label",
    "property",
    "account",
    "collection",
    "folder",
    "date",
    "when",
    "thread",
    "id",
    "mid",
    "linked",
    "attachment",
    "mimetype",
    "location",
    "changed",
];

/// The address roles of STORAGE Annex A.6 a field names.
pub const ROLES: [&str; 7] = ["from", "to", "cc", "bcc", "attendee", "organizer", "email"];

/// The fields whose value is a text term (§8.3, "Wildcard").
pub const TEXT_FIELDS: [&str; 16] = [
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

/// The roles STORAGE §14.2 names, which a `linked:` value may name before
/// its id.
const REFERENCE_ROLES: [&str; 6] = [
    "attachment",
    "invitation",
    "sender",
    "parent",
    "source",
    "related",
];

const LENGTH_MAX: usize = 1024;
const NESTING_MAX: usize = 32;
const TERMS_MAX: usize = 128;

/// Whether `linked:` reads `role` as a role of STORAGE §14.2: one named
/// there, or `x-` and a name.
pub fn reference_role(role: &str) -> bool {
    REFERENCE_ROLES.contains(&role) || role.strip_prefix("x-").is_some_and(|name| !name.is_empty())
}

/// Reads `query` as §8.3 does, or the refusal §8.4 gives, found in the
/// passes it orders.
pub fn parse(query: &str) -> Result<Node, Refusal> {
    let chars: Vec<char> = query.chars().collect();
    if chars.len() > LENGTH_MAX {
        return Err(refusal("limit-exceeded", LENGTH_MAX, chars.len()));
    }

    let lexemes = lexemes(&chars)?;

    let mut open = Vec::new();
    let mut faults = Vec::new();
    for lexeme in &lexemes {
        match lexeme.token {
            Token::Open => {
                open.push(lexeme.start);
                if open.len() == NESTING_MAX + 1 {
                    faults.push((lexeme.start, "limit-exceeded"));
                }
            }
            Token::Close if open.is_empty() => {
                faults.push((lexeme.start, "unbalanced-parenthesis"))
            }
            Token::Close => {
                open.pop();
            }
            _ => {}
        }
    }
    if let Some(at) = open.first() {
        faults.push((*at, "unbalanced-parenthesis"));
    }
    if let Some((at, code)) = faults.into_iter().min() {
        return Err(refusal(code, at, at + 1));
    }

    if lexemes.is_empty() {
        return Err(refusal("empty-query", 0, chars.len()));
    }
    let mut parser = Parser {
        lexemes,
        length: chars.len(),
        at: 0,
        terms: 0,
    };
    let node = parser.disjunction()?;
    match parser.peek() {
        None => Ok(node),
        Some(_) => Err(parser.refusal("unbalanced-parenthesis", parser.at)),
    }
}

/// One token of §8.3.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Open,
    Close,
    Minus {
        bare: bool,
    },
    Phrase(String),
    Word(String),
    Field {
        field: String,
        value: String,
        phrase: bool,
        /// Where its value starts, right after the colon.
        from: usize,
    },
}

/// A token and the span it spells.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Lexeme {
    token: Token,
    start: usize,
    end: usize,
}

fn refusal(code: &'static str, start: usize, end: usize) -> Refusal {
    Refusal { code, start, end }
}

fn is_sep(char: char) -> bool {
    matches!(char, ' ' | '\t' | '\r' | '\n')
}

fn lexemes(chars: &[char]) -> Result<Vec<Lexeme>, Refusal> {
    let mut lexemes = Vec::new();
    let mut at = 0;
    let mut primary = true;

    while at < chars.len() {
        let char = chars[at];
        if is_sep(char) {
            at += 1;
            primary = true;
            continue;
        }
        let start = at;
        let token = match char {
            '(' => {
                at += 1;
                primary = true;
                Token::Open
            }
            ')' => {
                at += 1;
                primary = false;
                Token::Close
            }
            '"' => {
                let (text, end) = phrase(chars, at)?;
                at = end;
                primary = false;
                Token::Phrase(text)
            }
            '-' if primary => {
                at += 1;
                let bare = chars
                    .get(at)
                    .is_none_or(|next| is_sep(*next) || *next == ')');
                Token::Minus { bare }
            }
            _ => {
                while at < chars.len()
                    && !is_sep(chars[at])
                    && !matches!(chars[at], '(' | ')' | '"')
                {
                    at += 1;
                }
                let word: String = chars[start..at].iter().collect();
                primary = false;
                match word.split_once(':') {
                    Some((field, value)) => {
                        let from = start + field.chars().count() + 1;
                        let quoted = value.is_empty() && chars.get(at) == Some(&'"');
                        let value = match quoted {
                            true => {
                                let (text, end) = phrase(chars, at)?;
                                at = end;
                                text
                            }
                            false => String::from(value),
                        };
                        Token::Field {
                            field: String::from(field),
                            value,
                            phrase: quoted,
                            from,
                        }
                    }
                    None => {
                        primary = ["and", "or", "not"]
                            .iter()
                            .any(|name| word.eq_ignore_ascii_case(name));
                        Token::Word(word)
                    }
                }
            }
        };
        lexemes.push(Lexeme {
            token,
            start,
            end: at,
        });
    }

    Ok(lexemes)
}

fn phrase(chars: &[char], at: usize) -> Result<(String, usize), Refusal> {
    let close = chars[at + 1..]
        .iter()
        .position(|char| *char == '"')
        .ok_or(refusal("unterminated-phrase", at, chars.len()))?;
    let text = chars[at + 1..at + 1 + close].iter().collect();
    Ok((text, at + close + 2))
}

struct Parser {
    lexemes: Vec<Lexeme>,
    length: usize,
    at: usize,
    terms: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.lexemes.get(self.at).map(|lexeme| &lexeme.token)
    }

    fn operator(&self, name: &str) -> bool {
        matches!(self.peek(), Some(Token::Word(word)) if word.eq_ignore_ascii_case(name))
    }

    /// The refusal `code` over the token at `at`, the end of the query
    /// past the last.
    fn refusal(&self, code: &'static str, at: usize) -> Refusal {
        match self.lexemes.get(at) {
            Some(lexeme) => refusal(code, lexeme.start, lexeme.end),
            None => refusal(code, self.length, self.length),
        }
    }

    fn disjunction(&mut self) -> Result<Node, Refusal> {
        let mut sides = vec![self.conjunction()?];
        while self.operator("or") {
            let or = self.at;
            self.at += 1;
            if self.ends() {
                return Err(self.refusal("dangling-operator", or));
            }
            sides.push(self.conjunction()?);
        }
        Ok(match sides.len() {
            1 => sides.remove(0),
            _ => Node::Or(sides),
        })
    }

    fn ends(&self) -> bool {
        matches!(self.peek(), None | Some(Token::Close))
            || self.operator("or")
            || self.operator("and")
    }

    fn conjunction(&mut self) -> Result<Node, Refusal> {
        if self.operator("and") || self.operator("or") {
            return Err(self.refusal("dangling-operator", self.at));
        }

        let mut sides = vec![self.negation()?];
        loop {
            if self.operator("and") {
                let and = self.at;
                self.at += 1;
                if self.ends() {
                    return Err(self.refusal("dangling-operator", and));
                }
            } else if matches!(self.peek(), None | Some(Token::Close)) || self.operator("or") {
                break;
            }
            sides.push(self.negation()?);
        }
        Ok(match sides.len() {
            1 => sides.remove(0),
            _ => Node::And(sides),
        })
    }

    /// A run of `not` and `-` and its primary: a `-` followed by a
    /// separator, a `)` or the end answers itself, a run missing its term
    /// its first operator (§8.4).
    fn negation(&mut self) -> Result<Node, Refusal> {
        let first = self.at;
        let mut nots = 0;
        while self.operator("not") || matches!(self.peek(), Some(Token::Minus { .. })) {
            if self.peek() == Some(&Token::Minus { bare: true }) {
                return Err(self.refusal("dangling-operator", self.at));
            }
            self.at += 1;
            nots += 1;
            if self.ends() {
                return Err(self.refusal("dangling-operator", first));
            }
        }

        let mut node = self.primary()?;
        for _ in 0..nots {
            node = Node::Not(Box::new(node));
        }
        Ok(node)
    }

    fn primary(&mut self) -> Result<Node, Refusal> {
        let Lexeme { token, start, end } = self.lexemes[self.at].clone();
        self.at += 1;
        match token {
            Token::Open => {
                if self.peek() == Some(&Token::Close) {
                    return Err(refusal("empty-query", start, self.lexemes[self.at].end));
                }
                let inner = self.disjunction()?;
                self.at += 1;
                Ok(Node::Group(Box::new(inner)))
            }
            Token::Close | Token::Minus { .. } => Err(refusal("dangling-operator", start, end)),
            Token::Phrase(text) => {
                self.count(start, end)?;
                Ok(Node::Term(Term {
                    field: None,
                    value: text,
                    phrase: true,
                    wildcard: false,
                }))
            }
            Token::Word(word) => {
                self.count(start, end)?;
                text_term(word, start, end).map(Node::Term)
            }
            Token::Field {
                field,
                value,
                phrase,
                from,
            } => {
                self.count(start, end)?;
                field_term(field, value, phrase, (start, from, end)).map(Node::Term)
            }
        }
    }

    fn count(&mut self, start: usize, end: usize) -> Result<(), Refusal> {
        self.terms += 1;
        match self.terms > TERMS_MAX {
            true => Err(refusal("limit-exceeded", start, end)),
            false => Ok(()),
        }
    }
}

/// Whether a text or a value begins a form §8.3 reserves: `~`, and of two
/// characters or more `/`.
fn reserved(text: &str) -> bool {
    text.starts_with('~') || (text.chars().count() >= 2 && text.starts_with('/'))
}

/// A text or a text field's value starting at `start` read for its
/// wildcard: a `*` closing two characters or more, any other refused at
/// the first.
fn wildcard(text: &str, start: usize) -> Result<(String, bool), Refusal> {
    let chars: Vec<char> = text.chars().collect();
    let Some(star) = chars.iter().position(|char| *char == '*') else {
        return Ok((String::from(text), false));
    };
    match star == chars.len() - 1 && star >= 2 {
        true => Ok((chars[..star].iter().collect(), true)),
        false => Err(refusal("invalid-wildcard", start + star, start + star + 1)),
    }
}

fn text_term(word: String, start: usize, end: usize) -> Result<Term, Refusal> {
    if reserved(&word) {
        return Err(refusal("reserved-form", start, end));
    }
    let (value, wildcard) = wildcard(&word, start)?;
    Ok(Term {
        field: None,
        value,
        phrase: false,
        wildcard,
    })
}

/// A field term spanning `start` to `end`, its value from `from`, checked
/// in §8.4's order: its field, a missing value, a reserved form, the
/// wildcard, then its value.
fn field_term(
    field: String,
    value: String,
    phrase: bool,
    (start, from, end): (usize, usize, usize),
) -> Result<Term, Refusal> {
    if !FIELDS.contains(&field.as_str()) {
        return Err(refusal("unknown-field", start, from));
    }
    if value.is_empty() && !phrase {
        return Err(refusal("invalid-value", start, end));
    }
    if !phrase && (reserved(&value) || value.starts_with('{')) {
        return Err(refusal("reserved-form", from, end));
    }

    let address = (ROLES.contains(&field.as_str()) || field == "with") && value.contains('@');
    let (value, wildcard) = match phrase {
        true => (value, false),
        false if TEXT_FIELDS.contains(&field.as_str()) && !address => wildcard(&value, from)?,
        false => match value.chars().position(|char| char == '*') {
            Some(star) => {
                return Err(refusal("invalid-wildcard", from + star, from + star + 1));
            }
            None => (value, false),
        },
    };

    let known = |values: &[&str]| values.contains(&value.as_str());
    let valid = match field.as_str() {
        "kind" => known(&["mail", "contact", "event", "task", "journal", "file"]),
        "is" => known(&[
            "unread",
            "read",
            "flagged",
            "answered",
            "draft",
            "encrypted",
        ]),
        "has" => known(&[
            "body",
            "attachment",
            "phone",
            "address",
            "birthday",
            "photo",
        ]),
        "status" => known(&["needs-action", "in-process", "completed", "cancelled"]),
        "priority" => known(&["high", "medium", "low", "none"]),
        "changed" => date::stamps(&value).is_some(),
        "label" => property_value(&value),
        "property" => match value.split_once('=') {
            Some((key, value)) => property_key(key) && property_value(value),
            None => false,
        },
        "linked" => !matches!(value.split_once(':'), Some((role, "")) if reference_role(role)),
        "date" | "when" => {
            if date::DateValue::parse(&value).is_none() {
                return Err(refusal("invalid-date", from, end));
            }
            true
        }
        _ => true,
    };
    match valid {
        true => Ok(Term {
            field: Some(field),
            value,
            phrase,
            wildcard,
        }),
        false => Err(refusal("invalid-value", from, end)),
    }
}

fn property_key(key: &str) -> bool {
    let part = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
    };
    key.chars().count() <= 64
        && key
            .split_once('/')
            .is_some_and(|(namespace, name)| part(namespace) && part(name))
}

fn property_value(value: &str) -> bool {
    !value.is_empty() && value.len() <= 1024 && !value.contains('\0')
}

#[cfg(test)]
mod tests {
    use crate::query::grammar::{Node, Term, parse};

    fn term(field: Option<&str>, value: &str) -> Node {
        Node::Term(Term {
            field: field.map(String::from),
            value: String::from(value),
            phrase: false,
            wildcard: false,
        })
    }

    #[test]
    fn precedence_binds_not_then_and_then_or() {
        let parsed = parse("a b or not c").unwrap();
        assert_eq!(
            parsed,
            Node::Or(vec![
                Node::And(vec![term(None, "a"), term(None, "b")]),
                Node::Not(Box::new(term(None, "c"))),
            ])
        );
        let parsed = parse("stand-up -from:-x --y").unwrap();
        assert_eq!(
            parsed,
            Node::And(vec![
                term(None, "stand-up"),
                Node::Not(Box::new(term(Some("from"), "-x"))),
                Node::Not(Box::new(Node::Not(Box::new(term(None, "y"))))),
            ])
        );
    }

    #[test]
    fn each_fault_answers_its_code() {
        let cases = [
            ("", Some("empty-query")),
            ("  ", Some("empty-query")),
            ("()", Some("empty-query")),
            ("\"a", Some("unterminated-phrase")),
            ("a)", Some("unbalanced-parenthesis")),
            ("(a", Some("unbalanced-parenthesis")),
            ("a or", Some("dangling-operator")),
            ("- a", Some("dangling-operator")),
            ("nope:x", Some("unknown-field")),
            ("kind:car", Some("invalid-value")),
            ("date:2026-13", Some("invalid-date")),
            ("~meeting", Some("reserved-form")),
            ("thread:{from:jane}", Some("reserved-form")),
            ("a*", Some("invalid-wildcard")),
            ("tag:ab*", Some("invalid-wildcard")),
            ("x~y a/b {x} / \"~meeting\" inv*", None),
            ("~", Some("reserved-form")),
            ("from:@exa*", Some("invalid-wildcard")),
            ("((a) b", Some("unbalanced-parenthesis")),
            ("subject:\"\"", None),
            ("kind:\"\"", Some("invalid-value")),
            ("linked:sender:", Some("invalid-value")),
            ("linked:other:", None),
        ];
        for (query, code) in cases {
            assert_eq!(
                parse(query).err().map(|refusal| refusal.code),
                code,
                "{query}"
            );
        }
    }

    #[test]
    fn a_dangling_run_answers_its_first_operator() {
        let cases = [
            ("not or reunion", (0, 3)),
            ("-or x", (0, 1)),
            ("and reunion", (0, 3)),
            ("x not not", (2, 5)),
            ("x not - y", (6, 7)),
            ("x and", (2, 5)),
            ("x or or y", (2, 4)),
        ];
        for (query, span) in cases {
            let refusal = parse(query).unwrap_err();
            assert_eq!(refusal.code, "dangling-operator", "{query}");
            assert_eq!((refusal.start, refusal.end), span, "{query}");
        }
    }
}
