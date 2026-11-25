//! # Property
//!
//! An item's local state (STORAGE §4.3): one value under one key, keyed by
//! the item's public id, so a property follows the item into every
//! collection and account filing it, through its retention, its restore
//! and a rekey, and goes with the last row holding the seq. No source
//! carries one: the sync never reads them and never pushes them (SYNC.md
//! §3).
//!
//! A key is `<namespace>/<name>`; `pimdir/` is the standard's, whose one
//! name defined so far is [`LABEL`], an item's local labels, kept as
//! written and compared folded. Every other key compares its values byte
//! for byte. `change` is what the owner applies for a `set-property`
//! (§15.3) and what a reader's overlay folds (§15.4), so both agree.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

/// The key of an item's local labels (§4.3), matched folded.
pub const LABEL: &str = "pimdir/label";

/// The most bytes a key holds (§13).
pub(crate) const MAX_KEY_BYTES: usize = 64;

/// The most bytes of UTF-8 a value holds (§13).
pub(crate) const MAX_VALUE_BYTES: usize = 1024;

/// The most values an item holds under one key (§13).
pub(crate) const MAX_VALUES: usize = 64;

/// One value an item holds under one key (§4.3).
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PimdirProperty {
    /// The key, `<namespace>/<name>`.
    pub key: String,
    /// One value held under it.
    pub value: String,
}

/// A live placement whose item holds a property (§14.1).
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PimdirPropertyHolder {
    /// The collection the placement sits in.
    pub collection: String,
    /// The item's public id.
    pub seq: i64,
}

/// Whether `key` is one the store holds (§4.3, §13): `<namespace>/<name>`,
/// each part one character or more of `a` to `z`, `0` to `9`, `.`, `_` and
/// `-`, at most [`MAX_KEY_BYTES`] bytes. A key under `pimdir/` the standard
/// does not define is a key all the same: a reader reads it as any other.
pub(crate) fn is_key(key: &str) -> bool {
    let part = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
    };
    match key.split_once('/') {
        Some((namespace, name)) => key.len() <= MAX_KEY_BYTES && part(namespace) && part(name),
        None => false,
    }
}

/// Whether `key` is one a `set-property` may name (§15.3): a key of
/// [`is_key`], [`LABEL`] or one outside `pimdir/`, the names the standard
/// keeps for its later versions being written by no program.
pub(crate) fn is_writable_key(key: &str) -> bool {
    is_key(key) && (key == LABEL || !key.starts_with("pimdir/"))
}

/// Whether `value` is one the store holds (§4.3, §13): 1 to
/// [`MAX_VALUE_BYTES`] bytes of UTF-8 holding no NUL.
pub(crate) fn is_value(value: &str) -> bool {
    (1..=MAX_VALUE_BYTES).contains(&value.len()) && !value.contains('\0')
}

/// A label's fold (§4.3), as Annex A.2 folds a card's `fn`: lowercased by
/// the default full mapping of Unicode, then trimmed of White_Space.
pub fn fold(value: &str) -> String {
    value.to_lowercase().trim().to_string()
}

/// Whether two values of `key` are the same one (§4.3): folded under
/// [`LABEL`], byte for byte under any other key.
pub(crate) fn same(key: &str, a: &str, b: &str) -> bool {
    match key == LABEL {
        true => a == b || fold(a) == fold(b),
        false => a == b,
    }
}

/// The rows a `set-property` writes under its key (§15.3): the values held
/// it removes and the values it adds, each byte for byte, a value removed
/// and added again in the same spelling being in neither.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct PimdirPropertyChange {
    /// The values held that go, as the store spells them.
    pub removed: Vec<String>,
    /// The values that come, as the action spells them.
    pub added: Vec<String>,
}

impl PimdirPropertyChange {
    /// Whether the change writes nothing, a value held added or one not
    /// held removed: it stamps no row (§4.5).
    pub fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.added.is_empty()
    }
}

/// A `set-property` leaving more than [`MAX_VALUES`] values under its key,
/// which the owner parks as the schema refuses the 65th (§15.3).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PimdirPropertyOverflow {
    /// How many values the change would leave.
    pub values: usize,
}

/// Folds a `set-property` over the values `held` under `key` (§15.3): its
/// removes first, each taking the value held that is [`same`] as it, then
/// its adds in order, each adding nothing when a value held or added
/// before it is the same. So a value both name is held after, in the
/// spelling `add` gives, which is how a label is respelled.
///
/// Answers the rows to write, or the overflow when the values left under
/// the key would pass [`MAX_VALUES`].
pub(crate) fn change(
    key: &str,
    held: &[String],
    remove: &[String],
    add: &[String],
) -> Result<PimdirPropertyChange, PimdirPropertyOverflow> {
    let mut left: Vec<&String> = held
        .iter()
        .filter(|value| !remove.iter().any(|gone| same(key, gone, value)))
        .collect();
    for value in add {
        if !left.iter().any(|kept| same(key, kept, value)) {
            left.push(value);
        }
    }
    if left.len() > MAX_VALUES {
        return Err(PimdirPropertyOverflow { values: left.len() });
    }

    Ok(PimdirPropertyChange {
        removed: held
            .iter()
            .filter(|value| !left.contains(value))
            .cloned()
            .collect(),
        added: left
            .into_iter()
            .filter(|value| !held.contains(value))
            .cloned()
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use alloc::format;

    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn a_key_is_two_parts_of_a_closed_alphabet_within_its_bound() {
        for key in [
            "pimdir/label",
            "himalaya/snooze-until",
            "x.y_z-0/a.b_c-9",
            "pimdir/session-key",
        ] {
            assert!(is_key(key), "{key}");
        }
        let long = format!("himalaya/{}", "x".repeat(MAX_KEY_BYTES - 9));
        assert!(is_key(&long));
        for key in [
            "muted",
            "/muted",
            "himalaya/",
            "himalaya/muted/now",
            "Himalaya/muted",
            "himalaya/mu ted",
            "himalaya/café",
            &format!("{long}x"),
        ] {
            assert!(!is_key(key), "{key}");
        }
    }

    #[test]
    fn pimdir_keeps_its_namespace_but_for_the_label() {
        assert!(is_writable_key(LABEL));
        assert!(is_writable_key("himalaya/muted"));
        assert!(!is_writable_key("pimdir/session-key"));
        assert!(!is_writable_key("pimdir/labels"));
    }

    #[test]
    fn a_value_is_one_to_1024_bytes_without_nul() {
        assert!(is_value("a"));
        assert!(is_value(&"é".repeat(512)));
        assert!(!is_value(""));
        assert!(!is_value("a\0b"));
        assert!(!is_value(&format!("{}x", "é".repeat(512))));
    }

    #[test]
    fn a_label_compares_folded_and_any_other_key_byte_for_byte() {
        assert!(same(LABEL, "Receipts", " RECEIPTS\u{a0}"));
        assert!(same(LABEL, "RÉSUMÉ", "résumé"));
        assert!(!same("himalaya/tag", "Receipts", "receipts"));
        assert_eq!(fold("ΟΔΟΣ"), "οδος");
    }

    #[test]
    fn the_removes_apply_before_the_adds_so_a_label_is_respelled() {
        let held = strings(&["Receipts", "Tax"]);
        let change = change(
            LABEL,
            &held,
            &strings(&["receipts"]),
            &strings(&["receipts"]),
        );
        assert_eq!(
            change,
            Ok(PimdirPropertyChange {
                removed: strings(&["Receipts"]),
                added: strings(&["receipts"]),
            })
        );
    }

    #[test]
    fn a_folded_duplicate_adds_nothing_nor_does_a_value_held() {
        let held = strings(&["Receipts"]);
        let unchanged = change(LABEL, &held, &[], &strings(&["RECEIPTS", "Receipts"]));
        assert_eq!(unchanged, Ok(PimdirPropertyChange::default()));

        let added = change(LABEL, &held, &[], &strings(&["Tax", "TAX"]));
        assert_eq!(added.unwrap().added, strings(&["Tax"]));

        let bytes = change("himalaya/tag", &held, &[], &strings(&["RECEIPTS"]));
        assert_eq!(bytes.unwrap().added, strings(&["RECEIPTS"]));
    }

    #[test]
    fn a_value_not_held_removed_and_one_removed_then_added_again_change_nothing() {
        let held = strings(&["a"]);
        assert!(
            change(LABEL, &held, &strings(&["b"]), &[])
                .unwrap()
                .is_empty()
        );
        assert!(
            change("x/y", &held, &strings(&["a"]), &strings(&["a"]))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn the_65th_value_overflows_and_a_remove_makes_room() {
        let held: Vec<String> = (0..MAX_VALUES).map(|n| format!("{n}")).collect();
        assert_eq!(
            change("x/y", &held, &[], &strings(&["new"])),
            Err(PimdirPropertyOverflow { values: 65 })
        );
        assert!(change("x/y", &held, &strings(&["0"]), &strings(&["new"])).is_ok());
        assert!(
            change("x/y", &held, &[], &strings(&["0"]))
                .unwrap()
                .is_empty()
        );
    }
}
