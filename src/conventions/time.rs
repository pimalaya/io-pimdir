//! The calendar arithmetic the conventions normalise dates with.
//!
//! Both date-carrying kinds fix their `sort_key` as an RFC 3339 instant in UTC
//! at seconds precision, so byte order is chronological order (spec Annex A.1,
//! A.3). Getting there from a wall time and an offset is a civil-calendar
//! conversion and nothing more, which is why it is a few functions here rather
//! than a dependency: the crate is `no_std`, and a date library that reads a
//! clock or a zone database is neither needed nor wanted at this seam.

use alloc::{format, string::String};

/// Days from the Unix epoch to a civil date, proleptic Gregorian.
///
/// Howard Hinnant's `days_from_civil`, which is exact over the whole range of
/// `i32` years and carries no table.
pub fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = year - i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = (year - era * 400) as i64;
    let month = month as i64;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;

    era as i64 * 146_097 + day_of_era - 719_468
}

/// The civil date a day count from the Unix epoch names: the inverse of
/// [`days_from_civil`].
pub fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = (month_prime + if month_prime < 10 { 3 } else { -9 }) as u32;

    (year as i32 + i32::from(month <= 2), month, day)
}

/// The Unix timestamp of a UTC wall time.
pub fn unix(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> i64 {
    days_from_civil(year, month, day) * 86_400
        + hour as i64 * 3_600
        + minute as i64 * 60
        + second as i64
}

/// A Unix timestamp as the RFC 3339 instant in UTC, at seconds precision, that
/// every date-carrying `sort_key` holds.
pub fn rfc3339(timestamp: i64) -> String {
    let days = timestamp.div_euclid(86_400);
    let seconds = timestamp.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);

    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3_600,
        seconds % 3_600 / 60,
        seconds % 60,
    )
}

/// The day of the week, Sunday 0 through Saturday 6, of a day count from the
/// Unix epoch (a Thursday).
pub fn weekday(days: i64) -> u32 {
    (days + 4).rem_euclid(7) as u32
}

/// The number of days in a month.
pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.rem_euclid(4) == 0
            && (year.rem_euclid(100) != 0 || year.rem_euclid(400) == 0) =>
        {
            29
        }
        2 => 28,
        _ => 0,
    }
}

/// Parses `n` decimal digits at `bytes[at..]`.
pub fn digits(bytes: &[u8], at: usize, n: usize) -> Option<u32> {
    let slice = bytes.get(at..at + n)?;
    if !slice.iter().all(u8::is_ascii_digit) {
        return None;
    }

    let mut value = 0;
    for byte in slice {
        value = value * 10 + u32::from(byte - b'0');
    }
    Some(value)
}
