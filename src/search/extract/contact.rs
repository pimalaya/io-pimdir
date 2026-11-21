//! # Contact extraction
//!
//! A `text/vcard` body read through vcard-rs (SEARCH §6).

use alloc::{borrow::Cow, string::String, string::ToString, vec, vec::Vec};

use vcard::{tree::cst::VcardCst, value::VcardValue};

use crate::search::{
    extract::{PimdirExtract, PimdirExtractStatus, push},
    fold,
};

impl PimdirExtract {
    /// Extracts a `text/vcard` body through vcard-rs: `FN`, `N` and
    /// `NICKNAME` as the title, every `EMAIL` and the `FN` as people (the
    /// addresses also under `email`), `ADR` as the place, `ORG`, `TITLE`
    /// and `ROLE` as the organisation, `NOTE` and every `TEL`, as written
    /// and as its digits alone, as the note. One no parser accepts is
    /// [`Unparseable`](PimdirExtractStatus::Unparseable) with every field
    /// empty, which the caller fills from the summary.
    pub fn contact(body: &[u8]) -> Self {
        let Ok(cst) = VcardCst::parse(body) else {
            return Self {
                status: PimdirExtractStatus::Unparseable,
                ..Default::default()
            };
        };

        let mut extract = Self::default();
        for prop in cst.decode().properties {
            let name = prop.name.to_ascii_uppercase();
            let text = vcard_text(&prop.value);
            match name.as_str() {
                "FN" => {
                    push(&mut extract.title, &text);
                    push(&mut extract.people, &text);
                }
                "N" | "NICKNAME" => push(&mut extract.title, &text),
                "EMAIL" => {
                    push(&mut extract.email, &text);
                    push(&mut extract.people, &text);
                }
                "ADR" => {
                    extract.adr = true;
                    push(&mut extract.place, &text);
                }
                "BDAY" => extract.bday = true,
                "PHOTO" => extract.photo = true,
                "CATEGORIES" => {
                    for value in vcard_values(&prop.value) {
                        let folded = fold::value(&value);
                        if !folded.is_empty() && !extract.categories.contains(&folded) {
                            extract.categories.push(folded);
                        }
                    }
                }
                "ORG" | "TITLE" | "ROLE" => push(&mut extract.org, &text),
                "NOTE" => push(&mut extract.note, &text),
                "TEL" => {
                    extract.tel = true;
                    let number = text.strip_prefix("tel:").unwrap_or(&text);
                    let digits: String = number.chars().filter(char::is_ascii_digit).collect();
                    push(&mut extract.note, number);
                    if digits != number {
                        push(&mut extract.note, &digits);
                    }
                }
                _ => (),
            }
        }

        extract
    }
}

/// A vCard value's items: a list's values, a text as one, anything else
/// none (the values of a `CATEGORIES`, RFC 6350 §6.7.1).
fn vcard_values(value: &VcardValue) -> Vec<String> {
    match value {
        VcardValue::TextList(list) => list.0.iter().map(|value| value.to_string()).collect(),
        VcardValue::Text(value) => vec![value.0.to_string()],
        _ => Vec::new(),
    }
}

/// A vCard value as text: a text, each component of a list or a
/// structured value on its own line, a URI as written.
fn vcard_text(value: &VcardValue) -> String {
    let mut text = String::new();
    let lists: Vec<&Vec<Cow<str>>> = match value {
        VcardValue::Text(value) => return value.0.to_string(),
        VcardValue::Uri(value) => return value.0.to_string(),
        VcardValue::TextList(list) => vec![&list.0],
        VcardValue::Org(org) => vec![&org.0],
        VcardValue::N(n) => vec![&n.prefixes, &n.given, &n.additional, &n.family, &n.suffixes],
        VcardValue::Adr(adr) => vec![
            &adr.po_box,
            &adr.extended,
            &adr.street,
            &adr.locality,
            &adr.region,
            &adr.postal_code,
            &adr.country,
        ],
        _ => Vec::new(),
    };
    for value in lists.into_iter().flatten() {
        push(&mut text, value);
    }
    text
}
