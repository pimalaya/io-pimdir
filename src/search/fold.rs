//! # Fold
//!
//! The two folds of pimdir SEARCH.md. [`text`] is SEARCH §5's, run before
//! any tokenizer reads a text, in the index and in a query alike: every
//! character lowercased by the Unicode default full lowercase mapping
//! (STORAGE Annex A.2, Rust's `str::to_lowercase`), decomposed (NFD),
//! stripped of the combining marks of U+0300 to U+036F, and composed again
//! (NFC). The tokenizer then folds no diacritic of its own and no SQL
//! function is registered for it.
//!
//! [`value`] is STORAGE Annex A.2's, the fold `category:` compares both
//! sides by and the `title` and `sender` orders sort by (SEARCH §8,
//! §8.2): the same lowercase mapping, then trimmed, diacritics kept.

use alloc::string::{String, ToString};
#[cfg(any(feature = "mail", feature = "contact"))]
use alloc::vec::Vec;

use unicode_normalization::UnicodeNormalization;
#[cfg(any(feature = "mail", feature = "contact"))]
use unicode_normalization::char::is_combining_mark;

#[cfg(any(feature = "mail", feature = "contact"))]
use crate::search::query::PimdirQueryText;

/// A text folded under SEARCH §5: `Réunion`, `REUNION` and `réunion` fold
/// alike, and a mark outside U+0300 to U+036F (a Devanagari vowel sign, a
/// Hebrew point) is kept. ASCII text, which no normalisation moves, is
/// only lowercased.
pub fn text(value: &str) -> String {
    if value.is_ascii() {
        return value.to_ascii_lowercase();
    }
    value
        .to_lowercase()
        .nfd()
        .filter(|c| !matches!(c, '\u{300}'..='\u{36f}'))
        .nfc()
        .collect()
}

/// A value folded as STORAGE Annex A.2 folds `fn`: the Unicode default
/// full lowercase mapping, locale-independent, then trimmed.
pub fn value(value: &str) -> String {
    value.to_lowercase().trim().to_string()
}

/// Whether `column` holds the tokens of `text` consecutively, its last one
/// a prefix when the text is (SEARCH §8.5), both folded and split as the
/// index splits them.
#[cfg(any(feature = "mail", feature = "contact"))]
pub(crate) fn holds(column: &str, text: &PimdirQueryText) -> bool {
    let wanted = tokens(&self::text(&text.text));
    let Some((last, first)) = wanted.split_last() else {
        return false;
    };

    let held = tokens(&self::text(column));
    held.windows(wanted.len()).any(|window| {
        let (tail, head) = window.split_last().expect("a window of one token or more");
        head == first
            && match text.prefix {
                true => tail.starts_with(last.as_str()),
                false => tail == last,
            }
    })
}

/// The tokens of a folded text as FTS5's `unicode61` splits it, its
/// default categories: a run of letters, numbers and private-use
/// characters is a token, and any other character separates two.
#[cfg(any(feature = "mail", feature = "contact"))]
fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !token_char(c))
        .filter(|token| !token.is_empty())
        .map(String::from)
        .collect()
}

/// Whether `c` is of a general category `unicode61` keeps in a token by
/// default (`L*`, `N*`, `Co`): an alphanumeric character but a mark and
/// the symbols the Alphabetic property counts (the circled and squared
/// Latin letters), or a private-use one.
#[cfg(any(feature = "mail", feature = "contact"))]
fn token_char(c: char) -> bool {
    let symbol = matches!(c, '\u{24B6}'..='\u{24E9}' | '\u{1F130}'..='\u{1F189}');
    let private = matches!(
        c,
        '\u{E000}'..='\u{F8FF}' | '\u{F0000}'..='\u{FFFFD}' | '\u{100000}'..='\u{10FFFD}'
    );
    private || (c.is_alphanumeric() && !is_combining_mark(c) && !symbol)
}

#[cfg(test)]
mod tests {
    use crate::search::fold;

    #[test]
    fn case_and_the_combining_diacritics_fold_away() {
        assert_eq!(fold::text("Réunion"), "reunion");
        assert_eq!(fold::text("REUNION"), "reunion");
        assert_eq!(fold::text("re\u{301}union"), "reunion", "decomposed input");
        assert_eq!(
            fold::text("Budget Αθήνα"),
            "budget αθηνα",
            "the Greek tonos"
        );
    }

    #[test]
    fn a_letter_and_a_mark_of_another_block_are_kept() {
        assert_eq!(fold::text("Øresund Łódź"), "øresund łodz");
        assert_eq!(fold::text("कि"), "कि", "a Devanagari vowel sign");
    }

    #[test]
    fn a_category_folds_by_the_full_unicode_lowercase_mapping() {
        assert_eq!(fold::value(" ÉCOLE "), "école");
        assert_eq!(fold::value("İ"), "i\u{307}");
    }
}
