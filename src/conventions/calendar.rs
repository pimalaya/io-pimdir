//! The `text/calendar` conventions (spec Annex A.3).
//!
//! The item is the calendar object resource, not the component: RFC 4791 §4.1
//! keeps every component sharing a `UID` in one resource, so a recurring series
//! and its overrides are one item, summarised from the master (the component
//! carrying no `RECURRENCE-ID`).
//!
//! Times are carried verbatim beside the `TZID` naming their zone, so a reader
//! holding a zone database re-derives an instant in its own zone and one
//! holding none shows the wall time the calendar wrote. The single resolved
//! projection is the sort key, which is why this module resolves a zone at all:
//! it reads the `VTIMEZONE` the resource carries, and nothing else.

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use io_replica::placement::{ReplicaLinkId, ReplicaMeta, ReplicaSortKey};
use serde::{Deserialize, Serialize};

use crate::conventions::{PimdirDerivation, fnv1a64, time, unfold};

/// The `text/calendar` summary (spec Annex A.3), `v: 1`.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PimdirCalendarMeta {
    /// The convention version, `1` today.
    pub v: u8,
    /// The resource's `UID`, verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
    /// What a reader renders the resource as: `VEVENT`, `VTODO` or `VJOURNAL`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    /// The master's `SUMMARY`. Required, and may be empty.
    #[serde(default)]
    pub summary: String,
    /// The `LOCATION`, verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// `DTSTART`, the value verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dtstart: Option<String>,
    /// The `TZID` parameter naming the zone `DTSTART` is local to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dtstart_tzid: Option<String>,
    /// Whether `DTSTART` is a `date-time` or a `date`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dtstart_value: Option<String>,
    /// `DTEND`, the value verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dtend: Option<String>,
    /// `DUE`, the value verbatim. A `VTODO` alone carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    /// Whether the resource carries an `RRULE` or an `RDATE`. Written even
    /// when false, since a reader planning an expansion has to tell "no rule"
    /// from "not examined".
    #[serde(default)]
    pub recurring: bool,
    /// The `RRULE`'s `UNTIL`, verbatim, when the series is bounded by one.
    /// With `dtstart` it brackets the series, so a date range can drop the
    /// item without materialising an occurrence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// The raw resource octets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// Derives a calendar resource's link id, summary and sort key.
///
/// The link id is the `UID`, which names the same object across sources (RFC
/// 5545 §3.8.4.7). A resource carrying none falls back to `hash:` over the
/// body, the same way a card with no `UID` does.
pub fn derive(body: &[u8]) -> PimdirDerivation {
    let components = parse(&unfold(body, false));
    let zones: Vec<&Component> = components
        .iter()
        .flat_map(|component| component.children.iter())
        .filter(|child| child.name == "VTIMEZONE")
        .collect();
    let master = master(&components);

    let uid = master.and_then(|master| master.value("UID"));
    let dtstart = master.and_then(|master| master.property("DTSTART"));
    let due = master.and_then(|master| master.value("DUE"));
    let component = master.map(|master| master.name.clone());
    let rrule = master.and_then(|master| master.value("RRULE"));

    let key = match (component.as_deref(), &due) {
        // NOTE: a VTODO is scheduled by its due date and need not carry a
        // DTSTART at all (RFC 5545 §3.8.2.3), so DUE decides its key.
        (Some("VTODO"), Some(due)) => master
            .and_then(|master| master.property("DUE"))
            .and_then(|due_property| sort_key(due_property, &zones))
            .or_else(|| sort_key_of(due, None, false, &zones)),
        _ => dtstart.and_then(|dtstart| sort_key(dtstart, &zones)),
    };

    let link_id = match &uid {
        Some(uid) => uid.clone(),
        None => format!("hash:{}", fnv1a64(body)),
    };

    let meta = PimdirCalendarMeta {
        v: 1,
        uid,
        component,
        summary: master
            .and_then(|master| master.value("SUMMARY"))
            .unwrap_or_default(),
        location: master.and_then(|master| master.value("LOCATION")),
        dtstart: dtstart.map(|dtstart| dtstart.value.clone()),
        dtstart_tzid: dtstart.and_then(|dtstart| dtstart.param("TZID")),
        dtstart_value: dtstart.map(|dtstart| {
            match dtstart.param("VALUE").as_deref() {
                Some("DATE") => "date",
                _ => "date-time",
            }
            .to_string()
        }),
        dtend: master.and_then(|master| master.value("DTEND")),
        due,
        recurring: rrule.is_some() || master.is_some_and(|master| master.value("RDATE").is_some()),
        until: rrule.as_deref().and_then(until),
        size: Some(body.len() as u64),
    };

    PimdirDerivation {
        link_id: ReplicaLinkId(link_id),
        meta: ReplicaMeta(serde_json::to_string(&meta).unwrap_or_default()),
        sort_key: ReplicaSortKey(key.unwrap_or_default()),
    }
}

/// The component a summary describes: the first `VEVENT`, `VTODO` or
/// `VJOURNAL` carrying no `RECURRENCE-ID`, which is the master of the set.
fn master(components: &[Component]) -> Option<&Component> {
    let candidates = components
        .iter()
        .flat_map(|component| component.children.iter())
        .filter(|child| matches!(child.name.as_str(), "VEVENT" | "VTODO" | "VJOURNAL"));

    let mut first = None;
    for candidate in candidates {
        if candidate.value("RECURRENCE-ID").is_none() {
            return Some(candidate);
        }
        first.get_or_insert(candidate);
    }

    first
}

/// The `UNTIL` of a recurrence rule, verbatim.
fn until(rrule: &str) -> Option<String> {
    rrule.split(';').find_map(|part| {
        let (name, value) = part.split_once('=')?;
        name.eq_ignore_ascii_case("UNTIL")
            .then(|| value.to_string())
    })
}

/// The sort key a start property resolves to.
fn sort_key(property: &Property, zones: &[&Component]) -> Option<String> {
    sort_key_of(
        &property.value,
        property.param("TZID").as_deref(),
        property.param("VALUE").as_deref() == Some("DATE"),
        zones,
    )
}

/// The instant a start value names, normalised to RFC 3339 in UTC.
///
/// Only a UTC date-time is an instant already; the rest are the conventions
/// Annex A.3 fixes, and the last three are conventions rather than facts:
/// a date-only value reads as midnight UTC, a floating one reads its wall time
/// as UTC, and a zone that will not resolve is read as floating rather than
/// dropped, since the error is then bounded by an offset where an empty key
/// would move the item to the far end of the listing.
fn sort_key_of(
    value: &str,
    tzid: Option<&str>,
    date_only: bool,
    zones: &[&Component],
) -> Option<String> {
    let bytes = value.as_bytes();
    let year = time::digits(bytes, 0, 4)? as i32;
    let month = time::digits(bytes, 4, 2)?;
    let day = time::digits(bytes, 6, 2)?;

    if date_only || bytes.len() == 8 {
        return Some(time::rfc3339(time::unix(year, month, day, 0, 0, 0)));
    }
    if bytes.get(8) != Some(&b'T') {
        return None;
    }

    let hour = time::digits(bytes, 9, 2)?;
    let minute = time::digits(bytes, 11, 2)?;
    let second = time::digits(bytes, 13, 2)?;
    let local = time::unix(year, month, day, hour, minute, second);

    if bytes.get(15) == Some(&b'Z') {
        return Some(time::rfc3339(local));
    }

    let zone = tzid.and_then(|tzid| {
        zones
            .iter()
            .find(|zone| zone.value("TZID").as_deref() == Some(tzid))
    });

    match zone {
        Some(zone) => Some(time::rfc3339(resolve(zone, local, year))),
        None => Some(time::rfc3339(local)),
    }
}

/// The instant a wall time names in a zone the resource carries.
///
/// A local time at a transition names two instants or none, and Annex A.3
/// fixes both: the ambiguous hour takes the offset in effect *before* the
/// transition, and the hour that does not exist takes the one in effect
/// *after*. Both are the numerically greater of the two offsets, so both are
/// the earlier instant, which is why one rule serves both.
fn resolve(zone: &Component, local: i64, year: i32) -> i64 {
    let transitions = transitions(zone, year);
    if transitions.is_empty() {
        return local;
    }

    let mut valid: Option<i64> = None;
    let mut earliest: Option<i64> = None;
    for offset in offsets(&transitions) {
        let instant = local - offset;
        earliest = Some(earliest.map_or(instant, |current: i64| current.min(instant)));
        if in_effect(&transitions, instant) == offset {
            valid = Some(valid.map_or(instant, |current: i64| current.min(instant)));
        }
    }

    valid.or(earliest).unwrap_or(local)
}

/// One offset change a `VTIMEZONE` states.
struct Transition {
    /// The instant the change takes effect.
    at: i64,
    /// The offset from UTC in effect after it.
    to: i64,
    /// The offset in effect before it, which is what places the first one.
    from: i64,
}

/// The transitions a zone states over the years around `year`.
///
/// Bounded to that window on purpose: a `VTIMEZONE` states its rules as
/// recurrences with no end, and the only ones that can decide a wall time are
/// the ones around it. It reads `FREQ=YEARLY` with `BYMONTH` and `BYDAY`, which
/// is what every zone written by a real calendar carries; a subcomponent whose
/// rule this cannot read contributes its `DTSTART` alone, which is still the
/// right answer for a zone that never changes.
fn transitions(zone: &Component, year: i32) -> Vec<Transition> {
    let mut transitions = Vec::new();

    for change in &zone.children {
        if !matches!(change.name.as_str(), "STANDARD" | "DAYLIGHT") {
            continue;
        }

        let (Some(from), Some(to)) = (
            change.value("TZOFFSETFROM").as_deref().and_then(offset),
            change.value("TZOFFSETTO").as_deref().and_then(offset),
        ) else {
            continue;
        };
        let Some(start) = change.value("DTSTART") else {
            continue;
        };
        let bytes = start.as_bytes();
        let (Some(hour), Some(minute), Some(second)) = (
            time::digits(bytes, 9, 2),
            time::digits(bytes, 11, 2),
            time::digits(bytes, 13, 2),
        ) else {
            continue;
        };

        match change.value("RRULE").as_deref().and_then(yearly) {
            Some((month, ordinal, weekday)) => {
                for year in year - 1..=year + 1 {
                    let Some(day) = nth_weekday(year, month, ordinal, weekday) else {
                        continue;
                    };
                    transitions.push(Transition {
                        at: time::unix(year, month, day, hour, minute, second) - from,
                        to,
                        from,
                    });
                }
            }
            None => {
                let (Some(year), Some(month), Some(day)) = (
                    time::digits(bytes, 0, 4),
                    time::digits(bytes, 4, 2),
                    time::digits(bytes, 6, 2),
                ) else {
                    continue;
                };
                transitions.push(Transition {
                    at: time::unix(year as i32, month, day, hour, minute, second) - from,
                    to,
                    from,
                });
            }
        }
    }

    transitions.sort_by_key(|transition| transition.at);
    transitions
}

/// Every offset the window's transitions put in effect, the one they started
/// from included.
fn offsets(transitions: &[Transition]) -> Vec<i64> {
    let mut offsets = Vec::new();
    for offset in transitions
        .first()
        .map(|first| first.from)
        .into_iter()
        .chain(transitions.iter().map(|transition| transition.to))
    {
        if !offsets.contains(&offset) {
            offsets.push(offset);
        }
    }

    offsets
}

/// The offset in effect at an instant: the last transition at or before it,
/// else what the first one moved away from.
fn in_effect(transitions: &[Transition], instant: i64) -> i64 {
    transitions
        .iter()
        .rfind(|transition| transition.at <= instant)
        .map(|transition| transition.to)
        .unwrap_or_else(|| transitions.first().map(|first| first.from).unwrap_or(0))
}

/// A `TZOFFSETFROM`/`TZOFFSETTO` value (`+hhmm[ss]`) in seconds.
fn offset(raw: &str) -> Option<i64> {
    let bytes = raw.trim().as_bytes();
    if !matches!(bytes.first(), Some(b'+' | b'-')) {
        return None;
    }

    let hours = time::digits(bytes, 1, 2)? as i64;
    let minutes = time::digits(bytes, 3, 2)? as i64;
    let seconds = time::digits(bytes, 5, 2).unwrap_or(0) as i64;
    let offset = hours * 3_600 + minutes * 60 + seconds;

    Some(if bytes[0] == b'-' { -offset } else { offset })
}

/// The `(month, ordinal, weekday)` a yearly transition rule names, when it is
/// one this reads.
fn yearly(rrule: &str) -> Option<(u32, i32, u32)> {
    let mut yearly = false;
    let mut month = None;
    let mut byday = None;

    for part in rrule.split(';') {
        let Some((name, value)) = part.split_once('=') else {
            continue;
        };
        match name.to_uppercase().as_str() {
            "FREQ" => yearly = value.eq_ignore_ascii_case("YEARLY"),
            "BYMONTH" => month = value.parse::<u32>().ok(),
            "BYDAY" => byday = Some(value.to_uppercase()),
            _ => {}
        }
    }

    if !yearly {
        return None;
    }

    let byday = byday?;
    let (ordinal, day) = byday.split_at(byday.len().checked_sub(2)?);
    let weekday = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"]
        .iter()
        .position(|name| *name == day)? as u32;

    Some((month?, ordinal.parse().unwrap_or(1), weekday))
}

/// The day of the month the `n`th weekday falls on, counting from the end when
/// `n` is negative.
fn nth_weekday(year: i32, month: u32, ordinal: i32, weekday: u32) -> Option<u32> {
    let length = time::days_in_month(year, month);
    if length == 0 || ordinal == 0 {
        return None;
    }

    let day = if ordinal > 0 {
        let first = time::days_from_civil(year, month, 1);
        let shift = (7 + weekday - time::weekday(first)) % 7;
        1 + shift as i32 + (ordinal - 1) * 7
    } else {
        let last = time::days_from_civil(year, month, length);
        let shift = (7 + time::weekday(last) - weekday) % 7;
        length as i32 - shift as i32 + (ordinal + 1) * 7
    };

    (1..=length as i32).contains(&day).then_some(day as u32)
}

/// One `BEGIN`/`END` block and what it holds.
struct Component {
    name: String,
    properties: Vec<Property>,
    children: Vec<Component>,
}

impl Component {
    /// The value of the first occurrence of a property.
    fn value(&self, name: &str) -> Option<String> {
        self.property(name).map(|property| property.value.clone())
    }

    /// The first occurrence of a property, parameters included.
    fn property(&self, name: &str) -> Option<&Property> {
        self.properties
            .iter()
            .find(|property| property.name == name)
    }
}

/// One content line: `name *(";" param "=" value) ":" value`.
struct Property {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

impl Property {
    /// A parameter's value, upper-cased names and verbatim values.
    fn param(&self, name: &str) -> Option<String> {
        self.params
            .iter()
            .find(|(param, _)| param == name)
            .map(|(_, value)| value.clone())
    }
}

/// Reads unfolded lines into the components they open and close.
///
/// A block whose `END` never arrives still closes at the end of the body,
/// which is what lets a truncated resource summarise as much as it carries.
fn parse(lines: &[String]) -> Vec<Component> {
    let mut roots = Vec::new();
    let mut stack: Vec<Component> = Vec::new();

    for line in lines {
        let Some((head, value)) = line.split_once(':') else {
            continue;
        };
        let mut parts = head.split(';');
        let name = parts.next().unwrap_or_default().trim().to_uppercase();

        match name.as_str() {
            "BEGIN" => stack.push(Component {
                name: value.trim().to_uppercase(),
                properties: Vec::new(),
                children: Vec::new(),
            }),
            "END" => {
                let Some(closed) = stack.pop() else { continue };
                match stack.last_mut() {
                    Some(parent) => parent.children.push(closed),
                    None => roots.push(closed),
                }
            }
            _ => {
                let Some(component) = stack.last_mut() else {
                    continue;
                };
                component.properties.push(Property {
                    name,
                    params: parts
                        .filter_map(|param| param.split_once('='))
                        .map(|(param, value)| {
                            (param.trim().to_uppercase(), value.trim().to_string())
                        })
                        .collect(),
                    value: value.to_string(),
                });
            }
        }
    }

    while let Some(unclosed) = stack.pop() {
        match stack.last_mut() {
            Some(parent) => parent.children.push(unclosed),
            None => roots.push(unclosed),
        }
    }

    roots
}
