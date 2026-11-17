//! A `VTIMEZONE` observance's rule against the calendar derivation
//! (Annex A.3): whatever its `FREQ`, `INTERVAL`, `COUNT` and `UNTIL`, the
//! derivation answers, and a rule whose next onset lies past the last
//! year a date spells keys the event as its `DTSTART` alone would.

use io_pimdir::summary::calendar;
use proptest::prelude::*;

/// A one-observance zone stating `rule`, and an event in it.
fn resource(rule: Option<&str>, from: &str) -> String {
    let rule = rule
        .map(|rule| format!("RRULE:{rule}\r\n"))
        .unwrap_or_default();
    format!(
        "BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:Z\r\nBEGIN:STANDARD\r\n\
         DTSTART:20260101T000000\r\nTZOFFSETFROM:{from}\r\nTZOFFSETTO:+0200\r\n\
         {rule}END:STANDARD\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:x\r\n\
         DTSTART;TZID=Z:20260801T120000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )
}

fn freq() -> impl Strategy<Value = (&'static str, i64)> {
    prop_oneof![
        Just(("SECONDLY", 1)),
        Just(("MINUTELY", 60)),
        Just(("HOURLY", 3_600)),
        Just(("DAILY", 86_400)),
        Just(("WEEKLY", 7 * 86_400)),
    ]
}

proptest! {
    #[test]
    fn a_rule_stepping_past_year_9999_keys_as_its_dtstart(
        (freq, step) in freq(),
        interval in 1..=i64::MAX,
        from in prop_oneof![Just("-1200"), Just("+0000"), Just("+1400")],
        until in proptest::option::of(Just(";UNTIL=99991231T235959Z")),
        count in proptest::option::of(1..=u32::MAX),
    ) {
        let mut rule = format!("FREQ={freq};INTERVAL={interval}");
        rule.push_str(until.unwrap_or_default());
        if let Some(count) = count {
            rule.push_str(&format!(";COUNT={count}"));
        }

        let derived = calendar::derive(resource(Some(&rule), from).as_bytes());
        let alone = calendar::derive(resource(None, from).as_bytes());
        let past = i128::from(step) * i128::from(interval) > 253_402_300_800;
        if past {
            prop_assert_eq!(derived.sort_key, alone.sort_key);
        }
    }

    #[test]
    fn a_yearly_rule_of_any_interval_is_derived(
        interval in 1..=i64::MAX,
        month in 1..=12u32,
        ordinal in -5..=5i32,
    ) {
        let rule = format!("FREQ=YEARLY;INTERVAL={interval};BYMONTH={month};BYDAY={ordinal}SU");
        let derived = calendar::derive(resource(Some(&rule), "+0000").as_bytes());
        prop_assert!(!derived.sort_key.as_str().is_empty());
    }
}
