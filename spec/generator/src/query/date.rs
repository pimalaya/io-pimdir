//! # Dates
//!
//! SEARCH §8.1 read as written: a `date-value` of §8.3, the span each
//! point covers in the zone a client states, a range from the start of
//! one span to the end of another, a bound past a four-digit year open,
//! and the `stamp-value` of `changed:`. The zones the model resolves are
//! UTC and Europe/Paris under the European Union's rule since 1996; a
//! span another zone or another year would need is no answer.

use std::ops::Range;

use crate::time::Instant;

/// The first instant no four-digit year states, 10000-01-01T00:00:00Z.
const PAST_YEARS: i64 = 253_402_300_800;

/// The first instant of year 0, 0000-01-01T00:00:00Z.
const YEAR_ZERO: i64 = -62_167_219_200;

/// A zone a client or an indexer states (SEARCH §7, §8.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    /// `UTC`.
    Utc,
    /// `Europe/Paris`.
    Paris,
}

impl Zone {
    /// The zone an IANA name names, none for one the model does not hold.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "UTC" => Some(Self::Utc),
            "Europe/Paris" => Some(Self::Paris),
            _ => None,
        }
    }

    /// Its IANA name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Utc => "UTC",
            Self::Paris => "Europe/Paris",
        }
    }

    /// Its offset from UTC at `at`, seconds; none where the model holds
    /// no rule.
    pub fn offset(&self, at: i64) -> Option<i64> {
        match self {
            Self::Utc => Some(0),
            Self::Paris => {
                let year = civil(at).0;
                if !(1996..=2099).contains(&year) {
                    return None;
                }
                let start = last_sunday(year, 3) + 3600;
                let end = last_sunday(year, 10) + 3600;
                Some(match (start..end).contains(&at) {
                    true => 7200,
                    false => 3600,
                })
            }
        }
    }

    /// The instant a wall time names here: a time a transition skips read
    /// with the offset in force before it, one it repeats as its first
    /// occurrence (SEARCH §8.1).
    pub fn instant(&self, wall: i64) -> Option<i64> {
        let mut fits = Vec::new();
        for offset in [7200, 3600, 0] {
            let at = wall - offset;
            if self.offset(at)? == offset {
                fits.push(at);
            }
        }
        match fits.first() {
            Some(at) => Some(*at),
            None => Some(wall - self.offset(wall - 86_400)?),
        }
    }

    /// The wall time `at` reads as here.
    pub fn wall(&self, at: i64) -> Option<i64> {
        Some(at + self.offset(at)?)
    }
}

/// A span, half-open, either side open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    /// Its first instant, none when open.
    pub start: Option<i64>,
    /// The instant it ends before, none when open.
    pub end: Option<i64>,
}

impl Span {
    /// Whether an occurrence from `start` to `end` overlaps it, one of zero
    /// length falling in it when it starts inside (SEARCH §8.1).
    pub fn meets(&self, start: i64, end: i64) -> bool {
        let before_end = self.end.is_none_or(|bound| start < bound);
        match start == end {
            true => before_end && self.start.is_none_or(|bound| bound <= start),
            false => before_end && self.start.is_none_or(|bound| end > bound),
        }
    }
}

/// A `date-value` of SEARCH §8.3.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DateValue {
    /// Its first point, none for `..b`.
    pub from: Option<Point>,
    /// Its last point, none for `a..`.
    pub to: Option<Point>,
    /// Whether it is a range.
    pub range: bool,
}

/// A `point` of SEARCH §8.3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Point {
    /// A year.
    Year(i64),
    /// A month.
    Month { year: i64, month: u32 },
    /// A day.
    Day { year: i64, month: u32, day: u32 },
    /// An instant, its second and its offset when stated.
    Instant {
        wall: i64,
        second: bool,
        offset: Option<i64>,
    },
    /// `today`.
    Today,
    /// `yesterday`.
    Yesterday,
    /// A relative amount.
    Relative { amount: i64, unit: Unit },
}

/// The unit of a relative amount.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// `min`.
    Minutes,
    /// `h`.
    Hours,
    /// `d`, of 24 hours.
    Days,
    /// `w`, of seven days.
    Weeks,
    /// `mo`, calendar months on UTC's calendar.
    Months,
    /// `y`, calendar years on UTC's calendar.
    Years,
}

impl DateValue {
    /// Reads a `date-value`, none for a value §8.3 refuses.
    pub fn parse(value: &str) -> Option<Self> {
        match value.split_once("..") {
            None => Some(Self {
                from: Some(Point::parse(value)?),
                to: None,
                range: false,
            }),
            Some(("", "")) => None,
            Some((from, to)) => Some(Self {
                from: match from {
                    "" => None,
                    from => Some(Point::parse(from)?),
                },
                to: match to {
                    "" => None,
                    to => Some(Point::parse(to)?),
                },
                range: true,
            }),
        }
    }

    /// The span it covers for a client at `now` in `zone`, a bound past a
    /// four-digit year open; none where the model cannot place it.
    pub fn span(&self, now: i64, zone: Zone) -> Option<Span> {
        let (start, end) = match (self.range, self.from, self.to) {
            (false, Some(Point::Relative { amount, unit }), _) => {
                (Some(relative(now, amount, unit)?), Some(now))
            }
            (false, Some(point), _) => {
                let (start, end) = point.span(now, zone)?;
                (Some(start), Some(end))
            }
            (true, from, to) => {
                let start = match from {
                    None => None,
                    Some(Point::Relative { amount, unit }) => Some(relative(now, amount, unit)?),
                    Some(point) => Some(point.span(now, zone)?.0),
                };
                let end = match to {
                    None => None,
                    Some(Point::Relative { amount, unit }) => Some(relative(now, amount, unit)?),
                    Some(point) => Some(point.span(now, zone)?.1),
                };
                (start, end)
            }
            (false, None, _) => return None,
        };

        Some(Span {
            start: start.filter(|start| *start >= YEAR_ZERO),
            end: end.filter(|end| *end < PAST_YEARS),
        })
    }
}

impl Point {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "today" => return Some(Self::Today),
            "yesterday" => return Some(Self::Yesterday),
            _ => {}
        }

        let digits = text.bytes().take_while(u8::is_ascii_digit).count();
        let unit = match &text[digits..] {
            "min" => Some(Unit::Minutes),
            "h" => Some(Unit::Hours),
            "d" => Some(Unit::Days),
            "w" => Some(Unit::Weeks),
            "mo" => Some(Unit::Months),
            "y" => Some(Unit::Years),
            _ => None,
        };
        if let Some(unit) = unit {
            if !(1..=6).contains(&digits) {
                return None;
            }
            return Some(Self::Relative {
                amount: text[..digits].parse().ok()?,
                unit,
            });
        }

        let number = |range: Range<usize>| -> Option<i64> {
            let part = text.get(range)?;
            match part.bytes().all(|byte| byte.is_ascii_digit()) {
                true => part.parse().ok(),
                false => None,
            }
        };
        let year = number(0..4)?;
        match text.len() {
            4 => return Some(Self::Year(year)),
            7 if &text[4..5] == "-" => {
                let month = number(5..7)? as u32;
                return (1..=12)
                    .contains(&month)
                    .then_some(Self::Month { year, month });
            }
            _ => {}
        }

        if text.len() < 10 || &text[4..5] != "-" || &text[7..8] != "-" {
            return None;
        }
        let month = number(5..7)? as u32;
        let day = number(8..10)? as u32;
        if !(1..=12).contains(&month) || day == 0 || day > days_in(year, month) {
            return None;
        }
        if text.len() == 10 {
            return Some(Self::Day { year, month, day });
        }

        let rest = &text[10..];
        let rest = rest.strip_prefix('T')?;
        let field = |text: &str, max: i64| -> Option<i64> {
            let value = (text.len() == 2 && text.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| text.parse::<i64>().ok())??;
            (value <= max).then_some(value)
        };
        let hour = field(rest.get(0..2)?, 23)?;
        if rest.get(2..3)? != ":" {
            return None;
        }
        let minute = field(rest.get(3..5)?, 59)?;
        let mut rest = &rest[5..];
        let mut second = None;
        if let Some(after) = rest.strip_prefix(':') {
            second = Some(field(after.get(0..2)?, 59)?);
            rest = &after[2..];
        }
        let offset = match rest {
            "" => None,
            "Z" => Some(0),
            _ => {
                let sign = match rest.get(0..1)? {
                    "+" => 1,
                    "-" => -1,
                    _ => return None,
                };
                let hours = field(rest.get(1..3)?, 23)?;
                if rest.get(3..4)? != ":" || rest.len() != 6 {
                    return None;
                }
                let minutes = field(rest.get(4..6)?, 59)?;
                Some(sign * (hours * 3600 + minutes * 60))
            }
        };

        let wall =
            Instant::at(year, month, day, hour as u32, minute as u32, 0).0 + second.unwrap_or(0);
        Some(Self::Instant {
            wall,
            second: second.is_some(),
            offset,
        })
    }

    /// Its span: from its first instant to the first after it.
    fn span(&self, now: i64, zone: Zone) -> Option<(i64, i64)> {
        let day = |year: i64, month: u32, day: u32| Instant::at(year, month, day, 0, 0, 0).0;
        let local = |wall: i64| zone.instant(wall);
        match *self {
            Self::Year(year) => Some((local(day(year, 1, 1))?, local(day(year + 1, 1, 1))?)),
            Self::Month { year, month } => {
                let (next_year, next_month) = match month {
                    12 => (year + 1, 1),
                    month => (year, month + 1),
                };
                Some((
                    local(day(year, month, 1))?,
                    local(day(next_year, next_month, 1))?,
                ))
            }
            Self::Day {
                year,
                month,
                day: date,
            } => {
                let start = day(year, month, date);
                Some((local(start)?, local(start + 86_400)?))
            }
            Self::Instant {
                wall,
                second,
                offset,
            } => {
                let start = match offset {
                    Some(offset) => wall - offset,
                    None => local(wall)?,
                };
                Some((start, start + if second { 1 } else { 60 }))
            }
            Self::Today | Self::Yesterday => {
                let today = zone.wall(now)?.div_euclid(86_400) * 86_400;
                let start = match self {
                    Self::Today => today,
                    _ => today - 86_400,
                };
                Some((local(start)?, local(start + 86_400)?))
            }
            Self::Relative { .. } => None,
        }
    }
}

/// The stamps a `stamp-value` covers, both ends included, either open;
/// none for a value §8.3 refuses.
pub fn stamps(value: &str) -> Option<(Option<i64>, Option<i64>)> {
    let stamp = |text: &str| -> Option<i64> {
        match (1..=18).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_digit()) {
            true => text.parse().ok(),
            false => None,
        }
    };
    match value.split_once("..") {
        None => {
            let stamp = stamp(value)?;
            Some((Some(stamp), Some(stamp)))
        }
        Some(("", "")) => None,
        Some((from, to)) => Some((
            match from {
                "" => None,
                from => Some(stamp(from)?),
            },
            match to {
                "" => None,
                to => Some(stamp(to)?),
            },
        )),
    }
}

/// The instant `amount` `unit`s before `now`, a month or a year counted on
/// UTC's calendar to the same day and time, a day the month reached lacks
/// landing on its last.
fn relative(now: i64, amount: i64, unit: Unit) -> Option<i64> {
    let seconds = match unit {
        Unit::Minutes => 60,
        Unit::Hours => 3600,
        Unit::Days => 86_400,
        Unit::Weeks => 7 * 86_400,
        Unit::Months | Unit::Years => {
            let (year, month, day, time) = civil(now);
            let months = match unit {
                Unit::Months => amount,
                _ => amount * 12,
            };
            let index = year * 12 + i64::from(month) - 1 - months;
            let (year, month) = (index.div_euclid(12), index.rem_euclid(12) as u32 + 1);
            let day = day.min(days_in(year, month));
            return Some(Instant::at(year, month, day, 0, 0, 0).0 + time);
        }
    };
    Some(now - amount * seconds)
}

fn civil(at: i64) -> (i64, u32, u32, i64) {
    let (year, month, day, ..) = Instant(at).parts();
    (year, month, day, at.rem_euclid(86_400))
}

fn days_in(year: i64, month: u32) -> u32 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The midnight, UTC, of the last Sunday of `month`.
fn last_sunday(year: i64, month: u32) -> i64 {
    let last = Instant::at(year, month, days_in(year, month), 0, 0, 0).0;
    let weekday = (last.div_euclid(86_400) + 4).rem_euclid(7);
    last - weekday * 86_400
}

#[cfg(test)]
mod tests {
    use crate::{
        query::date::{DateValue, Span, Zone, stamps},
        time::Instant,
    };

    fn at(text: &str) -> i64 {
        Instant::from_rfc3339(text).0
    }

    #[test]
    fn a_point_covers_its_span_in_the_zone_stated() {
        let now = at("2026-08-20T12:00:00Z");
        let span = |value: &str, zone| DateValue::parse(value).unwrap().span(now, zone);
        let between = |start: &str, end: &str| {
            Some(Span {
                start: Some(at(start)),
                end: Some(at(end)),
            })
        };

        assert_eq!(
            span("2026-08", Zone::Utc),
            between("2026-08-01T00:00:00Z", "2026-09-01T00:00:00Z")
        );
        assert_eq!(
            span("2026-08-01", Zone::Paris),
            between("2026-07-31T22:00:00Z", "2026-08-01T22:00:00Z")
        );
        assert_eq!(
            span("2026-10-25", Zone::Paris),
            between("2026-10-24T22:00:00Z", "2026-10-25T23:00:00Z")
        );
        assert_eq!(
            span("2026-03-29T02:30", Zone::Paris),
            between("2026-03-29T01:30:00Z", "2026-03-29T01:31:00Z")
        );
        assert_eq!(
            span("2026-08-01T12:00:30+02:00", Zone::Utc),
            between("2026-08-01T10:00:30Z", "2026-08-01T10:00:31Z")
        );
        assert_eq!(
            span("1mo", Zone::Utc),
            between("2026-07-20T12:00:00Z", "2026-08-20T12:00:00Z")
        );
        assert_eq!(
            span("2w..2w", Zone::Utc),
            between("2026-08-06T12:00:00Z", "2026-08-06T12:00:00Z")
        );
        assert_eq!(
            span("..2w", Zone::Utc),
            Some(Span {
                start: None,
                end: Some(at("2026-08-06T12:00:00Z"))
            })
        );
        assert_eq!(
            span("..9999", Zone::Utc),
            Some(Span {
                start: None,
                end: None
            })
        );
        assert!(DateValue::parse("2026-02-30").is_none());
        assert!(DateValue::parse("2026-08-01T24:00").is_none());
        assert_eq!(stamps("5.."), Some((Some(5), None)));
        assert_eq!(stamps("1234567890123456789"), None);
    }

    #[test]
    fn a_month_back_lands_on_the_last_day_the_month_has() {
        let now = at("2026-03-31T08:00:00Z");
        let span = DateValue::parse("1mo").unwrap().span(now, Zone::Utc);
        assert_eq!(span.unwrap().start, Some(at("2026-02-28T08:00:00Z")));
    }
}
