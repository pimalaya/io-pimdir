//! The crate's Annex A derivations against the format's own vectors.
//!
//! Annex A is informative, so nothing in the store enforces it and no
//! error reports a disagreement: two writers of one collection summarise
//! the same body differently, and a reader shows one of them a blank row.
//! The vectors (pimdir SPEC §16) are what a claim to implement the
//! conventions means, derived from the prose rather than from any
//! implementation, this one included.
//!
//! The spec is a sibling checkout rather than a vendored copy, so the
//! test skips when it is absent and runs when the two sit side by side.

use std::{fs, path::PathBuf};

use io_pimdir::{conventions, hash::PimdirHashAlgo};
use serde_json::Value;

/// The canonical spec checkout, beside this one.
fn spec_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .join("pimdir");

    dir.join("vectors/meta.json").is_file().then_some(dir)
}

#[test]
fn every_vector_derives_what_the_conventions_say() {
    let Some(spec) = spec_dir() else {
        eprintln!("skipped: no pimdir spec checkout beside this one");
        return;
    };

    let vectors: Value =
        serde_json::from_str(&fs::read_to_string(spec.join("vectors/meta.json")).unwrap()).unwrap();
    let cases = vectors["cases"].as_array().unwrap();
    assert!(!cases.is_empty(), "the vectors carry no case");

    for case in cases {
        let label = case["label"].as_str().unwrap();
        let kind = case["kind"].as_str().unwrap();

        // NOTE: read as bytes. The fixtures are CRLF throughout, as RFC
        // 5322 and RFC 5545 require, and a harness normalising line
        // endings changes the body and therefore its name.
        let fixture = spec.join("vectors").join(case["fixture"].as_str().unwrap());
        let body = fs::read(&fixture).unwrap();
        assert_eq!(
            body.len(),
            case["body"]["len"].as_u64().unwrap() as usize,
            "{label}: the fixture was not read as bytes"
        );
        assert_eq!(
            PimdirHashAlgo::Blake3.hash(&body).0,
            case["body"]["blake3"].as_str().unwrap(),
            "{label}: the fixture was not read as bytes"
        );

        let derived = conventions::derive(kind, &body).expect(label);

        match case["link_id"].as_str() {
            Some(link_id) => assert_eq!(derived.link_id.0, link_id, "{label}: link id"),
            // NOTE: the format pins no id for content carrying none, so
            // what is checked is that one was derived at all.
            None => assert!(!derived.link_id.0.is_empty(), "{label}: no id was derived"),
        }

        assert_eq!(
            derived.sort_key.0,
            case["sort_key"].as_str().unwrap(),
            "{label}: sort key"
        );

        // NOTE: parsed structures, never JSON text: key order is not
        // fixed by the vectors, and pinning one would pin an accident of
        // whichever serialiser wrote the file.
        let written: Value = serde_json::from_str(&derived.meta.0).unwrap();
        assert_eq!(written, case["meta"], "{label}: meta");
    }
}

/// The two fallbacks the vectors leave open, which this crate fixes so
/// two writers cannot link one item twice and store one body twice.
#[test]
fn a_body_with_no_identity_falls_back_to_a_stable_id() {
    let card = b"BEGIN:VCARD\r\nVERSION:4.0\r\nFN:No UID\r\nEND:VCARD\r\n";
    let derived = conventions::derive("text/vcard", card).unwrap();
    assert_eq!(derived.link_id.0, "hash:e03e3cd94de4c1b2");
    assert_eq!(
        conventions::derive("text/vcard", card).unwrap().link_id,
        derived.link_id,
        "the same body derives the same id"
    );

    let message = b"Subject: No id\r\nFrom: alice@example.org\r\n\
                    Date: Sat, 01 Aug 2026 10:00:00 +0000\r\n\r\nbody\r\n";
    let derived = conventions::derive("message/rfc822", message).unwrap();
    assert_eq!(
        derived.link_id.0, "alt:No id|2026-08-01T10:00:00Z|alice@example.org",
        "a message falls back to what identifies it, not to its bytes: the \
         same message re-fetched at another detail tier must link to the item \
         it already has"
    );
}

/// A media type this crate has no conventions for is neither an error nor
/// a guess: the caller writes its own summary.
#[test]
fn an_unknown_kind_derives_nothing() {
    assert!(conventions::derive("application/octet-stream", b"...").is_none());
    assert!(conventions::derive("text/vcard; charset=utf-8", b"BEGIN:VCARD\r\n").is_some());
}

/// A header value the writer folded, and an address list: neither appears
/// in the vectors, and both are what a real message carries.
#[test]
fn a_folded_header_and_an_address_list_read_as_one_value() {
    let message = b"Subject: a subject the writer\r\n folded across two lines\r\n\
                    From: \"Example, Alice\" <alice@example.org>\r\n\
                    To: bob@example.org, carol@example.org\r\n\
                    Date: 1 Aug 26 10:00 EDT\r\n\r\nbody\r\n";
    let meta: Value = serde_json::from_str(
        &conventions::derive("message/rfc822", message)
            .unwrap()
            .meta
            .0,
    )
    .unwrap();

    assert_eq!(
        meta["subject"],
        "a subject the writer folded across two lines"
    );
    // the display name carries the comma the list splits on, and the
    // seconds and zone are the obsolete forms RFC 5322 §4.3 requires
    assert_eq!(meta["from"], "alice@example.org");
    assert_eq!(meta["to"], "bob@example.org");
    assert_eq!(meta["date"], "2026-08-01T14:00:00Z");
}

/// A resource holding only an override, and a zone that never changes:
/// the two shapes the master and zone lookups fall back on.
#[test]
fn an_override_only_resource_and_a_fixed_zone_still_summarise() {
    let body = b"BEGIN:VCALENDAR\r\n\
                 BEGIN:VTIMEZONE\r\nTZID:Fixed/Plus2\r\n\
                 BEGIN:STANDARD\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0200\r\n\
                 DTSTART:19700101T000000\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n\
                 BEGIN:VEVENT\r\nUID:only-override@example.org\r\n\
                 RECURRENCE-ID;TZID=Fixed/Plus2:20260814T090000\r\n\
                 SUMMARY:Moved instance\r\n\
                 DTSTART;TZID=Fixed/Plus2:20260814T110000\r\n\
                 END:VEVENT\r\nEND:VCALENDAR\r\n";
    let derived = conventions::derive("text/calendar", body).unwrap();

    assert_eq!(derived.link_id.0, "only-override@example.org");
    assert_eq!(derived.sort_key.0, "2026-08-14T09:00:00Z", "+02:00 applied");
}
