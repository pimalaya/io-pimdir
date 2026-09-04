//! # Contact summaries
//!
//! The `text/vcard` derivation (Annex A.2): the `UID` as the key, the
//! `FN`, `KIND` and first `ORG` component unescaped, every `EMAIL`
//! canonical, and the name lowercased for ordering.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use crate::{
    placement::{PimdirLinkId, PimdirSortKey},
    summary::{PimdirAddress, PimdirDerivation, PimdirSummary, hash_key, unfold},
};

/// The `contact_summary` row of a card with the emails it carries.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirContactSummary {
    /// The `UID` verbatim.
    pub uid: Option<String>,
    /// The `FN`, unescaped; empty when absent.
    pub full_name: String,
    /// The `KIND` lowercased: `individual`, `group`, `org` or `location`.
    pub kind: Option<String>,
    /// The first component of the first `ORG`, unescaped.
    pub org: Option<String>,
    /// Every `EMAIL`, canonical, in document order.
    pub emails: Vec<PimdirAddress>,
}

impl PimdirContactSummary {
    /// The name lowercased by the Unicode simple mapping, then trimmed;
    /// read ascending, a nameless card first.
    pub fn sort_key(&self) -> PimdirSortKey {
        PimdirSortKey(self.full_name.to_lowercase().trim().to_string())
    }
}

/// Derives a card's key, summary and sort key from its bytes.
pub fn derive(body: &[u8]) -> PimdirDerivation {
    let lines: Vec<PimdirContentLine> = unfold(body, false)
        .iter()
        .filter_map(|line| PimdirContentLine::parse(line))
        .collect();
    let first = |name: &str| lines.iter().find(|line| line.is(name));

    let summary = PimdirContactSummary {
        uid: first("UID").map(|line| line.value.clone()),
        full_name: first("FN")
            .map(|line| unescape(&line.value))
            .unwrap_or_default(),
        kind: first("KIND").map(|line| line.value.trim().to_lowercase()),
        org: first("ORG").map(|line| unescape(components(&line.value)[0])),
        emails: lines
            .iter()
            .filter(|line| line.is("EMAIL"))
            .map(|line| PimdirAddress {
                address: PimdirAddress::canonical(&line.value),
                name: None,
            })
            .collect(),
    };

    PimdirDerivation {
        link_id: match &summary.uid {
            Some(uid) => PimdirLinkId(uid.clone()),
            None => hash_key(body),
        },
        sort_key: summary.sort_key(),
        summary: Some(PimdirSummary::Contact(summary)),
    }
}

/// One vCard or iCalendar content line:
/// `[group "."] name *(";" param) ":" value`.
///
/// Split on the first colon outside a quoted parameter value (RFC 6350
/// §3.3), the group dropped, the parameter names uppercased and their
/// values unquoted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PimdirContentLine {
    /// The property name, uppercased.
    pub name: String,
    /// The parameters, names uppercased, values unquoted.
    pub params: Vec<(String, String)>,
    /// The value, verbatim.
    pub value: String,
}

impl PimdirContentLine {
    /// Reads a line, or `None` for one carrying no colon outside quotes.
    pub fn parse(line: &str) -> Option<Self> {
        let mut quoted = false;
        let colon = line.char_indices().find_map(|(at, char)| match char {
            '"' => {
                quoted = !quoted;
                None
            }
            ':' if !quoted => Some(at),
            _ => None,
        })?;
        let (head, value) = (&line[..colon], &line[colon + 1..]);

        let mut parts = split_unquoted(head, ';').into_iter();
        let name = parts.next()?;
        let name = name
            .rsplit('.')
            .next()
            .unwrap_or_default()
            .trim()
            .to_uppercase();
        let params = parts
            .filter_map(|param| {
                let (name, value) = param.split_once('=')?;
                Some((
                    name.trim().to_uppercase(),
                    value.trim().trim_matches('"').to_string(),
                ))
            })
            .collect();

        Some(Self {
            name,
            params,
            value: value.to_string(),
        })
    }

    /// Whether the line carries the named property.
    pub fn is(&self, name: &str) -> bool {
        self.name == name
    }

    /// A parameter's value, by uppercased name.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(param, _)| param == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Splits on a separator outside double quotes.
fn split_unquoted(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut quoted = false;
    let mut start = 0;

    for (at, char) in text.char_indices() {
        match char {
            '"' => quoted = !quoted,
            char if char == separator && !quoted => {
                parts.push(&text[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);

    parts
}

/// A structured value's components, split on the semicolons no
/// backslash escapes (RFC 6350 §3.3).
pub(crate) fn components(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut escaped = false;

    for (at, char) in value.char_indices() {
        match (escaped, char) {
            (false, '\\') => escaped = true,
            (false, ';') => {
                parts.push(&value[start..at]);
                start = at + 1;
            }
            _ => escaped = false,
        }
    }
    parts.push(&value[start..]);

    parts
}

/// A text value unescaped (RFC 6350 §3.4, RFC 5545 §3.3.11): `\n` a
/// newline, `\,` `\;` `\\` the character itself.
pub(crate) fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut escaped = false;

    for char in value.chars() {
        match (escaped, char) {
            (false, '\\') => escaped = true,
            (true, 'n' | 'N') => {
                out.push('\n');
                escaped = false;
            }
            (true, char) => {
                out.push(char);
                escaped = false;
            }
            (false, char) => out.push(char),
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn a_content_line_splits_on_the_first_unquoted_colon() {
        let line = PimdirContentLine::parse("item1.URL;TYPE=\"a:b\";X=1:https://x.y").unwrap();
        assert_eq!(line.name, "URL");
        assert_eq!(line.param("TYPE"), Some("a:b"));
        assert_eq!(line.param("X"), Some("1"));
        assert_eq!(line.value, "https://x.y");
        assert_eq!(PimdirContentLine::parse("no colon"), None);
    }

    #[test]
    fn text_values_unescape_and_structured_ones_split_on_unescaped_semicolons() {
        assert_eq!(unescape("a\\,b\\;c\\\\d\\ne"), "a,b;c\\d\ne");
        assert_eq!(
            components("Acme\\; Inc;Sales"),
            vec!["Acme\\; Inc", "Sales"]
        );
    }

    #[test]
    fn a_card_derives_its_row_and_a_uid_less_one_a_hash_key() {
        let card = b"BEGIN:VCARD\r\nUID:c1\r\nFN:Ada\\, Lovelace\r\nKIND:Individual\r\nORG:Analytical\\, Engines;R&D\r\nEMAIL:Ada@Example.org\r\nEND:VCARD\r\n";
        let derivation = derive(card);
        let Some(PimdirSummary::Contact(summary)) = derivation.summary else {
            panic!("a contact summary");
        };
        assert_eq!(derivation.link_id.as_str(), "c1");
        assert_eq!(summary.full_name, "Ada, Lovelace");
        assert_eq!(summary.kind.as_deref(), Some("individual"));
        assert_eq!(summary.org.as_deref(), Some("Analytical, Engines"));
        assert_eq!(summary.emails[0].address, "ada@example.org");
        assert_eq!(derivation.sort_key.as_str(), "ada, lovelace");

        let anonymous = derive(b"BEGIN:VCARD\r\nEND:VCARD\r\n");
        assert!(anonymous.link_id.as_str().starts_with("hash:"));
        assert_eq!(anonymous.sort_key.as_str(), "");
    }
}
