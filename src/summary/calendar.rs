//! # Calendar summaries
//!
//! The `text/calendar` derivations (Annex A.3 to A.5): the item is the
//! resource, summarised from the master component, one row in the table
//! of its type. Times are carried verbatim with the parameters that give
//! them meaning; the one resolved instant is the sort key, read through
//! the `VTIMEZONE` the resource itself carries and nothing else.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use crate::{
    placement::{PimdirLinkId, PimdirSortKey},
    summary::{
        PimdirAddress, PimdirDerivation, PimdirSummary,
        contact::{PimdirContentLine, unescape},
        hash_key, time, unfold,
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
    /// The `ORGANIZER`.
    pub organizer: Option<PimdirAddress>,
    /// Every `ATTENDEE`, in document order.
    pub attendees: Vec<PimdirAddress>,
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
    let link_id = match &uid {
        Some(uid) => PimdirLinkId(uid.clone()),
        None => hash_key(body),
    };

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
/// A UTC date-time is an instant already; a zoned one resolves through
/// the resource's own `VTIMEZONE`; a zone that will not resolve reads as
/// floating; a date-only value reads as midnight UTC; a floating one
/// reads its wall time as UTC.
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

    let zone = time.tzid.as_deref().and_then(|tzid| {
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
/// An ambiguous hour takes the offset in effect before the transition
/// and a nonexistent one the offset after it, both the numerically
/// greater offset and the earlier instant, so one rule serves both.
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
    at: i64,
    to: i64,
    from: i64,
}

/// The transitions a zone states over the years around `year`.
///
/// It reads `FREQ=YEARLY` with `BYMONTH` and `BYDAY`, what every zone a
/// real calendar writes carries; a subcomponent whose rule this cannot
/// read contributes its `DTSTART` alone.
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

/// Every offset the transitions put in effect, the first one's origin included.
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

/// The offset in effect at an instant.
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

/// The `(month, ordinal, weekday)` a yearly transition rule names.
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

/// The day of the month the `n`th weekday falls on, from the end when negative.
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
    fn the_ambiguous_and_missing_hours_take_the_greater_offset() {
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
            "2026-03-29T00:30:00Z",
            "the skipped hour reads as summer time"
        );
    }
}
