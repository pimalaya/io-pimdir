//! # Calendar extraction
//!
//! A `text/calendar` body read through ical-rs (SEARCH §6).

use alloc::string::{String, ToString};

use ical::{param::IcalParam, tree::cst::IcalCst, value::IcalValue};

use crate::search::extract::{PimdirExtract, PimdirExtractStatus, person, push};

impl PimdirExtract {
    /// Extracts a `text/calendar` body through ical-rs, from every event,
    /// task and journal of the resource, overrides included: `SUMMARY` as
    /// the title, `ORGANIZER` and `ATTENDEE` with their `CN` as people and
    /// under their roles, `DESCRIPTION` as the body, the names `ATTACH`
    /// states as attachments, `LOCATION` as the place and `COMMENT` as the
    /// note. One no parser accepts is
    /// [`Unparseable`](PimdirExtractStatus::Unparseable) with every field
    /// empty, which the caller fills from the summary.
    pub fn calendar(body: &[u8]) -> Self {
        let Ok(cst) = IcalCst::parse(body) else {
            return Self {
                status: PimdirExtractStatus::Unparseable,
                ..Default::default()
            };
        };

        let mut extract = Self::default();
        let calendar = cst.decode();
        let items = calendar.components.iter().filter(|component| {
            matches!(
                component.name.to_ascii_uppercase().as_str(),
                "VEVENT" | "VTODO" | "VJOURNAL"
            )
        });
        for component in items {
            for prop in &component.props {
                let name = prop.name.to_ascii_uppercase();
                let text = ical_text(&prop.value);
                match name.as_str() {
                    "SUMMARY" => push(&mut extract.title, &text),
                    "DESCRIPTION" => push(&mut extract.body, &text),
                    "LOCATION" => push(&mut extract.place, &text),
                    "COMMENT" => push(&mut extract.note, &text),
                    "ORGANIZER" | "ATTENDEE" => {
                        let address = text
                            .get(..7)
                            .filter(|scheme| scheme.eq_ignore_ascii_case("mailto:"))
                            .map_or(text.as_str(), |_| &text[7..]);
                        let cn = prop.params.iter().find_map(|param| match param {
                            IcalParam::Cn(cn) => Some(cn.as_ref()),
                            _ => None,
                        });
                        let text = person(cn, address);
                        let field = match name.as_str() {
                            "ORGANIZER" => &mut extract.organizer,
                            _ => &mut extract.attendee,
                        };
                        push(field, &text);
                        push(&mut extract.people, &text);
                    }
                    // NOTE: its `FILENAME` (RFC 8607) and `FMTTYPE` (SEARCH
                    // §6), so `has:attachment` reads a non-empty field.
                    "ATTACH" => {
                        for param in &prop.params {
                            match param {
                                IcalParam::Unknown { name, values }
                                    if matches!(
                                        name.to_ascii_uppercase().as_str(),
                                        "FILENAME" | "X-FILENAME"
                                    ) =>
                                {
                                    for value in values {
                                        push(&mut extract.attachment, value);
                                    }
                                }
                                IcalParam::FmtType(media_type) => {
                                    push(&mut extract.attachment, media_type);
                                }
                                _ => (),
                            }
                        }
                    }
                    _ => (),
                }
            }
        }

        extract
    }
}

/// An iCalendar value as text: a text, a list's items on their own lines,
/// an address or a URI as written.
fn ical_text(value: &IcalValue) -> String {
    match value {
        IcalValue::Text(text) => text.0.to_string(),
        IcalValue::TextList(list) => {
            let mut text = String::new();
            for item in &list.0 {
                push(&mut text, item);
            }
            text
        }
        IcalValue::CalAddress(address) => address.0.to_string(),
        IcalValue::Uri(uri) => uri.0.to_string(),
        _ => String::new(),
    }
}
