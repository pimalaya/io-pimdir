//! # Occurrences
//!
//! Calendar time derived for the index (SEARCH §7): every occurrence of a
//! calendar resource within the horizon, one year back and two forward,
//! as instants, so a date term reaches the twelfth Friday of a series
//! whose sort key holds its first.
//!
//! Expansion is ical-rs's: the resource's own `VTIMEZONE`s, else the
//! bundled database by `TZID`, else floating. One item yields at most
//! [`OCCURRENCES_MAX`]. What re-expands an item, and the rows the
//! occurrences are written to, are the refresh's
//! ([`refresh`](crate::search::refresh)).

use alloc::vec::Vec;

use ical::{recur::instant::IcalRecurTimeline, tree::cst::IcalCst, tzdb, value::IcalValue};

use crate::summary::time;

/// The most occurrences one calendar item writes (SEARCH §7): a series
/// past it is cut, and the refresh counts it.
pub const OCCURRENCES_MAX: usize = 1000;

/// The seconds a horizon has to move by before the index rolls it: a
/// day, so a refresh re-expands at most once a day.
pub(crate) const ROLL: i64 = 86_400;

/// The window occurrences are materialised within, `[start, end)` in Unix
/// seconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Horizon {
    pub(crate) start: i64,
    pub(crate) end: i64,
}

impl Horizon {
    /// The default horizon around `now` (SEARCH §7): the same instant one
    /// calendar year before and two after, a 29 February landing on the
    /// 28th.
    pub(crate) fn around(now: i64) -> Self {
        let shift = |years: i32| {
            let days = now.div_euclid(86_400);
            let seconds = now.rem_euclid(86_400);
            let (year, month, day) = time::civil_from_days(days);
            let year = year + years;
            let day = day.min(time::days_in_month(year, month));
            time::days_from_civil(year, month, day) * 86_400 + seconds
        };
        Self {
            start: shift(-1),
            end: shift(2),
        }
    }

    /// The occurrences of a calendar resource overlapping the horizon, as
    /// `(start, end)` Unix seconds, at most [`OCCURRENCES_MAX`], and
    /// whether the resource held more: one timeline per `UID` of its
    /// events, tasks and journals, overrides included, every time resolved
    /// through its own `VTIMEZONE`s, else the bundled database, else
    /// floating, read on UTC. A resource no parser accepts, or a component
    /// naming no `UID`, yields nothing.
    pub(crate) fn occurrences(&self, body: &[u8]) -> (Vec<(i64, i64)>, bool) {
        let Ok(cst) = IcalCst::parse(body) else {
            return (Vec::new(), false);
        };
        let calendar = cst.decode();

        let mut uids: Vec<&str> = Vec::new();
        for component in &calendar.components {
            let item = matches!(
                component.name.to_ascii_uppercase().as_str(),
                "VEVENT" | "VTODO" | "VJOURNAL"
            );
            let uid = component.props.iter().find_map(|prop| match &prop.value {
                IcalValue::Text(uid) if prop.name.eq_ignore_ascii_case("UID") => {
                    Some(uid.0.as_ref())
                }
                _ => None,
            });
            if let Some(uid) = uid.filter(|uid| item && !uids.contains(uid)) {
                uids.push(uid);
            }
        }

        let through = time::civil_from_days(self.end.div_euclid(86_400)).0 + 1;
        let zones = tzdb::zones(&calendar.components, through);
        let mut occurrences = Vec::new();
        for uid in uids {
            let timeline = IcalRecurTimeline::of_uid(&calendar.components, uid, &zones);
            let room = OCCURRENCES_MAX + 1 - occurrences.len();
            occurrences.extend(
                timeline
                    .expand(self.start, self.end)
                    .take(room)
                    .map(|instant| (instant.start, instant.end)),
            );
            if occurrences.len() > OCCURRENCES_MAX {
                break;
            }
        }

        let cut = occurrences.len() > OCCURRENCES_MAX;
        occurrences.truncate(OCCURRENCES_MAX);
        (occurrences, cut)
    }
}

/// The version of the time zone database occurrences resolve through,
/// empty with none.
pub(crate) fn tzdb_version() -> &'static str {
    tzdb::version().unwrap_or_default()
}

/// An RFC 3339 instant in UTC, `YYYY-MM-DDTHH:MM:SSZ`, as Unix seconds.
pub(crate) fn instant(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() != 20 || bytes[10] != b'T' || bytes[19] != b'Z' {
        return None;
    }
    Some(time::unix(
        time::digits(bytes, 0, 4)? as i32,
        time::digits(bytes, 5, 2)?,
        time::digits(bytes, 8, 2)?,
        time::digits(bytes, 11, 2)?,
        time::digits(bytes, 14, 2)?,
        time::digits(bytes, 17, 2)?,
    ))
}

#[cfg(test)]
mod tests {
    use crate::{search::occurrence::*, summary::time};

    #[test]
    fn the_horizon_spans_a_year_back_and_two_forward() {
        let now = time::unix(2026, 8, 20, 12, 0, 0);
        let horizon = Horizon::around(now);
        assert_eq!(time::rfc3339(horizon.start), "2025-08-20T12:00:00Z");
        assert_eq!(time::rfc3339(horizon.end), "2028-08-20T12:00:00Z");

        let leap = Horizon::around(time::unix(2028, 2, 29, 0, 0, 0));
        assert_eq!(time::rfc3339(leap.start), "2027-02-28T00:00:00Z");
        assert_eq!(time::rfc3339(leap.end), "2030-02-28T00:00:00Z");
    }

    #[test]
    fn an_instant_reads_back_what_it_renders() {
        let at = time::unix(2026, 8, 20, 12, 34, 56);
        assert_eq!(instant(&time::rfc3339(at)), Some(at));
        assert_eq!(instant("2026-08-20"), None);
        assert_eq!(instant("2026-08-20T12:00:00+02:00"), None);
    }

    #[test]
    fn one_item_writes_at_most_the_bound() {
        let body = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:daily\r\n\
                    DTSTART:20260101T090000Z\r\nRRULE:FREQ=HOURLY\r\nEND:VEVENT\r\n\
                    END:VCALENDAR\r\n";
        let horizon = Horizon::around(time::unix(2026, 8, 20, 0, 0, 0));
        let (occurrences, cut) = horizon.occurrences(body.as_bytes());
        assert_eq!(occurrences.len(), OCCURRENCES_MAX);
        assert!(cut);
    }
}
