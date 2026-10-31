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
//! sides by and the `name` order sorts titles by (SEARCH §8, §8.2): the
//! same lowercase mapping, then trimmed, diacritics kept.

use alloc::string::{String, ToString};

use unicode_normalization::UnicodeNormalization;

/// A text folded under SEARCH §5: `Réunion`, `REUNION` and `réunion` fold
/// alike, and a mark outside U+0300 to U+036F (a Devanagari vowel sign, a
/// Hebrew point) is kept.
pub fn text(value: &str) -> String {
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
