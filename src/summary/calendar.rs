//! # Calendar summaries
//!
//! The `text/calendar` derivations (Annex A.3 to A.5): the item is the
//! resource, summarised from the master component, one row in the table
//! of its type. Times are carried verbatim with the parameters that give
//! them meaning; the one resolved instant is the sort key, read through
//! the `VTIMEZONE` the resource itself carries, else the IANA time zone
//! database `jiff` bundles, the one the index expands occurrences through.

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use jiff::{
    civil,
    tz::{AmbiguousOffset, TimeZone},
};

use crate::{
    placement::PimdirSortKey,
    summary::{
        PimdirAddress, PimdirDerivation, PimdirSummary,
        contact::{PimdirContentLine, split_unescaped, split_unquoted, split_value, unescape},
        logical_lines, time, uid_key, unfold,
    },
};

/// A `DTSTART` or `DUE` value with the parameters that give it meaning.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirTime {
    /// The value verbatim.
    pub value: String,
    /// The `TZID` parameter naming the zone the value is local to.
    pub tzid: Option<String>,
    /// Whether the value is a `date` rather than a `date-time`.
    pub date: bool,
}

impl PimdirTime {
    /// The `dtstart_value` spelling: `date` or `date-time`.
    pub fn value_kind(&self) -> &'static str {
        if self.date { "date" } else { "date-time" }
    }
}

/// The `event_summary` row of a `VEVENT` resource with its people.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirEventSummary {
    /// The `UID` verbatim.
    pub uid: Option<String>,
    /// The master's `SUMMARY`, unescaped; empty when absent.
    pub summary: String,
    /// The `LOCATION`, unescaped.
    pub location: Option<String>,
    /// The master's `DTSTART`.
    pub dtstart: Option<PimdirTime>,
    /// The `DTEND` verbatim; a `DURATION` is not resolved.
    pub dtend: Option<String>,
    /// Whether the master carries an `RRULE` or `RDATE`; `None` when not examined.
    pub recurring: Option<bool>,
    /// The `RRULE`'s `UNTIL` verbatim.
    pub until: Option<String>,
    /// The `ORGANIZER`.
    pub organizer: Option<PimdirAddress>,
    /// Every `ATTENDEE`, in document order.
    pub attendees: Vec<PimdirAddress>,
}

/// The `task_summary` row of a `VTODO` resource with its people.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirTaskSummary {
    /// The `UID` verbatim.
    pub uid: Option<String>,
    /// The master's `SUMMARY`, unescaped; empty when absent.
    pub summary: String,
    /// The master's `DTSTART`.
    pub dtstart: Option<PimdirTime>,
    /// The `DUE`.
    pub due: Option<PimdirTime>,
    /// The `STATUS` uppercased verbatim.
    pub status: Option<String>,
    /// The `COMPLETED` verbatim, always UTC per RFC 5545.
    pub completed: Option<String>,
    /// The `PERCENT-COMPLETE`.
    pub percent: Option<i64>,
    /// Whether the master carries an `RRULE` or `RDATE`; `None` when not examined.
    pub recurring: Option<bool>,
    /// The `RRULE`'s `UNTIL` verbatim.
    pub until: Option<String>,
    /// The `PRIORITY`, 0 to 9; `None` when absent or out of range.
    pub priority: Option<i64>,
    /// Every `CATEGORIES` value in document order, decoded.
    pub categories: Vec<String>,
    /// The first `RELATED-TO` of type `PARENT`, or of no type, verbatim.
    pub parent: Option<String>,
    /// The message id the first `mid:` URI of an `ATTACH` or `URL` names.
    pub mid: Option<String>,
    /// When it was captured: `CREATED` as an instant on Annex A.3's rules,
    /// else `DTSTAMP` as one, `None` when neither reads (A.4, revision 2).
    pub created: Option<String>,
    /// The `ORGANIZER`.
    pub organizer: Option<PimdirAddress>,
    /// Every `ATTENDEE`, in document order.
    pub attendees: Vec<PimdirAddress>,
    /// Every `CONTACT`, in document order: who the task waits on (A.4,
    /// revision 2), filed under the `contact` role.
    pub contacts: Vec<PimdirAddress>,
}

/// The `journal_summary` row of a `VJOURNAL` resource with its people.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirJournalSummary {
    /// The `UID` verbatim.
    pub uid: Option<String>,
    /// The master's `SUMMARY`, unescaped; empty when absent.
    pub summary: String,
    /// The master's `DTSTART`.
    pub dtstart: Option<PimdirTime>,
    /// The `ORGANIZER`.
    pub organizer: Option<PimdirAddress>,
    /// Every `ATTENDEE`, in document order.
    pub attendees: Vec<PimdirAddress>,
}

/// Derives a resource's key, summary and sort key from its bytes.
///
/// The master is the first `VEVENT`, `VTODO` or `VJOURNAL` carrying no
/// `RECURRENCE-ID`. A resource holding none of the three is an item
/// with a body and no summary row (Annex A.3).
pub fn derive(body: &[u8]) -> PimdirDerivation {
    let components = parse(&unfold(body, false));
    let zones: Vec<&Component> = components
        .iter()
        .flat_map(|component| component.children.iter())
        .filter(|child| child.name == "VTIMEZONE")
        .collect();
    let master = master(&components);

    let uid = master
        .or_else(|| {
            components
                .iter()
                .flat_map(|component| component.children.iter())
                .find(|child| child.name != "VTIMEZONE")
        })
        .and_then(|component| component.value("UID"));
    let link_id = uid_key(uid.as_deref(), body);

    let Some(master) = master else {
        return PimdirDerivation {
            link_id,
            summary: None,
            sort_key: PimdirSortKey::default(),
        };
    };

    let text = |name: &str| master.value(name).map(|value| unescape(&value));
    let dtstart = master.line("DTSTART").map(time_of);
    let rrule = master.value("RRULE");
    let recurring = Some(rrule.is_some() || master.value("RDATE").is_some());
    let until = rrule.as_deref().and_then(until);
    let organizer = master.line("ORGANIZER").map(person);
    let attendees = master
        .lines
        .iter()
        .filter(|line| line.is("ATTENDEE"))
        .map(person)
        .collect();
    let summary_text = text("SUMMARY").unwrap_or_default();

    let (summary, key) = match master.name.as_str() {
        "VEVENT" => (
            PimdirSummary::Event(PimdirEventSummary {
                uid,
                summary: summary_text,
                location: text("LOCATION"),
                dtend: master.value("DTEND"),
                recurring,
                until,
                organizer,
                attendees,
                dtstart: dtstart.clone(),
            }),
            dtstart.as_ref().and_then(|start| instant(start, &zones)),
        ),
        "VTODO" => {
            let due = master.line("DUE").map(time_of);
            let key = due
                .as_ref()
                .and_then(|due| instant(due, &zones))
                .or_else(|| dtstart.as_ref().and_then(|start| instant(start, &zones)));
            (
                PimdirSummary::Task(PimdirTaskSummary {
                    uid,
                    summary: summary_text,
                    dtstart,
                    due,
                    status: master
                        .value("STATUS")
                        .map(|status| status.trim().to_uppercase()),
                    completed: master.value("COMPLETED"),
                    percent: master
                        .value("PERCENT-COMPLETE")
                        .and_then(|percent| percent.trim().parse().ok()),
                    recurring,
                    until,
                    priority: master
                        .value("PRIORITY")
                        .and_then(|priority| priority.trim().parse().ok())
                        .filter(|priority| (0..=9).contains(priority)),
                    categories: categories(master),
                    parent: parent(master),
                    mid: mid(master),
                    created: captured(master, &zones),
                    contacts: contacts(master),
                    organizer,
                    attendees,
                }),
                key,
            )
        }
        _ => (
            PimdirSummary::Journal(PimdirJournalSummary {
                uid,
                summary: summary_text,
                organizer,
                attendees,
                dtstart: dtstart.clone(),
            }),
            dtstart.as_ref().and_then(|start| instant(start, &zones)),
        ),
    };

    PimdirDerivation {
        link_id,
        summary: Some(summary),
        sort_key: PimdirSortKey(key.unwrap_or_default()),
    }
}

/// A time property with the parameters the summary keeps.
fn time_of(line: &PimdirContentLine) -> PimdirTime {
    PimdirTime {
        value: line.value.clone(),
        tzid: line.param("TZID").map(String::from),
        date: line
            .param("VALUE")
            .is_some_and(|value| value.eq_ignore_ascii_case("DATE")),
    }
}

/// An `ORGANIZER` or `ATTENDEE`: the `mailto:` stripped and the value
/// canonical, the `CN` parameter decoded as the name (Annex A.6).
fn person(line: &PimdirContentLine) -> PimdirAddress {
    PimdirAddress {
        address: PimdirAddress::canonical(&line.value),
        name: line.param("CN").map(unescape),
    }
}

/// When a task was captured (A.4, revision 2): `CREATED`, else `DTSTAMP`,
/// each as an instant on A.3's rules, `None` when neither reads as one.
fn captured(master: &Component, zones: &[&Component]) -> Option<String> {
    ["CREATED", "DTSTAMP"].iter().find_map(|name| {
        master
            .line(name)
            .and_then(|line| instant(&time_of(line), zones))
    })
}

/// Every `CONTACT` (RFC 5545 §3.8.4.2), in document order (A.4, revision
/// 2): the address its `ALTREP` names when that is a `mailto:` URI, else
/// its text, canonical either way (A.6); the text decoded as the name, none
/// when the address was read from a text that is an addr-spec. One with an
/// empty text and no such `ALTREP` names nobody.
fn contacts(master: &Component) -> Vec<PimdirAddress> {
    master
        .lines
        .iter()
        .filter(|line| line.is("CONTACT"))
        .filter_map(|line| {
            let text = unescape(&line.value);
            let mailto = line.param("ALTREP").filter(|altrep| {
                altrep
                    .get(..7)
                    .is_some_and(|scheme| scheme.eq_ignore_ascii_case("mailto:"))
            });
            match mailto {
                Some(altrep) => Some(PimdirAddress {
                    address: PimdirAddress::canonical(altrep),
                    name: (!text.is_empty()).then_some(text),
                }),
                None if text.is_empty() => None,
                None => Some(PimdirAddress {
                    address: PimdirAddress::canonical(&text),
                    name: (!addr_spec(&text)).then_some(text),
                }),
            }
        })
        .collect()
}

/// Whether a text is a bare addr-spec: one `@` between two runs holding no
/// space, `@`, `<` or `>`.
fn addr_spec(text: &str) -> bool {
    let bare = |part: &str| {
        !part.is_empty()
            && !part
                .chars()
                .any(|char| char.is_whitespace() || matches!(char, '@' | '<' | '>'))
    };
    text.split_once('@')
        .is_some_and(|(local, domain)| bare(local) && bare(domain))
}

/// Every `CATEGORIES` value in document order: each property's list split
/// on the commas no backslash escapes, decoded, empty values dropped.
fn categories(master: &Component) -> Vec<String> {
    master
        .lines
        .iter()
        .filter(|line| line.is("CATEGORIES"))
        .flat_map(|line| split_unescaped(&line.value, ','))
        .map(unescape)
        .filter(|category| !category.is_empty())
        .collect()
}

/// The first `RELATED-TO` naming a parent: of type `PARENT`, or of no type,
/// RFC 5545 §3.2.15's default.
fn parent(master: &Component) -> Option<String> {
    master
        .lines
        .iter()
        .filter(|line| line.is("RELATED-TO"))
        .find(|line| {
            line.param("RELTYPE")
                .is_none_or(|reltype| reltype.eq_ignore_ascii_case("PARENT"))
        })
        .map(|line| line.value.clone())
}

/// The message the first `mid:` URI (RFC 2392) of an `ATTACH` or `URL`
/// names: its message-id part, before any `/`, percent-decoded.
fn mid(master: &Component) -> Option<String> {
    let uri = master.lines.iter().find_map(|line| {
        if !line.is("ATTACH") && !line.is("URL") {
            return None;
        }
        let scheme = line.value.get(..4)?;
        scheme
            .eq_ignore_ascii_case("mid:")
            .then(|| &line.value[4..])
    })?;
    let id = uri.split('/').next().unwrap_or_default();
    Some(percent_decoded(id))
}

/// A URI part with its `%XX` escapes decoded (RFC 3986 §2.1); an escape
/// that is not one is kept as written.
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let escape = (bytes[at] == b'%')
            .then(|| text.get(at + 1..at + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escape {
            Some(byte) => {
                out.push(byte);
                at += 3;
            }
            None => {
                out.push(bytes[at]);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The component a summary describes: the first `VEVENT`, `VTODO` or
/// `VJOURNAL` carrying no `RECURRENCE-ID`, else the first override.
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

/// The instant a start names, normalised to RFC 3339 in UTC (Annex A.3).
///
/// A UTC date-time is an instant already. A zoned one resolves through
/// the first that names its `TZID` byte for byte: the resource's own
/// `VTIMEZONE`, then the IANA time zone database bundled with `jiff`;
/// one neither names reads as floating. A date-only value reads as
/// midnight UTC; a floating one reads its wall time as UTC.
fn instant(time: &PimdirTime, zones: &[&Component]) -> Option<String> {
    let bytes = time.value.as_bytes();
    let year = time::digits(bytes, 0, 4)? as i32;
    let month = time::digits(bytes, 4, 2)?;
    let day = time::digits(bytes, 6, 2)?;

    if time.date || bytes.len() == 8 {
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
    let Some(tzid) = time.tzid.as_deref() else {
        return Some(time::rfc3339(local));
    };

    let zone = zones
        .iter()
        .find(|zone| zone.value("TZID").as_deref() == Some(tzid));
    let instant = match zone {
        Some(zone) => resolve(zone, local),
        None => iana(tzid, local).unwrap_or(local),
    };

    Some(time::rfc3339(instant))
}

/// The instant a wall time names in a zone of the IANA time zone database
/// (Annex A.3, RFC 7809), or `None` when the database names no zone
/// `tzid`, matched byte for byte. A wall time a transition repeats or
/// skips takes the offset in effect before it (RFC 5545 §3.3.5).
pub(crate) fn iana(tzid: &str, local: i64) -> Option<i64> {
    let zone = TimeZone::get(tzid)
        .ok()
        .filter(|zone| zone.iana_name() == Some(tzid))?;

    let (year, month, day) = time::civil_from_days(local.div_euclid(86_400));
    let seconds = local.rem_euclid(86_400);
    let wall = civil::DateTime::new(
        i16::try_from(year).ok()?,
        i8::try_from(month).ok()?,
        i8::try_from(day).ok()?,
        (seconds / 3_600) as i8,
        (seconds % 3_600 / 60) as i8,
        (seconds % 60) as i8,
        0,
    )
    .ok()?;

    let offset = match zone.to_ambiguous_timestamp(wall).offset() {
        AmbiguousOffset::Unambiguous { offset } => offset,
        AmbiguousOffset::Gap { before, .. } | AmbiguousOffset::Fold { before, .. } => before,
    };

    Some(local - i64::from(offset.seconds()))
}

/// The instant a wall time names in a zone the resource carries.
///
/// A wall time a transition repeats or skips takes the offset in effect
/// before it (RFC 5545 §3.3.5): of a repeated one the earlier instant, a
/// skipped one read at the offset before the gap. Any other takes the one
/// offset in effect at the instant it names.
fn resolve(zone: &Component, local: i64) -> i64 {
    let transitions = transitions(zone, local);
    if transitions.is_empty() {
        return local;
    }

    for transition in &transitions {
        let before = transition.at + transition.from;
        let after = transition.at + transition.to;
        if before.min(after) <= local && local < before.max(after) {
            return local - transition.from;
        }
    }

    let mut offsets: Vec<i64> = Vec::new();
    for transition in &transitions {
        for offset in [transition.from, transition.to] {
            if !offsets.contains(&offset) {
                offsets.push(offset);
            }
        }
    }
    offsets
        .into_iter()
        .find(|offset| in_effect(&transitions, local - offset) == *offset)
        .map_or_else(
            || local - in_effect(&transitions, local),
            |offset| local - offset,
        )
}

/// One onset a `VTIMEZONE` observance states: the instant it begins and
/// the offsets on either side of it, in seconds.
struct Transition {
    at: i64,
    from: i64,
    to: i64,
}

/// The most periods an observance's `RRULE` is followed for from its
/// `DTSTART` (Annex A.3), the last onset found standing beyond them.
const ZONE_PERIODS_MAX: i64 = 1_000_000;

/// How far from a wall time, in seconds of local time, an onset can skip
/// or repeat it: an offset is under a day, so a transition spans under two,
/// and three leave a margin.
const NEAR: i64 = 3 * 86_400;

/// The transitions of a zone that bear on the wall time `local`, sorted:
/// each observance's first onset, its last one more than [`NEAR`] before
/// `local`, and every one within [`NEAR`] of it.
///
/// An observance's `RRULE` is read when it steps by a fixed span
/// (`SECONDLY` to `WEEKLY`, with `INTERVAL`, `COUNT` and `UNTIL` alone),
/// or when it is `YEARLY` in one `BYMONTH` by `BYDAY`, an ordinal before
/// it or a `BYMONTHDAY` beside it, with `INTERVAL`, `COUNT` and `UNTIL`:
/// the shapes zones are written in. One it cannot read contributes its
/// `DTSTART` alone.
fn transitions(zone: &Component, local: i64) -> Vec<Transition> {
    let mut transitions = Vec::new();

    for observance in &zone.children {
        if !matches!(observance.name.as_str(), "STANDARD" | "DAYLIGHT") {
            continue;
        }

        let (Some(from), Some(to)) = (
            observance.value("TZOFFSETFROM").as_deref().and_then(offset),
            observance.value("TZOFFSETTO").as_deref().and_then(offset),
        ) else {
            continue;
        };
        let Some(start) = observance.value("DTSTART").as_deref().and_then(wall) else {
            continue;
        };

        let rule = observance
            .value("RRULE")
            .as_deref()
            .and_then(ZoneRule::parse);
        let onsets = match rule {
            Some(rule) => rule.onsets(start, from, local),
            None => Vec::from([start]),
        };
        transitions.extend(onsets.into_iter().map(|onset| Transition {
            at: onset - from,
            from,
            to,
        }));
    }

    transitions.sort_by_key(|transition| transition.at);
    transitions
}

/// The offset in effect at an instant: the one the latest onset at or
/// before it puts in effect, else the first onset's origin.
fn in_effect(transitions: &[Transition], instant: i64) -> i64 {
    transitions
        .iter()
        .rfind(|transition| transition.at <= instant)
        .map(|transition| transition.to)
        .unwrap_or_else(|| transitions.first().map(|first| first.from).unwrap_or(0))
}

/// A `TZOFFSETFROM` or `TZOFFSETTO` value (`+hhmm[ss]`) in seconds.
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

/// A `YYYYMMDDTHHMMSS` date-time as seconds of its wall clock from the
/// epoch, a trailing `Z` ignored.
fn wall(value: &str) -> Option<i64> {
    let bytes = value.trim().as_bytes();
    if bytes.get(8) != Some(&b'T') {
        return None;
    }

    Some(time::unix(
        time::digits(bytes, 0, 4)? as i32,
        time::digits(bytes, 4, 2)?,
        time::digits(bytes, 6, 2)?,
        time::digits(bytes, 9, 2)?,
        time::digits(bytes, 11, 2)?,
        time::digits(bytes, 13, 2)?,
    ))
}

/// An observance's `RRULE` in a shape the zone resolution reads.
struct ZoneRule {
    /// The onsets a rule states, by its shape.
    shape: ZoneShape,
    /// `INTERVAL`, 1 when absent.
    interval: i64,
    /// `COUNT`, the onsets including the `DTSTART`.
    count: Option<i64>,
    /// `UNTIL` as wall-clock seconds, and whether it is UTC.
    until: Option<(i64, bool)>,
}

/// What a rule's periods each yield.
enum ZoneShape {
    /// One onset a period, a fixed number of seconds apart.
    Step(i64),
    /// The days of one month a year that are one weekday, the `n`th of
    /// them only (from the end when negative) or those in a day list.
    Yearly {
        month: u32,
        weekday: u32,
        ordinal: Option<i32>,
        days: Vec<u32>,
    },
}

impl ZoneRule {
    /// Reads a rule, `None` for a shape [`transitions`] does not follow.
    fn parse(rrule: &str) -> Option<Self> {
        let mut freq = None;
        let mut interval = 1;
        let mut count = None;
        let mut until = None;
        let mut month = None;
        let mut byday = None;
        let mut days = Vec::new();

        for part in rrule.split(';') {
            let (name, value) = part.split_once('=')?;
            match name.to_ascii_uppercase().as_str() {
                "FREQ" => freq = Some(value.to_ascii_uppercase()),
                "INTERVAL" => interval = value.parse().ok().filter(|interval| *interval > 0)?,
                "COUNT" => count = Some(value.parse().ok()?),
                "UNTIL" => until = Some((wall(value)?, value.trim().ends_with('Z'))),
                "BYMONTH" => {
                    month = Some(
                        value
                            .parse()
                            .ok()
                            .filter(|month| (1..=12).contains(month))?,
                    )
                }
                "BYDAY" => byday = Some(value.to_ascii_uppercase()),
                "BYMONTHDAY" => {
                    for day in value.split(',') {
                        days.push(day.parse().ok().filter(|day| (1..=31).contains(day))?);
                    }
                }
                "WKST" => {}
                _ => return None,
            }
        }

        let step = match freq?.as_str() {
            "SECONDLY" => Some(1),
            "MINUTELY" => Some(60),
            "HOURLY" => Some(3_600),
            "DAILY" => Some(86_400),
            "WEEKLY" => Some(7 * 86_400),
            "YEARLY" => None,
            _ => return None,
        };
        let shape = match (step, month, byday) {
            (Some(step), None, None) if days.is_empty() => ZoneShape::Step(step),
            (None, Some(month), Some(byday)) => {
                let (ordinal, day) = byday.split_at(byday.len().checked_sub(2)?);
                let weekday = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"]
                    .iter()
                    .position(|name| *name == day)? as u32;
                let ordinal = match ordinal {
                    "" => None,
                    ordinal => Some(ordinal.parse().ok().filter(|ordinal| *ordinal != 0)?),
                };
                ZoneShape::Yearly {
                    month,
                    weekday,
                    ordinal,
                    days,
                }
            }
            _ => return None,
        };

        Some(Self {
            shape,
            interval,
            count,
            until,
        })
    }

    /// The local onsets from `start` that bear on the wall time `local`:
    /// the first, the last more than [`NEAR`] before it and those within
    /// [`NEAR`] of it, at most [`ZONE_PERIODS_MAX`] periods followed.
    fn onsets(&self, start: i64, from: i64, local: i64) -> Vec<i64> {
        let (low, high) = (local - NEAR, local + NEAR);
        let mut onsets = Vec::from([start]);
        let mut before = None;

        // NOTE: the onsets in order, the `DTSTART` the first of `COUNT`.
        let mut push = |onset: i64, index: i64, onsets: &mut Vec<i64>| -> bool {
            let past_until = self.until.is_some_and(|(until, utc)| match utc {
                true => onset - from > until,
                false => onset > until,
            });
            if past_until || self.count.is_some_and(|count| index >= count) || onset > high {
                return false;
            }
            match onset < low {
                true => before = Some(onset),
                false => onsets.push(onset),
            }
            true
        };

        match &self.shape {
            ZoneShape::Step(step) => {
                let step = step * self.interval;
                let last = ZONE_PERIODS_MAX - 1;
                let first = (low - start).div_euclid(step).clamp(0, last);
                let from = match first {
                    0 => 1,
                    first => first,
                };
                for index in from..=last {
                    if !push(start + index * step, index, &mut onsets) {
                        break;
                    }
                }
            }
            ZoneShape::Yearly {
                month,
                weekday,
                ordinal,
                days,
            } => {
                let (first, ..) = time::civil_from_days(start.div_euclid(86_400));
                let clock = start.rem_euclid(86_400);
                let mut index = 0;
                'periods: for period in 0..ZONE_PERIODS_MAX {
                    let Some(year) = i32::try_from(period * self.interval)
                        .ok()
                        .and_then(|years| first.checked_add(years))
                        .filter(|year| *year <= 9999)
                    else {
                        break;
                    };
                    let length = time::days_in_month(year, *month) as i32;
                    let mut matching: Vec<u32> = (1..=length as u32)
                        .filter(|day| {
                            time::weekday(time::days_from_civil(year, *month, *day)) == *weekday
                        })
                        .filter(|day| days.is_empty() || days.contains(day))
                        .collect();
                    if let Some(ordinal) = ordinal {
                        let at = match *ordinal > 0 {
                            true => usize::try_from(ordinal - 1).ok(),
                            false => matching.len().checked_sub(ordinal.unsigned_abs() as usize),
                        };
                        matching = at
                            .and_then(|at| matching.get(at).copied())
                            .into_iter()
                            .collect();
                    }
                    for day in matching {
                        let onset = time::days_from_civil(year, *month, day) * 86_400 + clock;
                        if onset <= start {
                            continue;
                        }
                        index += 1;
                        if !push(onset, index, &mut onsets) {
                            break 'periods;
                        }
                    }
                }
            }
        }

        onsets.extend(before);
        onsets
    }
}

/// Whether a resource asks its source to notify its attendees (STORAGE
/// Annex B.1): a component naming an `ATTENDEE` with some `ORGANIZER` or
/// `ATTENDEE` not marked `SCHEDULE-AGENT=CLIENT` or `NONE`.
pub fn scheduled(body: &[u8]) -> bool {
    components(body).iter().any(|component| {
        let people: Vec<&PimdirContentLine> = component
            .lines
            .iter()
            .filter(|line| line.is("ATTENDEE") || line.is("ORGANIZER"))
            .collect();

        people.iter().any(|line| line.is("ATTENDEE")) && people.iter().any(|line| notified(line))
    })
}

/// The resource with `SCHEDULE-AGENT=NONE` written on every `ORGANIZER`
/// and `ATTENDEE` lacking `SCHEDULE-AGENT=CLIENT` or `NONE`, in every
/// component naming an `ATTENDEE` (Annex B.1): how a producer records
/// the user's acceptance that nobody is notified, which [`scheduled`]
/// then reads false. A rewritten line is folded at 75 octets; every other
/// line is kept byte for byte.
pub fn unschedule(body: &[u8]) -> Vec<u8> {
    let lines = logical_lines(body);
    let parsed: Vec<Option<PimdirContentLine>> = lines
        .iter()
        .map(|(_, line)| PimdirContentLine::parse(line))
        .collect();

    // NOTE: a property belongs to the component it is read in as
    // `components` reads them: a child of the root, time zones aside.
    let mut depth = 0usize;
    let mut current = None;
    let mut attended: Vec<bool> = Vec::new();
    let mut owners: Vec<Option<usize>> = Vec::with_capacity(parsed.len());
    for line in &parsed {
        let Some(line) = line else {
            owners.push(None);
            continue;
        };
        match line.name.as_str() {
            "BEGIN" => {
                depth += 1;
                if depth == 2 {
                    attended.push(false);
                    let zone = line.value.trim().eq_ignore_ascii_case("VTIMEZONE");
                    current = (!zone).then(|| attended.len() - 1);
                }
                owners.push(None);
            }
            "END" => {
                if depth == 2 {
                    current = None;
                }
                depth = depth.saturating_sub(1);
                owners.push(None);
            }
            name if depth == 2 => {
                if let (Some(component), "ATTENDEE") = (current, name) {
                    attended[component] = true;
                }
                owners.push(current);
            }
            _ => owners.push(None),
        }
    }

    let mut unscheduled = Vec::with_capacity(body.len());
    for (((raw, text), line), owner) in lines.iter().zip(&parsed).zip(&owners) {
        let marked = owner.is_some_and(|component| attended[component])
            && line.as_ref().is_some_and(|line| {
                (line.is("ORGANIZER") || line.is("ATTENDEE")) && notified(line)
            });
        let Some((head, value)) = split_value(text).filter(|_| marked) else {
            unscheduled.extend_from_slice(raw);
            continue;
        };

        let mut params = split_unquoted(head, ';');
        params.retain(|param| {
            !param
                .split_once('=')
                .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case("SCHEDULE-AGENT"))
        });
        let text = format!("{};SCHEDULE-AGENT=NONE:{value}", params.join(";"));

        // NOTE: folded at 75 octets, on a character boundary, with the
        // line ending the line had (RFC 5545 §3.1).
        let ending: &[u8] = match raw.ends_with(b"\r\n") {
            true => b"\r\n",
            false if raw.ends_with(b"\n") => b"\n",
            false => b"",
        };
        let mut width = 0;
        for char in text.chars() {
            if width + char.len_utf8() > 75 {
                unscheduled.extend_from_slice(if ending.is_empty() { b"\r\n" } else { ending });
                unscheduled.push(b' ');
                width = 1;
            }
            let mut bytes = [0; 4];
            unscheduled.extend_from_slice(char.encode_utf8(&mut bytes).as_bytes());
            width += char.len_utf8();
        }
        unscheduled.extend_from_slice(ending);
    }

    unscheduled
}

/// Whether an `ORGANIZER` or `ATTENDEE` is one a server notifies, lacking
/// `SCHEDULE-AGENT=CLIENT` or `NONE` (RFC 6638 §7.1).
fn notified(line: &PimdirContentLine) -> bool {
    !line.param("SCHEDULE-AGENT").is_some_and(|agent| {
        agent.eq_ignore_ascii_case("CLIENT") || agent.eq_ignore_ascii_case("NONE")
    })
}

/// The components carrying `RECURRENCE-ID`, `DTSTAMP` and `LAST-MODIFIED`
/// aside, in an order two equal sets share, so an update changing an
/// occurrence compares unequal (Annex B.1).
pub fn occurrences(body: &[u8]) -> Vec<Vec<String>> {
    let mut occurrences: Vec<Vec<String>> = components(body)
        .iter()
        .filter(|component| component.line("RECURRENCE-ID").is_some())
        .map(|component| {
            let mut lines: Vec<String> = component
                .lines
                .iter()
                .filter(|line| !line.is("DTSTAMP") && !line.is("LAST-MODIFIED"))
                .map(|line| format!("{}{:?}:{}", line.name, line.params, line.value))
                .collect();
            lines.sort();
            lines
        })
        .collect();
    occurrences.sort();
    occurrences
}

/// Whether a component asks for an online meeting of the source's
/// provider, `X-PIMDIR-ONLINE-MEETING:TRUE` (Annex B.1).
pub fn online_meeting(body: &[u8]) -> bool {
    components(body).iter().any(|component| {
        component
            .value("X-PIMDIR-ONLINE-MEETING")
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("TRUE"))
    })
}

/// Every component under the resource's `VCALENDAR`, time zones aside.
fn components(body: &[u8]) -> Vec<Component> {
    parse(&unfold(body, false))
        .into_iter()
        .flat_map(|root| root.children)
        .filter(|child| child.name != "VTIMEZONE")
        .collect()
}

/// The deepest component a derivation reads, the root at one: an
/// observance under a `VTIMEZONE` under the `VCALENDAR`.
const DEPTH: usize = 3;

/// One `BEGIN`/`END` block and what it holds.
struct Component {
    name: String,
    lines: Vec<PimdirContentLine>,
    children: Vec<Component>,
}

impl Component {
    /// The value of the first occurrence of a property.
    fn value(&self, name: &str) -> Option<String> {
        self.line(name).map(|line| line.value.clone())
    }

    /// The first occurrence of a property, parameters included.
    fn line(&self, name: &str) -> Option<&PimdirContentLine> {
        self.lines.iter().find(|line| line.is(name))
    }
}

/// Reads unfolded lines into the components they open and close; a
/// block whose `END` never arrives still closes at the end of the body.
///
/// A block nested past [`DEPTH`] is dropped as it closes, so the tree
/// holds no deeper level, nor drops one recursively, however deep the
/// body nests.
fn parse(lines: &[String]) -> Vec<Component> {
    let mut roots = Vec::new();
    let mut stack: Vec<Component> = Vec::new();

    for line in lines
        .iter()
        .filter_map(|line| PimdirContentLine::parse(line))
    {
        match line.name.as_str() {
            "BEGIN" => stack.push(Component {
                name: line.value.trim().to_uppercase(),
                lines: Vec::new(),
                children: Vec::new(),
            }),
            "END" => {
                let Some(closed) = stack.pop() else { continue };
                if stack.len() >= DEPTH {
                    continue;
                }
                match stack.last_mut() {
                    Some(parent) => parent.children.push(closed),
                    None => roots.push(closed),
                }
            }
            _ => {
                if let Some(component) = stack.last_mut() {
                    component.lines.push(line);
                }
            }
        }
    }

    while let Some(unclosed) = stack.pop() {
        if stack.len() >= DEPTH {
            continue;
        }
        match stack.last_mut() {
            Some(parent) => parent.children.push(unclosed),
            None => roots.push(unclosed),
        }
    }

    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVENT: &[u8] = b"BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:Europe/Paris\r\nBEGIN:STANDARD\r\nDTSTART:19701025T030000\r\nRRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nEND:STANDARD\r\nBEGIN:DAYLIGHT\r\nDTSTART:19700329T020000\r\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0200\r\nEND:DAYLIGHT\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:e1\r\nSUMMARY:Stand\\, up\r\nDTSTART;TZID=Europe/Paris:20260801T120000\r\nDTEND;TZID=Europe/Paris:20260801T123000\r\nORGANIZER;CN=\"Alice, A.\":mailto:Alice@Example.org\r\nATTENDEE:mailto:bob@example.org\r\nRRULE:FREQ=WEEKLY;UNTIL=20261231T000000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn a_task_reads_its_organisation_on_annex_a4s_edges() {
        let body = b"BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t\r\nPRIORITY:12\r\n\
CATEGORIES:,a\\,b,\r\nRELATED-TO;RELTYPE=CHILD:c\r\nRELATED-TO;RELTYPE=parent:p\r\n\
URL:MID:x%2Fy%40h/part%zz\r\nATTACH:mid:z\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let Some(PimdirSummary::Task(task)) = derive(body).summary else {
            panic!("a task summary");
        };
        assert_eq!(task.priority, None, "out of range");
        assert_eq!(task.categories, ["a,b"], "empty values dropped");
        assert_eq!(task.parent.as_deref(), Some("p"), "a CHILD skipped");
        assert_eq!(task.mid.as_deref(), Some("x/y@h"), "the id before the part");
        assert_eq!(percent_decoded("a%zz%4"), "a%zz%4", "no escape kept");
    }

    #[test]
    fn a_task_reads_its_contacts_on_annex_a4s_edges() {
        let body = b"BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t\r\nCREATED:bad\r\n\
DTSTAMP:20261029T080000Z\r\nCONTACT:\r\nCONTACT;ALTREP=\"mailto:Ann@X.org\":\r\n\
CONTACT;ALTREP=\"https://x.org/ann\":Ann <ANN@X.org>\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let Some(PimdirSummary::Task(task)) = derive(body).summary else {
            panic!("a task summary");
        };
        assert_eq!(
            task.created.as_deref(),
            Some("2026-10-29T08:00:00Z"),
            "a CREATED reading as no instant falls back to DTSTAMP"
        );
        let contacts: Vec<(&str, Option<&str>)> = task
            .contacts
            .iter()
            .map(|contact| (contact.address.as_str(), contact.name.as_deref()))
            .collect();
        assert_eq!(
            contacts,
            [("ann@x.org", None), ("ann@x.org", Some("Ann <ANN@X.org>"))],
            "an empty CONTACT names nobody, a mailto: ALTREP needs no text, \
             another ALTREP leaves the text canonical as the address"
        );
    }

    #[test]
    fn an_event_resolves_its_zoned_start_and_reads_its_people() {
        let derivation = derive(EVENT);
        let Some(PimdirSummary::Event(event)) = derivation.summary else {
            panic!("an event summary");
        };
        assert_eq!(derivation.link_id.as_str(), "e1");
        assert_eq!(derivation.sort_key.as_str(), "2026-08-01T10:00:00Z");
        assert_eq!(event.summary, "Stand, up");
        assert_eq!(
            event.dtstart.as_ref().unwrap().tzid.as_deref(),
            Some("Europe/Paris")
        );
        assert_eq!(event.until.as_deref(), Some("20261231T000000Z"));
        assert_eq!(event.recurring, Some(true));
        let organizer = event.organizer.unwrap();
        assert_eq!(organizer.address, "alice@example.org");
        assert_eq!(organizer.name.as_deref(), Some("Alice, A."));
        assert_eq!(event.attendees[0].address, "bob@example.org");
    }

    #[test]
    fn a_task_keys_on_its_due_date_and_a_journal_on_its_start() {
        let task = derive(b"BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t1\r\nDUE;VALUE=DATE:20260810\r\nSTATUS:needs-action\r\nPERCENT-COMPLETE:40\r\nEND:VTODO\r\nEND:VCALENDAR\r\n");
        assert_eq!(task.sort_key.as_str(), "2026-08-10T00:00:00Z");
        let Some(PimdirSummary::Task(task)) = task.summary else {
            panic!("a task summary");
        };
        assert_eq!(task.status.as_deref(), Some("NEEDS-ACTION"));
        assert_eq!(task.percent, Some(40));
        assert!(task.due.unwrap().date);

        let journal = derive(b"BEGIN:VCALENDAR\r\nBEGIN:VJOURNAL\r\nUID:j1\r\nDTSTART:20260801T100000Z\r\nEND:VJOURNAL\r\nEND:VCALENDAR\r\n");
        assert!(matches!(journal.summary, Some(PimdirSummary::Journal(_))));
        assert_eq!(journal.sort_key.as_str(), "2026-08-01T10:00:00Z");
    }

    #[test]
    fn a_resource_with_no_table_is_an_item_with_no_summary() {
        let busy = derive(
            b"BEGIN:VCALENDAR\r\nBEGIN:VFREEBUSY\r\nUID:f1\r\nEND:VFREEBUSY\r\nEND:VCALENDAR\r\n",
        );
        assert_eq!(busy.link_id.as_str(), "f1");
        assert_eq!(busy.summary, None);
        assert_eq!(busy.sort_key.as_str(), "");
    }

    #[test]
    fn the_ambiguous_and_skipped_hours_take_the_offset_before() {
        let with_start = |start: &str| {
            let body = alloc::format!(
                "BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:Europe/Paris\r\nBEGIN:STANDARD\r\nDTSTART:19701025T030000\r\nRRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nEND:STANDARD\r\nBEGIN:DAYLIGHT\r\nDTSTART:19700329T020000\r\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0200\r\nEND:DAYLIGHT\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTART;TZID=Europe/Paris:{start}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
            );
            derive(body.as_bytes()).sort_key.0
        };
        assert_eq!(
            with_start("20261025T023000"),
            "2026-10-25T00:30:00Z",
            "the repeated hour reads as summer time"
        );
        assert_eq!(
            with_start("20260329T023000"),
            "2026-03-29T01:30:00Z",
            "the skipped hour reads as winter time"
        );
    }

    /// The pre-2007 United States rule stops at its `UNTIL`, so the last
    /// week of October 2026 reads under the later rule alone, in summer time.
    #[test]
    fn an_observance_rule_stops_at_its_until() {
        let body = b"BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:Eastern\r\n\
BEGIN:STANDARD\r\nDTSTART:19671029T020000\r\nTZOFFSETFROM:-0400\r\nTZOFFSETTO:-0500\r\n\
RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU;UNTIL=20061029T060000Z\r\nEND:STANDARD\r\n\
BEGIN:DAYLIGHT\r\nDTSTART:20070311T020000\r\nTZOFFSETFROM:-0500\r\nTZOFFSETTO:-0400\r\n\
RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU\r\nEND:DAYLIGHT\r\n\
BEGIN:STANDARD\r\nDTSTART:20071104T020000\r\nTZOFFSETFROM:-0400\r\nTZOFFSETTO:-0500\r\n\
RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n\
BEGIN:VEVENT\r\nUID:x\r\nDTSTART;TZID=Eastern:20261028T120000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        assert_eq!(derive(body).sort_key.as_str(), "2026-10-28T16:00:00Z");
    }

    /// A rule stepping by a fixed span is followed, within its `COUNT`.
    #[test]
    fn an_observance_rule_stepping_weekly_is_followed() {
        let with_count = |count: &str| {
            let body = alloc::format!(
                "BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:Weekly\r\n\
BEGIN:STANDARD\r\nDTSTART:20260104T000000\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0000\r\n\
RRULE:FREQ=WEEKLY\r\nEND:STANDARD\r\n\
BEGIN:DAYLIGHT\r\nDTSTART:20260101T000000\r\nTZOFFSETFROM:+0000\r\nTZOFFSETTO:+0100\r\n\
RRULE:FREQ=WEEKLY{count}\r\nEND:DAYLIGHT\r\nEND:VTIMEZONE\r\n\
BEGIN:VEVENT\r\nUID:x\r\nDTSTART;TZID=Weekly:20260109T120000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
            );
            derive(body.as_bytes()).sort_key.0
        };
        assert_eq!(with_count(""), "2026-01-09T11:00:00Z", "Thursday's onset");
        assert_eq!(
            with_count(";COUNT=1"),
            "2026-01-09T12:00:00Z",
            "its first alone"
        );
    }

    /// A `TZID` no `VTIMEZONE` defines resolves through the IANA database
    /// by its exact name, the offset before a gap included; another
    /// spelling reads as floating.
    #[test]
    fn a_tzid_without_its_vtimezone_resolves_through_the_database() {
        let with_start = |tzid: &str, start: &str| {
            let body = alloc::format!(
                "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTART;TZID={tzid}:{start}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
            );
            derive(body.as_bytes()).sort_key.0
        };
        assert_eq!(
            with_start("America/New_York", "20261101T013000"),
            "2026-11-01T05:30:00Z",
            "the repeated hour reads as summer time"
        );
        assert_eq!(
            with_start("Europe/Paris", "20260329T023000"),
            "2026-03-29T01:30:00Z",
            "the skipped hour reads as winter time"
        );
        assert_eq!(
            with_start("europe/paris", "20260814T090000"),
            "2026-08-14T09:00:00Z",
            "matched byte for byte"
        );
    }

    /// Accepting that nobody is notified marks every person of a component
    /// naming an attendee, a server agent replaced, a long line folded, and
    /// leaves the rest as it was: a marked person, an alarm's attendee, a
    /// component naming none, a time zone.
    #[test]
    fn an_unscheduled_resource_is_scheduled_no_more() {
        let long = "a".repeat(70);
        let body = format!(
            "BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:Z\r\nEND:VTIMEZONE\r\n\
BEGIN:VEVENT\r\nUID:e\r\nORGANIZER;CN=\"Ann: A\":mailto:ann@x.org\r\n\
ATTENDEE;SCHEDULE-AGENT=SERVER;RSVP=TRUE:mailto:\r\n bob@x.org\r\n\
ATTENDEE;SCHEDULE-AGENT=CLIENT:mailto:cid@x.org\r\n\
ATTENDEE;CN={long}:mailto:dan@x.org\r\n\
BEGIN:VALARM\r\nACTION:EMAIL\r\nATTENDEE:mailto:ann@x.org\r\nEND:VALARM\r\n\
END:VEVENT\r\nBEGIN:VEVENT\r\nUID:e\r\nRECURRENCE-ID:20261010T090000Z\r\n\
ORGANIZER:mailto:ann@x.org\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        );
        assert!(scheduled(body.as_bytes()));

        let unscheduled = unschedule(body.as_bytes());
        assert!(!scheduled(&unscheduled));
        let expected = format!(
            "BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:Z\r\nEND:VTIMEZONE\r\n\
BEGIN:VEVENT\r\nUID:e\r\nORGANIZER;CN=\"Ann: A\";SCHEDULE-AGENT=NONE:mailto:ann@x.org\r\n\
ATTENDEE;RSVP=TRUE;SCHEDULE-AGENT=NONE:mailto:bob@x.org\r\n\
ATTENDEE;SCHEDULE-AGENT=CLIENT:mailto:cid@x.org\r\n\
ATTENDEE;CN={}\r\n {};SCHEDULE-AGENT=NONE:mailto:dan@x.org\r\n\
BEGIN:VALARM\r\nACTION:EMAIL\r\nATTENDEE:mailto:ann@x.org\r\nEND:VALARM\r\n\
END:VEVENT\r\nBEGIN:VEVENT\r\nUID:e\r\nRECURRENCE-ID:20261010T090000Z\r\n\
ORGANIZER:mailto:ann@x.org\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
            &long[..63],
            &long[63..],
        );
        assert_eq!(String::from_utf8(unscheduled.clone()).unwrap(), expected);
        assert_eq!(unschedule(&unscheduled), unscheduled, "marked once");
    }
}
