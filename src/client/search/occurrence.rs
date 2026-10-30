//! # Occurrences
//!
//! Calendar time in the index (SEARCH §7): every occurrence of a calendar
//! item within the horizon, one year back and two forward, materialised
//! as instants into `occurrence`, so a date term reaches the twelfth
//! Friday of a series whose sort key holds its first.
//!
//! An item is expanded when its body changes, and again when the horizon
//! rolls or the time zone database the instants were resolved through
//! names another version (`items_to_reexpand`). Expansion is ical-rs's:
//! the resource's own `VTIMEZONE`s, else the bundled database by `TZID`,
//! else floating. One item writes at most [`OCCURRENCES_MAX`].

use alloc::{string::String, vec::Vec};

use ical::{recur::instant::IcalRecurTimeline, tree::cst::IcalCst, tzdb, value::IcalValue};
use log::{debug, trace};
use rusqlite::{OptionalExtension, Transaction, named_params};

use crate::{
    client::{
        PimdirError, blobs::PimdirBlobs, rows, schema::StoreShape, search::PimdirIndexReport,
    },
    object::PimdirHash,
    sql::search,
    summary::time,
};

/// The most occurrences one calendar item writes (SEARCH §7): a series
/// past it is cut, and the refresh report counts it.
pub const OCCURRENCES_MAX: usize = 1000;

/// The seconds a horizon has to move by before the index rolls it: a
/// day, so a refresh re-expands at most once a day.
const ROLL: i64 = 86_400;

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

    /// The horizon `index_meta` records, `None` before one was.
    pub(crate) fn recorded(tx: &Transaction) -> Result<Option<Self>, PimdirError> {
        let (start, end): (String, String) =
            tx.query_row(search::LOAD_INDEX_META, [], |r| Ok((r.get(4)?, r.get(5)?)))?;
        Ok(instant(&start)
            .zip(instant(&end))
            .map(|(start, end)| Self { start, end }))
    }
}

/// Rolls the horizon to the one around `now` when it moved by a day or
/// more, and re-expands what `items_to_reexpand` names when it rolled or
/// the time zone database names another version than the one recorded
/// (SEARCH §7), in the caller's transaction. Answers the horizon in force.
pub(crate) fn roll(
    tx: &Transaction,
    blobs: &PimdirBlobs,
    shape: StoreShape,
    now: i64,
    report: &mut PimdirIndexReport,
) -> Result<Horizon, PimdirError> {
    let around = Horizon::around(now);
    let version = tzdb_version();
    let recorded: String = tx.query_row(search::LOAD_INDEX_META, [], |r| r.get(6))?;

    let Some(old) = Horizon::recorded(tx)? else {
        record(tx, &around, version)?;
        return Ok(around);
    };
    let rolled = (around.start - old.start).abs() >= ROLL;
    if !rolled && recorded == version {
        return Ok(old);
    }

    let new = if rolled { around } else { old };
    debug!(
        "re-expanding calendar items for the horizon {}..{} under tzdb {version:?}",
        time::rfc3339(new.start),
        time::rfc3339(new.end)
    );
    let items = rows(
        tx,
        &shape.sql(tx, search::ITEMS_TO_REEXPAND)?,
        named_params! {
            ":old_start": time::rfc3339(old.start),
            ":old_end": time::rfc3339(old.end),
            ":new_start": time::rfc3339(new.start),
            ":new_end": time::rfc3339(new.end),
            ":tzdb_version": version,
        },
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
    )?;
    for (collection, seq) in items {
        expand(tx, blobs, &collection, seq, &new, report)?;
    }

    record(tx, &new, version)?;
    Ok(new)
}

/// Replaces one placement's occurrences with those of the body the index
/// holds for it within the horizon; none when it holds no body, the blob
/// is gone, or no expander accepts the series.
pub(crate) fn expand(
    tx: &Transaction,
    blobs: &PimdirBlobs,
    collection: &str,
    seq: i64,
    horizon: &Horizon,
    report: &mut PimdirIndexReport,
) -> Result<(), PimdirError> {
    let key = named_params! { ":collection": collection, ":seq": seq };
    tx.execute(search::REPLACE_OCCURRENCES, key)?;

    let hash: Option<String> = tx
        .query_row(search::PLACEMENT_HASH, key, |r| r.get(0))
        .optional()?
        .flatten();
    let Some(hash) = hash else {
        return Ok(());
    };
    let Some(body) = blobs.get(&PimdirHash(hash))? else {
        return Ok(());
    };

    let (occurrences, cut) = occurrences(&body, horizon);
    trace!("{} occurrences of {collection}/{seq}", occurrences.len());
    for (start, end) in occurrences {
        tx.execute(
            search::INSERT_OCCURRENCE,
            named_params! {
                ":collection": collection,
                ":seq": seq,
                ":start": time::rfc3339(start),
                ":end": time::rfc3339(end),
            },
        )?;
    }

    report.expanded += 1;
    if cut {
        report.bounded += 1;
    }
    Ok(())
}

/// Records the horizon and the time zone database version in force.
fn record(tx: &Transaction, horizon: &Horizon, version: &str) -> Result<(), PimdirError> {
    tx.execute(
        search::SET_HORIZON,
        named_params! {
            ":horizon_start": time::rfc3339(horizon.start),
            ":horizon_end": time::rfc3339(horizon.end),
        },
    )?;
    tx.execute(
        search::SET_TZDB_VERSION,
        named_params! { ":tzdb_version": version },
    )?;
    Ok(())
}

/// The occurrences of a calendar resource overlapping the horizon, as
/// `(start, end)` Unix seconds, at most [`OCCURRENCES_MAX`], and whether
/// the resource held more: one timeline per `UID` of its events, tasks
/// and journals, overrides included, every time resolved through its own
/// `VTIMEZONE`s, else the bundled database, else floating, read on UTC.
/// A resource no parser accepts, or a component naming no `UID`, yields
/// nothing.
fn occurrences(body: &[u8], horizon: &Horizon) -> (Vec<(i64, i64)>, bool) {
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
            IcalValue::Text(uid) if prop.name.eq_ignore_ascii_case("UID") => Some(uid.0.as_ref()),
            _ => None,
        });
        if let Some(uid) = uid.filter(|uid| item && !uids.contains(uid)) {
            uids.push(uid);
        }
    }

    let through = time::civil_from_days(horizon.end.div_euclid(86_400)).0 + 1;
    let zones = tzdb::zones(&calendar.components, through);
    let mut occurrences = Vec::new();
    for uid in uids {
        let timeline = IcalRecurTimeline::of_uid(&calendar.components, uid, &zones);
        let room = OCCURRENCES_MAX + 1 - occurrences.len();
        occurrences.extend(
            timeline
                .expand(horizon.start, horizon.end)
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

/// The version of the time zone database occurrences resolve through,
/// empty with none.
fn tzdb_version() -> &'static str {
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
    use crate::{client::search::occurrence::*, summary::time};

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
        let (occurrences, cut) = occurrences(body.as_bytes(), &horizon);
        assert_eq!(occurrences.len(), OCCURRENCES_MAX);
        assert!(cut);
    }
}
