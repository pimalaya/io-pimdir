//! # Security
//!
//! The two text rules of STORAGE §18 a program applies before a name or
//! an error leaves the store's control: [`path_component`] neutralises a
//! name a server or a body gave before it becomes a file name, and
//! [`scrub`] removes the credentials an error's text may carry before it
//! is recorded. The filesystem rules (modes, link-free lock opens, a
//! store from elsewhere) are the [`client`](crate::client)'s.

use alloc::string::String;

/// What a removed credential is written as.
const REDACTED: &str = "***";

/// The key endings naming a credential, lowercase: a value after one of
/// them and `=` or `:` is scrubbed.
const SECRETS: [&str; 8] = [
    "password", "passwd", "pwd", "token", "secret", "apikey", "api_key", "api-key",
];

/// The header names whose whole value, to the end of the line, is a
/// credential, lowercase.
const HEADERS: [&str; 4] = [
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
];

/// The bidirectional formatting characters STORAGE §18 names.
const BIDI: [char; 12] = [
    '\u{061C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}',
    '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
];

/// A name as one path component (STORAGE §18): every path separator (`/`
/// and `\`, read as one on some platform), NUL, control character and
/// bidirectional formatting character replaced by `_`, and a name that
/// would be empty, `.` or `..` written as underscores.
///
/// The answer is a single component that names no other directory, so a
/// path joined from a directory and it stays in that directory. A file's
/// name (Annex A.7), an attachment's `filename`, a collection's name and a
/// part's section go through it before they name a file.
pub fn path_component(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' => '_',
            c if c.is_control() || BIDI.contains(&c) => '_',
            c => c,
        })
        .collect();

    match out.as_str() {
        "" => out.push('_'),
        "." | ".." => out = "_".repeat(out.len()),
        _ => (),
    }

    out
}

/// `text` with the credentials it may carry replaced by `***` (STORAGE
/// §9.2, §18): a URL's userinfo, the value of an `Authorization`, a
/// `Cookie` or a `Set-Cookie` header to the end of its line, a `Bearer`
/// token, and the value after `=` or `:` of any key ending in `password`,
/// `passwd`, `pwd`, `token`, `secret` or `apikey` (a query string's
/// `access_token`, a JSON `"client_secret"`). What a program records from
/// an error, a parked row's reason say, goes through it first.
pub fn scrub(text: &str) -> String {
    scrub_keys(&scrub_userinfo(text))
}

/// `text` with the userinfo of every URL (`scheme://user:pass@host`)
/// replaced, the scheme and the host kept.
fn scrub_userinfo(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(at) = rest.find("://") {
        let (head, tail) = rest.split_at(at + 3);
        out.push_str(head);

        let end = tail
            .find(|c: char| matches!(c, '/' | '?' | '#') || c.is_whitespace())
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        match authority.rfind('@') {
            Some(user) => {
                out.push_str(REDACTED);
                out.push_str(&authority[user..]);
            }
            None => out.push_str(authority),
        }
        rest = &tail[end..];
    }

    out.push_str(rest);
    out
}

/// `text` with the value of every credential key, header and `Bearer`
/// token replaced.
fn scrub_keys(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut at = 0;

    while at < bytes.len() {
        if !is_key_byte(bytes[at]) || (at > 0 && is_key_byte(bytes[at - 1])) {
            at += 1;
            continue;
        }

        let start = at;
        while at < bytes.len() && is_key_byte(bytes[at]) {
            at += 1;
        }
        let key = text[start..at].to_ascii_lowercase();

        let Some((value, end)) = credential(bytes, &key, at) else {
            continue;
        };
        out.push_str(&text[copied..value]);
        out.push_str(REDACTED);
        copied = end;
        at = end;
    }

    out.push_str(&text[copied..]);
    out
}

/// Where the credential a key ending at `at` names starts and ends in
/// `bytes`, `None` when the key names none or its value is empty.
fn credential(bytes: &[u8], key: &str, at: usize) -> Option<(usize, usize)> {
    if key == "bearer" {
        let value = at + 1;
        if bytes.get(at) != Some(&b' ') || bytes.get(value).is_none_or(|b| ends_value(*b)) {
            return None;
        }
        return Some((value, value_end(bytes, value, false)));
    }

    let header = HEADERS.contains(&key);
    if !header && !SECRETS.iter().any(|secret| key.ends_with(secret)) {
        return None;
    }

    let mut at = at;
    if matches!(bytes.get(at), Some(b'"' | b'\'')) {
        at += 1;
    }
    at = skip_spaces(bytes, at);
    match bytes.get(at) {
        Some(b'=' | b':') => (),
        _ => return None,
    }
    at = skip_spaces(bytes, at + 1);

    let quote = matches!(bytes.get(at), Some(b'"' | b'\''));
    if quote {
        at += 1;
    }
    let end = match header && !quote {
        true => bytes[at..]
            .iter()
            .position(|b| matches!(b, b'\r' | b'\n'))
            .map_or(bytes.len(), |end| at + end),
        false => value_end(bytes, at, quote),
    };

    (end > at).then_some((at, end))
}

/// Where a value starting at `at` ends: its closing quote when quoted,
/// else the first byte that ends a value.
fn value_end(bytes: &[u8], at: usize, quoted: bool) -> usize {
    let ends = |b: &u8| match quoted {
        true => matches!(b, b'"' | b'\'' | b'\r' | b'\n'),
        false => ends_value(*b),
    };
    bytes[at..]
        .iter()
        .position(ends)
        .map_or(bytes.len(), |end| at + end)
}

/// Whether `byte` ends an unquoted value.
fn ends_value(byte: u8) -> bool {
    byte.is_ascii_whitespace()
        || matches!(
            byte,
            b'&' | b',' | b';' | b'"' | b'\'' | b')' | b']' | b'}' | b'<' | b'>'
        )
}

/// The first byte at or after `at` that is no space or tab.
fn skip_spaces(bytes: &[u8], mut at: usize) -> usize {
    while matches!(bytes.get(at), Some(b' ' | b'\t')) {
        at += 1;
    }
    at
}

/// Whether `byte` may belong to a key.
fn is_key_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
}

#[cfg(test)]
mod tests {
    use crate::security::{path_component, scrub};

    #[test]
    fn a_name_stays_one_component() {
        let cases = [
            ("report.pdf", "report.pdf"),
            ("../../etc/passwd", ".._.._etc_passwd"),
            ("..", "__"),
            (".", "_"),
            ("", "_"),
            ("a\\b", "a_b"),
            ("nul\0byte", "nul_byte"),
            ("tab\there\u{7f}\u{85}", "tab_here__"),
            ("invoice\u{202E}fdp.exe", "invoice_fdp.exe"),
            ("\u{2066}x\u{2069}\u{200E}\u{061C}", "_x___"),
            ("Résumé 2026", "Résumé 2026"),
        ];
        for (name, component) in cases {
            assert_eq!(path_component(name), component, "{name:?}");
        }
    }

    #[test]
    fn a_credential_is_scrubbed_and_the_rest_kept() {
        let cases = [
            (
                "connect to imaps://ann:hunter2@mail.example.org:993/INBOX failed",
                "connect to imaps://***@mail.example.org:993/INBOX failed",
            ),
            (
                "GET https://dav.example.org/cal?access_token=abc.def&x=1: 401",
                "GET https://dav.example.org/cal?access_token=***&x=1: 401",
            ),
            (
                "request header Authorization: Basic YW5uOmh1bnRlcjI=\nnext line",
                "request header Authorization: ***\nnext line",
            ),
            (
                "server said: Bearer eyJhbGci.x.y expired",
                "server said: Bearer *** expired",
            ),
            (
                r#"{"client_secret": "s3cr3t", "user": "ann"}"#,
                r#"{"client_secret": "***", "user": "ann"}"#,
            ),
            ("password=hunter2;user=ann", "password=***;user=ann"),
            ("Set-Cookie: sid=42; Path=/", "Set-Cookie: ***"),
            (
                "no such mailbox: Archive/2026 at https://example.org/a",
                "no such mailbox: Archive/2026 at https://example.org/a",
            ),
            ("tokenizer: unicode61", "tokenizer: unicode61"),
            ("?password=&x=1", "?password=&x=1"),
        ];
        for (text, scrubbed) in cases {
            assert_eq!(scrub(text), scrubbed, "{text:?}");
        }
    }
}
