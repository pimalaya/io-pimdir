//! # Occurrences
//!
//! Calendar time derived for the index (SEARCH §7): every occurrence of a
//! calendar resource within the horizon, one year back and two forward,
//! as instants, so a date term reaches the twelfth Friday of a series
//! whose sort key holds its first.
//!
//! Expansion is ical-rs's: the resource's own `VTIMEZONE`s, else the
//! bundled database by `TZID`, else floating, a floating time and a date
//! read in the zone the indexer states. One item writes at most
//! [`OCCURRENCES_MAX`] occurrences and a rule is examined for at most
//! [`PERIODS_MAX`] periods; an item either bound stops records its cut.
//! What re-expands an item, and the rows the occurrences are written to,
//! are the refresh's ([`refresh`](crate::search::refresh)).

use alloc::vec::Vec;

use ical::{
    recur::{
        IcalRecurDateTime, IcalRecurFreq, IcalRecurRule, expand::IcalRecurExpand,
        instant::IcalRecurTimeline, set::IcalRecurZone,
    },
    tree::cst::IcalCst,
    tz::IcalTz,
    tzdb,
    value::IcalValue,
};

use crate::summary::{calendar, time};

/// The most occurrences one calendar item writes (SEARCH §7): the first,
/// by start then end, of those overlapping the horizon.
pub const OCCURRENCES_MAX: usize = 10_000;

/// The most periods of a recurrence rule an expansion examines (SEARCH
/// §7), a period being `INTERVAL` times the rule's `FREQ` from the one
/// holding its `DTSTART`, whether or not it yields.
pub const PERIODS_MAX: i64 = 1_000_000;

/// The zone floating times and dates are read in when the indexer states
/// none (SEARCH §7).
pub const FLOATING_ZONE: &str = "UTC";

/// The seconds a horizon has to move by before the index rolls it: a
/// day, so a refresh re-expands at most once a day.
pub(crate) const ROLL: i64 = 86_400;

/// Seconds in a day: how far a floating occurrence may move once read in
/// a zone, so its walk is widened by it on both sides.
const DAY: i64 = 86_400;

/// A calendar resource's occurrences within the horizon, `(start, end)`
/// Unix seconds by start then end, and its cut: the instant from which
/// they are not all written, `None` when no bound stopped it.
pub(crate) type Expansion = (Vec<(i64, i64)>, Option<i64>);

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

    /// The occurrences of a calendar resource overlapping the horizon and
    /// its cut (SEARCH §7): one timeline per `UID` of its events, tasks and
    /// journals, overrides included, every time resolved through its own
    /// `VTIMEZONE`s, else the bundled database, else read in `zone`, an
    /// IANA name. A rule's occurrences from its first unexamined period on
    /// are not written, and of the rest the first [`OCCURRENCES_MAX`] by
    /// start then end are. A resource no parser accepts, or a component
    /// naming no `UID`, yields nothing.
    pub(crate) fn occurrences(&self, body: &[u8], zone: &str) -> Expansion {
        let Ok(cst) = IcalCst::parse(body) else {
            return (Vec::new(), None);
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
        let floating = |seconds: i64| match zone {
            FLOATING_ZONE => seconds,
            zone => calendar::iana(zone, seconds).unwrap_or(seconds),
        };

        let mut occurrences = Vec::new();
        let mut cut: Option<i64> = None;
        for uid in uids {
            let timeline = IcalRecurTimeline::of_uid(&calendar.components, uid, &zones);
            let examined = self.examined(&timeline, &zones, &floating);
            if let Some(examined) = examined {
                cut = Some(cut.map_or(examined, |cut| cut.min(examined)));
            }

            let walked = timeline
                .expand(self.start - DAY, self.end + DAY)
                .map(|instant| match instant.floating {
                    true => (floating(instant.start), floating(instant.end)),
                    false => (instant.start, instant.end),
                })
                .filter(|(start, end)| self.overlaps(*start, *end))
                .filter(|(start, _)| examined.is_none_or(|examined| *start < examined))
                .take(OCCURRENCES_MAX + 1);
            occurrences.extend(walked);
        }

        occurrences.sort_unstable();
        if let Some((start, _)) = occurrences.get(OCCURRENCES_MAX) {
            cut = Some(cut.map_or(*start, |cut| cut.min(*start)));
        }
        occurrences.truncate(OCCURRENCES_MAX);
        (occurrences, cut)
    }

    /// Whether an occurrence overlaps the horizon: it starts before the
    /// horizon ends and ends after it starts, or, lasting no time, starts
    /// within it.
    fn overlaps(&self, start: i64, end: i64) -> bool {
        start < self.end && (end > self.start || start >= self.start)
    }

    /// The instant a series' rules stop being examined (SEARCH §7): the
    /// earliest start of a rule's first unexamined period lying before the
    /// horizon's end, `None` when every rule ends before it, by its `UNTIL`
    /// or its `COUNT`, or reaches the horizon's end within the bound.
    fn examined(
        &self,
        timeline: &IcalRecurTimeline,
        zones: &[IcalTz],
        floating: &impl Fn(i64) -> i64,
    ) -> Option<i64> {
        let set = timeline.set();
        let start = set.start?;
        let instant = |local: IcalRecurDateTime| match &set.zone {
            IcalRecurZone::Utc => local.seconds(),
            IcalRecurZone::Tz(id) => match zones.iter().find(|zone| zone.id == *id) {
                Some(zone) => zone.resolve(local).literal_instant(local),
                None => floating(local.seconds()),
            },
            IcalRecurZone::Floating => floating(local.seconds()),
        };

        set.rules
            .iter()
            .filter_map(|rule| {
                let unexamined = unexamined(rule, start)?;
                if instant(unexamined) >= self.end {
                    return None;
                }
                if rule.until.is_some_and(|until| until < unexamined) {
                    return None;
                }
                if let Some(count) = rule.count {
                    let before = IcalRecurExpand::new(rule.clone(), start)
                        .take_while(|at| *at < unexamined)
                        .take(count as usize)
                        .count();
                    if before == count as usize {
                        return None;
                    }
                }
                Some(instant(unexamined))
            })
            .min()
    }
}

/// The start of a rule's first period past [`PERIODS_MAX`], on the
/// series' clock: the period holding `start` is the first, each next one
/// `INTERVAL` times `FREQ` on, as RFC 5545 §3.3.10 steps them. `None` past
/// the last year a date spells.
fn unexamined(rule: &IcalRecurRule, start: IcalRecurDateTime) -> Option<IcalRecurDateTime> {
    let steps = PERIODS_MAX * i64::from(rule.interval.max(1));
    let day = start.seconds().div_euclid(DAY);
    let date = |day: i64| IcalRecurDateTime::from_seconds(day * DAY);

    let at = match rule.freq {
        IcalRecurFreq::Secondly => IcalRecurDateTime::from_seconds(start.seconds() + steps),
        IcalRecurFreq::Minutely => IcalRecurDateTime::from_seconds(start.seconds() + steps * 60),
        IcalRecurFreq::Hourly => IcalRecurDateTime::from_seconds(start.seconds() + steps * 3600),
        IcalRecurFreq::Daily => date(day + steps),
        IcalRecurFreq::Weekly => {
            // NOTE: 1970-01-01 was a Thursday, weekday 4 counted from
            // Sunday as `WKST` is.
            let back = (day + 4 - rule.week_start as i64).rem_euclid(7);
            date(day - back + steps * 7)
        }
        IcalRecurFreq::Monthly => {
            let month = i64::from(start.year) * 12 + i64::from(start.month) - 1 + steps;
            let year = i32::try_from(month.div_euclid(12)).ok()?;
            IcalRecurDateTime::date(year, month.rem_euclid(12) as u8 + 1, 1)
        }
        IcalRecurFreq::Yearly => {
            let year = i32::try_from(i64::from(start.year) + steps).ok()?;
            IcalRecurDateTime::date(year, 1, 1)
        }
    };

    (at.year <= 9999).then_some(at)
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

    fn event(start: &str, rule: &str) -> alloc::string::String {
        alloc::format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:series\r\n\
             DTSTART{start}\r\nDURATION:PT5M\r\nRRULE:{rule}\r\nEND:VEVENT\r\n\
             END:VCALENDAR\r\n"
        )
    }

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

    /// The occurrence bound keeps the first by start and cuts at the start
    /// of the one past it.
    #[test]
    fn one_item_writes_at_most_the_bound_and_is_cut_at_the_next() {
        let body = event(":20260101T090000Z", "FREQ=HOURLY");
        let horizon = Horizon::around(time::unix(2026, 8, 20, 0, 0, 0));
        let (occurrences, cut) = horizon.occurrences(body.as_bytes(), FLOATING_ZONE);
        assert_eq!(occurrences.len(), OCCURRENCES_MAX);
        let next = occurrences[OCCURRENCES_MAX - 1].0 + 3600;
        assert_eq!(cut, Some(next));
    }

    /// The period bound stops a rule whose `BYxxx` parts seldom match at
    /// its millionth period, counted from `DTSTART` before the horizon;
    /// one whose `UNTIL` or `COUNT` ends first is not cut.
    #[test]
    fn a_rule_is_examined_for_at_most_the_period_bound() {
        let horizon = Horizon::around(time::unix(2026, 8, 20, 0, 0, 0));
        let sparse = "FREQ=SECONDLY;BYMONTH=8;BYMONTHDAY=1;BYHOUR=0;BYMINUTE=0;BYSECOND=0";
        let body = event(":20260801T000000Z", sparse);
        let (occurrences, cut) = horizon.occurrences(body.as_bytes(), FLOATING_ZONE);
        let start = time::unix(2026, 8, 1, 0, 0, 0);
        assert_eq!(occurrences, [(start, start + 300)]);
        assert_eq!(cut, Some(start + PERIODS_MAX));

        let until = alloc::format!("{sparse};UNTIL=20260810T000000Z");
        let body = event(":20260801T000000Z", &until);
        assert_eq!(horizon.occurrences(body.as_bytes(), FLOATING_ZONE).1, None);
        let body = event(":20260801T000000Z", &alloc::format!("{sparse};COUNT=1"));
        assert_eq!(horizon.occurrences(body.as_bytes(), FLOATING_ZONE).1, None);

        let body = event(":20260801T000000Z", "FREQ=DAILY");
        assert_eq!(
            horizon.occurrences(body.as_bytes(), FLOATING_ZONE).1,
            None,
            "a daily rule reaches the horizon's end long before its bound"
        );
    }

    /// A floating time is read in the zone the indexer states, a zoned or
    /// UTC one never.
    #[test]
    fn a_floating_time_is_read_in_the_stated_zone() {
        let horizon = Horizon::around(time::unix(2026, 8, 20, 0, 0, 0));
        let floating = event(":20260801T090000", "COUNT=1;FREQ=DAILY");
        let utc = event(":20260801T090000Z", "COUNT=1;FREQ=DAILY");
        let nine = time::unix(2026, 8, 1, 9, 0, 0);

        let (read, _) = horizon.occurrences(floating.as_bytes(), FLOATING_ZONE);
        assert_eq!(read[0].0, nine);
        let (read, _) = horizon.occurrences(floating.as_bytes(), "Europe/Paris");
        assert_eq!(read[0].0, nine - 2 * 3600);
        let (read, _) = horizon.occurrences(utc.as_bytes(), "Europe/Paris");
        assert_eq!(read[0].0, nine);
    }
}
