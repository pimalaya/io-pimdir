//! # Declared schema
//!
//! What the canonical migrations declare, read from their text alone: the
//! statements, the object each creates and its text as the schema check
//! compares it. The build script compiles this module too, so the
//! canonical schema is a constant ([`crate::sql::DECLARED`]) rather than
//! parsed on every open.

use alloc::{string::String, vec::Vec};
use std::collections::BTreeMap;

/// A declaring statement as the schema check compares it: every `--`
/// comment dropped and whitespace collapsed to single spaces.
pub(crate) fn normalised(sql: &str) -> String {
    let mut out = String::new();
    for line in sql.lines() {
        let code = line.split_once("--").map_or(line, |(code, _)| code);
        for word in code.split_whitespace() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(word);
        }
    }
    out
}

/// Every table, index, trigger and view `migrations` create, by name, the
/// text SQLite records for each [`normalised`]: read from the statements
/// themselves, so no database is built to learn them.
///
/// A statement runs from its first word to its `;`, a trigger's from
/// `CREATE` to the `;` after the `END` closing its `BEGIN`, a `CASE` in
/// its body nesting; comments, strings and quoted names hold no
/// boundary. SQLite records that text from `CREATE` on, which
/// [`normalised`] reads alike.
pub(crate) fn declared(migrations: &[&str]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for migration in migrations {
        for statement in statements(migration) {
            if let Some(name) = created(statement) {
                out.insert(name, normalised(statement));
            }
        }
    }
    out
}

/// The statements of a migration, each from its first word to before its
/// `;`.
fn statements(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    let mut start: Option<usize> = None;
    let mut words: Vec<String> = Vec::new();
    let mut depth = 0usize;
    while at < bytes.len() {
        let byte = bytes[at];
        match byte {
            b'-' if bytes.get(at + 1) == Some(&b'-') => {
                while at < bytes.len() && bytes[at] != b'\n' {
                    at += 1;
                }
                continue;
            }
            b'/' if bytes.get(at + 1) == Some(&b'*') => {
                at += 2;
                while at + 1 < bytes.len() && !(bytes[at] == b'*' && bytes[at + 1] == b'/') {
                    at += 1;
                }
                at += 2;
                continue;
            }
            b'\'' | b'"' | b'`' | b'[' => {
                start.get_or_insert(at);
                let close = if byte == b'[' { b']' } else { byte };
                at += 1;
                while at < bytes.len() {
                    if bytes[at] == close {
                        if close != b']' && bytes.get(at + 1) == Some(&close) {
                            at += 2;
                            continue;
                        }
                        break;
                    }
                    at += 1;
                }
                at += 1;
                continue;
            }
            b';' => {
                let trigger = words.len() >= 2 && words[1] == "TRIGGER"
                    || words.len() >= 3 && words[2] == "TRIGGER";
                if !trigger || (depth == 0 && words.iter().any(|word| word == "BEGIN")) {
                    if let Some(from) = start.take() {
                        out.push(&text[from..at]);
                    }
                    words.clear();
                    depth = 0;
                }
                at += 1;
                continue;
            }
            byte if byte.is_ascii_alphanumeric() || byte == b'_' => {
                let from = at;
                while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
                    at += 1;
                }
                start.get_or_insert(from);
                let word = text[from..at].to_ascii_uppercase();
                match word.as_str() {
                    "BEGIN" | "CASE" => depth += 1,
                    "END" => depth = depth.saturating_sub(1),
                    _ => (),
                }
                if words.len() < 3 || word == "BEGIN" {
                    words.push(word);
                }
                continue;
            }
            byte if byte.is_ascii_whitespace() => (),
            _ => {
                start.get_or_insert(at);
            }
        }
        at += 1;
    }
    out
}

/// The name a `CREATE` statement gives its table, index, trigger or view,
/// `None` for another statement.
fn created(statement: &str) -> Option<String> {
    let mut words = statement.split_whitespace();
    if !words.next()?.eq_ignore_ascii_case("CREATE") {
        return None;
    }
    let mut word = words.next()?;
    for skipped in ["UNIQUE", "TEMP", "TEMPORARY", "VIRTUAL"] {
        if word.eq_ignore_ascii_case(skipped) {
            word = words.next()?;
        }
    }
    if !["TABLE", "INDEX", "TRIGGER", "VIEW"]
        .iter()
        .any(|kind| word.eq_ignore_ascii_case(kind))
    {
        return None;
    }
    let mut name = words.next()?;
    if name.eq_ignore_ascii_case("IF") {
        words.next()?;
        words.next()?;
        name = words.next()?;
    }
    let name = name.split('(').next()?;
    let name = name.rsplit('.').next()?;
    Some(name.trim_matches(|c| c == '"' || c == '`').into())
}
