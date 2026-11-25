//! # Text
//!
//! SEARCH §5 and §8.5 read as written: text folded (STORAGE Annex A.2's
//! lowercase, the marks of U+0300 to U+036F dropped), split into tokens as
//! `unicode61` splits them, and a term matching a field holding its
//! tokens consecutively, the last a prefix under the wildcard.
//!
//! The model folds and splits ASCII, the Latin-1 Supplement, Latin
//! Extended-A and Greek, whose canonical decompositions it states below;
//! a text holding any other character is no answer, never a guess.

use std::mem;

use crate::query::Tri;

/// Letters of the blocks the model folds that decompose to a base letter
/// and marks of U+0300 to U+036F alone, lowercase, with that base letter.
const BASES: [(&str, char); 26] = [
    ("àáâãäåāăą", 'a'),
    ("çćĉċč", 'c'),
    ("ď", 'd'),
    ("èéêëēĕėęě", 'e'),
    ("ĝğġģ", 'g'),
    ("ĥ", 'h'),
    ("ìíîïĩīĭį", 'i'),
    ("ĵ", 'j'),
    ("ķ", 'k'),
    ("ĺļľ", 'l'),
    ("ñńņň", 'n'),
    ("òóôõöōŏő", 'o'),
    ("ŕŗř", 'r'),
    ("śŝşš", 's'),
    ("ţť", 't'),
    ("ùúûüũūŭůűų", 'u'),
    ("ŵ", 'w'),
    ("ýÿŷ", 'y'),
    ("źżž", 'z'),
    ("ά", 'α'),
    ("έ", 'ε'),
    ("ή", 'η'),
    ("ίϊΐ", 'ι'),
    ("ό", 'ο'),
    ("ύϋΰ", 'υ'),
    ("ώ", 'ω'),
];

/// Lowercase letters of those blocks with no canonical decomposition,
/// each its own token character.
const KEPT: &str = "ßæðøþđħıĳĸŀłŉŋœŧſαβγδεζηθικλμνξοπρςστυφχψω";

/// The folded tokens of `text` (SEARCH §5, §8.5), none when it holds a
/// character the model does not fold.
pub fn tokens(text: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    for char in text.to_lowercase().chars() {
        let folded = match char {
            '\u{300}'..='\u{36f}' => continue,
            char if char.is_ascii_alphanumeric() => Some(char),
            char if char.is_ascii() => None,
            char if KEPT.contains(char) => Some(char),
            char => Some(
                BASES
                    .iter()
                    .find(|(letters, _)| letters.contains(char))
                    .map(|(_, base)| *base)?,
            ),
        };
        match folded {
            Some(char) => token.push(char),
            None if !token.is_empty() => tokens.push(mem::take(&mut token)),
            None => {}
        }
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    Some(tokens)
}

/// A term's tokens and whether its last is a prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Words {
    /// The tokens, in order.
    pub tokens: Vec<String>,
    /// Whether the last token is a prefix (the wildcard).
    pub prefix: bool,
}

impl Words {
    /// Whether a field holding `values` matches the term (§8.5): a value
    /// holding its tokens consecutively is a match; a term that could only
    /// run from the end of one value into the start of another is no
    /// answer, the order values join in being the index's own.
    pub fn in_values(&self, values: &[String]) -> Tri {
        if self.tokens.is_empty() {
            return Tri::No;
        }

        let mut held = Vec::new();
        for value in values {
            let Some(tokens) = tokens(value) else {
                return Tri::Unknown;
            };
            if tokens
                .windows(self.tokens.len())
                .any(|window| self.fits(window, 0))
            {
                return Tri::Yes;
            }
            held.push(tokens);
        }

        for split in 1..self.tokens.len() {
            for (first, left) in held.iter().enumerate() {
                let ends = left.len() >= split && self.fits(&left[left.len() - split..], 0);
                let joins = held.iter().enumerate().any(|(second, right)| {
                    let rest = self.tokens.len() - split;
                    let reach = right.len().min(rest);
                    second != first && reach > 0 && self.fits(&right[..reach], split)
                });
                if ends && joins {
                    return Tri::Unknown;
                }
            }
        }
        Tri::No
    }

    /// Whether `held` matches the term's tokens from `from` on, one by
    /// one, the last of the term a prefix under the wildcard.
    fn fits(&self, held: &[String], from: usize) -> bool {
        held.iter().enumerate().all(|(at, held)| {
            let at = from + at;
            let token = &self.tokens[at];
            match self.prefix && at == self.tokens.len() - 1 {
                true => held.starts_with(token.as_str()),
                false => held == token,
            }
        })
    }
}

/// `text` folded as STORAGE Annex A.2 folds `fn`: lowercased, then trimmed.
pub fn fold_name(text: &str) -> String {
    String::from(text.to_lowercase().trim())
}

#[cfg(test)]
mod tests {
    use crate::query::{
        Tri,
        text::{Words, tokens},
    };

    #[test]
    fn a_text_folds_case_and_the_marks_of_its_block_alone() {
        assert_eq!(
            tokens("Réunion, REUNION!"),
            Some(vec![String::from("reunion"), String::from("reunion")])
        );
        assert_eq!(
            tokens("Αθηνά stand-up"),
            Some(vec![
                String::from("αθηνα"),
                String::from("stand"),
                String::from("up")
            ])
        );
        assert_eq!(tokens("İstanbul"), Some(vec![String::from("istanbul")]));
        assert_eq!(tokens("ŁÓDŹ"), Some(vec![String::from("łodz")]));
        assert_eq!(tokens("नमस्ते"), None);
    }

    #[test]
    fn a_term_matches_consecutive_tokens_of_one_value() {
        let words = |text: &str, prefix| Words {
            tokens: tokens(text).unwrap(),
            prefix,
        };
        let values = [String::from("Weekly sync notes"), String::from("plan")];
        assert_eq!(words("sync notes", false).in_values(&values), Tri::Yes);
        assert_eq!(words("sync no", true).in_values(&values), Tri::Yes);
        assert_eq!(words("weekly no", true).in_values(&values), Tri::No);
        assert_eq!(words("notes weekly", false).in_values(&values), Tri::No);
        assert_eq!(words("notes plan", false).in_values(&values), Tri::Unknown);
        assert_eq!(
            words("plan weekly sync", false).in_values(&values),
            Tri::Unknown
        );
        assert_eq!(words("sync plan", false).in_values(&values), Tri::No);
        assert_eq!(words("budget", false).in_values(&values), Tri::No);
        assert_eq!(words("", false).in_values(&values), Tri::No);
    }
}
