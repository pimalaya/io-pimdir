//! Writes a representative store with a released io-pimdir, into
//! $FIXTURE_DIR: the generator of tests/fixtures/store-v*/, which
//! tests/store_version.rs opens with this one.
//!
//! Not compiled here. To regenerate, copy it into tests/ of a `git archive`
//! of the release (v0.5.1, v0.7.0, and 3b9154a for 0.8.0), run
//! `FIXTURE_DIR=<dir> cargo test --test fixture_gen` there, and check in
//! the resulting pimdir.db and objects/.

use std::{io::Write, path::Path};

use io_pimdir::{
    change::{PimdirDropReason, PimdirWriteOp},
    client::{blobs::PimdirBlobs, producer::PimdirProducer, PimdirStore},
    codec::PimdirAction,
    collection::PimdirCollectionId,
    hash::PimdirHashAlgo,
    object::PimdirObject,
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
    summary,
};

const MAIL_A: &[u8] = b"Message-ID: <a@example.org>\r\nFrom: Alice <alice@example.org>\r\nTo: Bob <bob@example.org>\r\nSubject: first\r\nDate: Mon, 05 Oct 2026 10:00:00 +0000\r\n\r\nhello\r\n";
const MAIL_B: &[u8] = b"Message-ID: <b@example.org>\r\nFrom: Carol <carol@example.org>\r\nTo: Bob <bob@example.org>\r\nSubject: invitation\r\nDate: Tue, 06 Oct 2026 10:00:00 +0000\r\nMIME-Version: 1.0\r\nContent-Type: text/calendar; method=REQUEST; charset=utf-8\r\n\r\nBEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//fixture//EN\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:meet-1@example.org\r\nDTSTAMP:20261006T100000Z\r\nDTSTART:20261010T100000Z\r\nDTEND:20261010T110000Z\r\nSUMMARY:Meeting\r\nORGANIZER:mailto:carol@example.org\r\nATTENDEE:mailto:bob@example.org\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
const MAIL_C: &[u8] = b"Message-ID: <c@example.org>\r\nFrom: Dave <dave@example.org>\r\nTo: Bob <bob@example.org>\r\nSubject: retained\r\nDate: Wed, 07 Oct 2026 10:00:00 +0000\r\n\r\nbye\r\n";
const MAIL_NEW: &[u8] = b"Message-ID: <new@example.org>\r\nFrom: Bob <bob@example.org>\r\nTo: Alice <alice@example.org>\r\nSubject: queued\r\nDate: Thu, 08 Oct 2026 10:00:00 +0000\r\n\r\nqueued body\r\n";
const CARD: &[u8] = b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:card-1\r\nFN:Alice Example\r\nEMAIL:alice@example.org\r\nEND:VCARD\r\n";
const TASK: &[u8] = b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//fixture//EN\r\nBEGIN:VTODO\r\nUID:task-1@example.org\r\nDTSTAMP:20261006T100000Z\r\nSUMMARY:Write the fixture\r\nPRIORITY:1\r\nCATEGORIES:work,home\r\nDUE:20261101T100000Z\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
const LIST: &[u8] = b"a list in a custom kind\n";

fn placement(collection: &str, kind: &str, handle: &str, body: &[u8]) -> (PimdirWriteOp, PimdirWriteOp) {
    let hash = PimdirHashAlgo::default().hash(body);
    let derived = summary::derive(kind, body);
    let link_id = derived
        .as_ref()
        .map(|d| d.link_id.clone())
        .unwrap_or_else(|| PimdirLinkId(format!("hash:{handle}")));
    let flags = PimdirFlags::from_iter(["\\Seen"]);
    let placement = PimdirPlacement {
        sort_key: derived.as_ref().map(|d| d.sort_key.clone()).unwrap_or_default(),
        collection: PimdirCollectionId(collection.into()),
        handle: PimdirHandle(handle.into()),
        link_id: Some(link_id),
        object: Some(hash.clone()),
        level: PimdirLevel::Full,
        summary: derived.and_then(|d| d.summary),
        flags: flags.clone(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags,
            revision: Some(format!("rev-{handle}")),
            object: Some(hash.clone()),
        }),
        origin: None,
    };
    let object = PimdirWriteOp::StoreObject {
        object: PimdirObject {
            hash,
            size: body.len(),
        },
        body: Some(body.to_vec()),
    };
    (object, PimdirWriteOp::UpsertPlacement(placement))
}

fn write_blob(dir: &Path, body: &[u8]) -> PimdirObject {
    let blobs = PimdirBlobs::open(dir, PimdirHashAlgo::default());
    let hash = blobs.hash(body);
    let mut writer = blobs.writer().unwrap();
    writer.write_all(body).unwrap();
    let size = writer.commit(&hash).unwrap();
    PimdirObject {
        hash,
        size: size as usize,
    }
}

#[test]
fn generate() {
    let dir = std::env::var("FIXTURE_DIR").expect("FIXTURE_DIR");
    let dir = Path::new(&dir);
    std::fs::create_dir_all(dir).unwrap();

    {
        let mut store = PimdirStore::open(dir).unwrap().for_source("imap");
        store.ensure_collection("INBOX", "message/rfc822").unwrap();
        store.ensure_collection("Contacts", "text/vcard").unwrap();
        store.ensure_collection("Tasks", "text/calendar").unwrap();
        store.ensure_collection("Lists", "text/x-vlist").unwrap();

        let mut ops = Vec::new();
        for (collection, kind, handle, body) in [
            ("INBOX", "message/rfc822", "1", MAIL_A),
            ("INBOX", "message/rfc822", "2", MAIL_B),
            ("INBOX", "message/rfc822", "3", MAIL_C),
            ("Contacts", "text/vcard", "c1", CARD),
            ("Tasks", "text/calendar", "t1", TASK),
            ("Lists", "text/x-vlist", "l1", LIST),
        ] {
            let (object, upsert) = placement(collection, kind, handle, body);
            ops.push(object);
            ops.push(upsert);
        }
        store.write(ops).unwrap();

        // the server dropped message 3: retained, not deleted
        store
            .write(vec![PimdirWriteOp::DropPlacement {
                collection: PimdirCollectionId("INBOX".into()),
                handle: PimdirHandle("3".into()),
                reason: PimdirDropReason::Deleted,
            }])
            .unwrap();

        let seq_a = store
            .list_items("INBOX", None, 10)
            .unwrap()
            .into_iter()
            .find(|i| i.link_id.0.contains("a@example.org"))
            .unwrap()
            .seq;

        // a parked row: flags of an unknown seq
        let mut producer = PimdirProducer::open(dir, "fixture").unwrap();
        producer
            .enqueue(
                "INBOX",
                &PimdirAction::SetFlags {
                    seq: 9999,
                    flags: PimdirFlags::from_iter(["\\Flagged"]),
                },
                None,
            )
            .unwrap();
        // an applied row (a receipt where the release keeps them)
        producer
            .enqueue(
                "INBOX",
                &PimdirAction::SetFlags {
                    seq: seq_a,
                    flags: PimdirFlags::from_iter(["\\Seen", "\\Answered"]),
                },
                None,
            )
            .unwrap();
        drop(producer);
        let report = store.drain().unwrap();
        assert_eq!((report.applied, report.parked), (1, 1));

        // pending rows: a create with its body, a flag change
        let object = write_blob(dir, MAIL_NEW);
        let mut producer = PimdirProducer::open(dir, "fixture").unwrap();
        producer
            .enqueue(
                "INBOX",
                &PimdirAction::Add {
                    link_id: None,
                    flags: PimdirFlags::from_iter(["\\Seen"]),
                    object: Some(object.hash.clone()),
                },
                Some(&object),
            )
            .unwrap();
        producer
            .enqueue(
                "INBOX",
                &PimdirAction::SetFlags {
                    seq: seq_a,
                    flags: PimdirFlags::from_iter(["\\Flagged"]),
                },
                None,
            )
            .unwrap();
        assert_eq!(store.list_pending_actions().unwrap().len(), 2);
        assert_eq!(store.parked_actions().unwrap().len(), 1);
        assert_eq!(store.list_retained("INBOX", None, 10).unwrap().len(), 1);
    }

    // fold the WAL into the file, so pimdir.db alone is the store
    let conn = rusqlite::Connection::open(dir.join("pimdir.db")).unwrap();
    conn.pragma_update(None, "journal_mode", "DELETE").unwrap();
}
