//! A mail sync lists newest first within a scope on the `Date` (pimdir
//! SYNC.md §4, §5; STORAGE §4.3, §11.3, §14.1): members arrive named, a
//! round lands page by page and closes with its coverage, which every
//! reader lists with the collection, the owner collects below a date,
//! and the mail readers count, page and search across collections.

use std::collections::BTreeMap;

use io_pimdir::{
    change::PimdirChange,
    client::{
        PimdirError, PimdirRunError, PimdirStore,
        reader::{PimdirMailFilter, PimdirReader, like_pattern},
    },
    collection::{PimdirCheckpoint, PimdirCollectionId, PimdirCursor, PimdirScope},
    placement::{PimdirFlags, PimdirHandle, PimdirLevel},
    remote::{
        PimdirEnumerate, PimdirEnumerated, PimdirFetchedItem, PimdirPushOutcome, PimdirPushResult,
        PimdirRemote, PimdirRemoteItem, PimdirRemoteMeta, PimdirRemoteSnapshot, PimdirTier,
    },
    summary::mail,
    sync::PimdirSyncOptions,
};

/// A message dated `date`, with a `Content-Type` of `kind`.
fn message(id: &str, subject: &str, date: &str, kind: &str) -> Vec<u8> {
    format!(
        "Message-ID: <{id}>\r\nFrom: Alice <alice@example.org>\r\nTo: bob@example.org\r\n\
         Subject: {subject}\r\nDate: {date}\r\nContent-Type: {kind}\r\n\r\nbody\r\n"
    )
    .into_bytes()
}

/// A member as a listing names it, from its header block (Annex A.1).
fn member(handle: &str, flags: &[&str], body: &[u8]) -> PimdirRemoteItem {
    PimdirRemoteItem {
        handle: PimdirHandle::from(handle),
        flags: PimdirFlags::from_iter(flags.iter().copied()),
        revision: None,
        meta: PimdirRemoteMeta::new(mail::derive_meta(body, Some(body.len() as u64), None)),
    }
}

/// A remote answering the pages it is given, failing once they run out
/// when `fail` says so, and accepting every push.
#[derive(Default)]
struct Paged {
    pages: Vec<PimdirEnumerated>,
    requests: Vec<PimdirEnumerate>,
    bound: bool,
}

impl PimdirRemote for Paged {
    type Error = &'static str;

    fn enumerate(
        &mut self,
        _: &PimdirCollectionId,
        request: PimdirEnumerate,
    ) -> Result<PimdirEnumerated, &'static str> {
        self.requests.push(request);
        if self.pages.is_empty() {
            return Err("connection lost");
        }
        Ok(self.pages.remove(0))
    }

    fn fetch(
        &mut self,
        _: &PimdirCollectionId,
        _: Vec<PimdirHandle>,
        _: PimdirTier,
    ) -> Result<Vec<PimdirFetchedItem>, &'static str> {
        Ok(Vec::new())
    }

    fn push(
        &mut self,
        _: &PimdirCollectionId,
        changes: Vec<PimdirChange>,
    ) -> Result<Vec<PimdirPushResult>, &'static str> {
        Ok(changes
            .iter()
            .map(|change| PimdirPushResult {
                handle: change.handle().clone(),
                outcome: PimdirPushOutcome::Accepted,
                assigned: None,
                revision: None,
            })
            .collect())
    }

    fn scope_bound(&self) -> bool {
        self.bound
    }
}

fn page(items: Vec<PimdirRemoteItem>, cursor: Option<&str>) -> PimdirEnumerated {
    PimdirEnumerated::Page(PimdirRemoteSnapshot::page(
        items,
        cursor.map(|cursor| PimdirCursor(cursor.as_bytes().to_vec())),
        Some(PimdirCheckpoint(b"c1".to_vec())),
    ))
}

fn since(date: &str) -> PimdirSyncOptions {
    PimdirSyncOptions {
        scope: PimdirScope::since(date),
        ..Default::default()
    }
}

/// An interrupted round keeps what landed, is listed with no coverage,
/// resumes from its cursor and closes with the coverage of its scope.
#[test]
fn an_interrupted_round_resumes_and_closes_with_its_coverage() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("imap");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    let recent = message(
        "a@x",
        "Recent",
        "Mon, 5 Oct 2026 10:00:00 +0000",
        "text/plain",
    );
    let older = message(
        "b@x",
        "Older",
        "Tue, 1 Sep 2026 10:00:00 +0000",
        "text/plain",
    );

    let mut remote = Paged {
        pages: vec![page(vec![member("2", &[], &recent)], Some("p1"))],
        ..Default::default()
    };
    let opts = since("2026-09-01T00:00:00Z");
    let interrupted = store.sync("INBOX", opts.clone(), &mut remote);
    assert!(matches!(interrupted, Err(PimdirRunError::Remote(_))));

    let reader = PimdirReader::open(dir.path()).unwrap();
    assert_eq!(reader.count_items("INBOX").unwrap(), 1, "the page landed");
    let coverage = reader.list_coverage("INBOX").unwrap();
    assert_eq!(coverage.len(), 1);
    assert_eq!(coverage[0].coverage, None, "no round ever closed");
    assert_eq!(
        coverage[0].round.as_ref().map(|round| &round.scope),
        Some(&PimdirScope::since("2026-09-01T00:00:00Z")),
    );
    assert_eq!(reader.list_collections().unwrap()[0].coverage, None);

    remote.pages = vec![page(vec![member("1", &[], &older)], None)];
    let report = store.sync("INBOX", opts, &mut remote).unwrap();
    assert_eq!(report.pulled, 1);
    assert_eq!(
        remote.requests.last().and_then(|request| request.cursor()),
        Some(&PimdirCursor(b"p1".to_vec())),
        "the round resumed from its cursor",
    );

    let collections = reader.list_collections().unwrap();
    let coverage = collections[0].coverage.as_ref().expect("a coverage");
    assert_eq!(coverage.scope, PimdirScope::since("2026-09-01T00:00:00Z"));
    assert!(reader.list_coverage("INBOX").unwrap()[0].round.is_none());
}

/// A collection's coverage is the narrowest of its sources', and none
/// while one of them has never closed a round.
#[test]
fn a_collection_is_listed_with_the_narrowest_coverage() {
    let dir = tempfile::tempdir().unwrap();
    let body = message("a@x", "Hi", "Mon, 5 Oct 2026 10:00:00 +0000", "text/plain");
    let mut imap = PimdirStore::open(dir.path()).unwrap().for_source("imap");
    imap.ensure_collection("INBOX", "message/rfc822").unwrap();

    let mut remote = Paged {
        pages: vec![page(vec![member("1", &[], &body)], None)],
        ..Default::default()
    };
    imap.sync("INBOX", since("2026-09-01T00:00:00Z"), &mut remote)
        .unwrap();
    drop(imap);

    let mut phone = PimdirStore::open(dir.path()).unwrap().for_source("phone");
    let mut remote = Paged {
        pages: vec![page(vec![member("p1", &[], &body)], Some("next"))],
        ..Default::default()
    };
    let _ = phone.sync("INBOX", since("2026-10-01T00:00:00Z"), &mut remote);
    assert_eq!(
        phone.list_collections().unwrap()[0].coverage,
        None,
        "the phone never closed a round"
    );

    remote.pages = vec![page(Vec::new(), None)];
    phone
        .sync("INBOX", since("2026-10-01T00:00:00Z"), &mut remote)
        .unwrap();
    let collections = phone.list_collections().unwrap();
    assert_eq!(
        collections[0].coverage.as_ref().map(|c| &c.scope),
        Some(&PimdirScope::since("2026-10-01T00:00:00Z")),
        "the latest floor of the two",
    );
}

/// A bounded scope is mail's: another kind refuses it, naming the kind.
#[test]
fn a_scope_on_another_kind_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("dav");
    store.ensure_collection("Contacts", "text/vcard").unwrap();

    let refused = store
        .sync(
            "Contacts",
            since("2026-09-01T00:00:00Z"),
            &mut Paged::default(),
        )
        .unwrap_err();
    assert!(
        matches!(&refused, PimdirRunError::Store(PimdirError::Scope { kind, .. }) if kind == "text/vcard"),
        "{refused}",
    );
}

/// The owner's collection below a date removes what owes nothing, keeps
/// what does and what has no date, and pushes nothing (STORAGE §11.3).
#[test]
fn collecting_below_a_date_keeps_what_is_owed() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("imap");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    let old = message(
        "old@x",
        "Old",
        "Sat, 1 Aug 2026 10:00:00 +0000",
        "text/plain",
    );
    let owed = message(
        "owed@x",
        "Owed",
        "Sun, 2 Aug 2026 10:00:00 +0000",
        "text/plain",
    );
    let undated = message("undated@x", "When", "someday", "text/plain");
    let recent = message(
        "new@x",
        "New",
        "Mon, 5 Oct 2026 10:00:00 +0000",
        "text/plain",
    );

    let mut remote = Paged {
        pages: vec![page(
            vec![
                member("1", &[], &old),
                member("2", &[], &owed),
                member("3", &[], &undated),
                member("4", &[], &recent),
            ],
            None,
        )],
        ..Default::default()
    };
    store
        .sync("INBOX", PimdirSyncOptions::default(), &mut remote)
        .unwrap();
    store
        .mutate(
            "INBOX",
            io_pimdir::mutate::PimdirMutation::SetFlags {
                handle: PimdirHandle::from("2"),
                flags: PimdirFlags::from_iter(["\\Flagged"]),
            },
        )
        .unwrap();

    let old_seq = store.seq_for_link("INBOX", "old@x").unwrap().unwrap();
    let report = store
        .collect_before("INBOX", "2026-10-01T00:00:00Z")
        .unwrap();
    assert_eq!(report.seqs, vec![old_seq]);
    assert_eq!(store.count_items("INBOX").unwrap(), 3);
    assert_eq!(store.count_retained("INBOX").unwrap(), 0, "no tombstone");

    // NOTE: the next delta derives no push for what was collected.
    remote.pages = vec![PimdirEnumerated::Page(PimdirRemoteSnapshot::delta(
        Vec::new(),
        Vec::new(),
        PimdirCheckpoint(b"c2".to_vec()),
    ))];
    let report = store
        .sync("INBOX", PimdirSyncOptions::default(), &mut remote)
        .unwrap();
    assert_eq!(
        report.pushed, 1,
        "the owed flag, and nothing for the collected"
    );
}

/// The mail readers: counts under the chips, per day and unread, pages
/// across collections on one cursor, and the sender and subject search.
#[test]
fn the_mail_readers_count_page_and_search_across_collections() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("imap");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    store
        .ensure_collection("Archive", "message/rfc822")
        .unwrap();

    let mixed = "multipart/mixed; boundary=b";
    let inbox = vec![
        member(
            "1",
            &["\\Seen"],
            &message(
                "1@x",
                "Invoice 100%",
                "Mon, 5 Oct 2026 10:00:00 +0000",
                mixed,
            ),
        ),
        member(
            "2",
            &[],
            &message(
                "2@x",
                "Lunch",
                "Mon, 5 Oct 2026 12:00:00 +0000",
                "text/plain",
            ),
        ),
        member(
            "3",
            &[],
            &message(
                "3@x",
                "Report",
                "Sun, 4 Oct 2026 09:00:00 +0000",
                "text/plain",
            ),
        ),
    ];
    let archive = vec![member(
        "9",
        &["\\Seen"],
        &message(
            "9@x",
            "Old invoice",
            "Sat, 3 Oct 2026 09:00:00 +0000",
            mixed,
        ),
    )];
    for (collection, items) in [("INBOX", inbox), ("Archive", archive)] {
        let mut remote = Paged {
            pages: vec![page(items, None)],
            ..Default::default()
        };
        store
            .sync(collection, PimdirSyncOptions::default(), &mut remote)
            .unwrap();
    }

    let reader = PimdirReader::open(dir.path()).unwrap();
    let both = ["INBOX", "Archive"];
    let any = PimdirMailFilter::default();
    assert_eq!(reader.count_mail(&both, any).unwrap(), 4);
    let unread = PimdirMailFilter {
        seen: Some(false),
        ..any
    };
    assert_eq!(reader.count_mail(&both, unread).unwrap(), 2);
    let attached = PimdirMailFilter {
        attachment: Some(true),
        ..any
    };
    assert_eq!(reader.count_mail(&both, attached).unwrap(), 2);

    let days: BTreeMap<Option<String>, u64> = reader
        .count_mail_by_day(&both, any, None)
        .unwrap()
        .into_iter()
        .map(|day| (day.day, day.count))
        .collect();
    assert_eq!(days[&Some("2026-10-05".into())], 2);
    assert_eq!(days[&Some("2026-10-03".into())], 1);
    let shifted = reader
        .count_mail_by_day(&["INBOX"], any, Some("-11 hours"))
        .unwrap();
    assert_eq!(shifted[0].day.as_deref(), Some("2026-10-05"));
    assert_eq!(shifted[0].count, 1, "10:00 UTC falls the day before");

    let unread = reader.count_unread(&both, None).unwrap();
    assert_eq!(unread.get("INBOX"), Some(&2));
    assert_eq!(unread.get("Archive"), None, "a collection with none");

    let first = reader.list_mail_page_filtered(&both, any, None, 2).unwrap();
    assert_eq!(
        first
            .iter()
            .map(|entry| entry.item.link_id.as_str())
            .collect::<Vec<_>>(),
        ["2@x", "1@x"],
        "newest first"
    );
    assert!(first[0].item.summary.is_some(), "with its summary");
    let rest = reader
        .list_mail_page_filtered(&both, any, Some(&first[1].cursor()), 10)
        .unwrap();
    assert_eq!(
        rest.iter()
            .map(|entry| (entry.collection.as_str(), entry.item.link_id.as_str()))
            .collect::<Vec<_>>(),
        [("INBOX", "3@x"), ("Archive", "9@x")],
    );

    let hits = reader
        .search_mail(&both, &like_pattern("invoice"), any, None, 10)
        .unwrap();
    assert_eq!(hits.len(), 2, "case folded, both collections");
    let hits = reader
        .search_mail(&both, &like_pattern("100%"), any, None, 10)
        .unwrap();
    assert_eq!(hits.len(), 1, "a literal % matches itself");
    let hits = reader
        .search_mail(&both, &like_pattern("alice"), attached, None, 10)
        .unwrap();
    assert_eq!(hits.len(), 2, "the sender's name, under the chip");
    assert_eq!(like_pattern(" a_b\\ "), "%a\\_b\\\\%");
}

/// A store an earlier draft wrote (probes, no coverage, rows at level 0)
/// is read by a reader as it is and reconciled by its owner on open
/// (STORAGE §6): the columns added, the trigger created, the probes
/// dropped, and a level-0 row read as a `Meta` claim to revisit.
#[test]
fn an_earlier_draft_store_is_reconciled_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let body = message("a@x", "Hi", "Mon, 5 Oct 2026 10:00:00 +0000", "text/plain");
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("imap");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    let mut remote = Paged {
        pages: vec![page(vec![member("1", &[], &body)], None)],
        ..Default::default()
    };
    store
        .sync("INBOX", PimdirSyncOptions::default(), &mut remote)
        .unwrap();
    drop(store);

    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys = OFF;
         DROP TRIGGER sources_stamp_coverage;
         ALTER TABLE bindings DROP COLUMN round;
         CREATE TABLE old_sources (
             collection TEXT NOT NULL REFERENCES collections(id) ON UPDATE CASCADE ON DELETE CASCADE,
             source TEXT NOT NULL, checkpoint BLOB, PRIMARY KEY (collection, source)) STRICT;
         INSERT INTO old_sources SELECT collection, source, checkpoint FROM sources;
         DROP TABLE sources;
         ALTER TABLE old_sources RENAME TO sources;
         CREATE TABLE probes (collection TEXT NOT NULL, source TEXT NOT NULL,
             handle TEXT NOT NULL, flags TEXT, PRIMARY KEY (collection, source, handle)) STRICT;
         INSERT INTO probes VALUES ('INBOX', 'imap', '2', NULL);
         UPDATE items SET level = 0;",
    )
    .unwrap();
    drop(conn);

    let reader = PimdirReader::open(dir.path()).unwrap();
    let collections = reader.list_collections().unwrap();
    assert_eq!(collections[0].coverage, None, "read as NULL, unreconciled");
    assert!(reader.list_coverage("INBOX").unwrap().is_empty());
    drop(reader);

    let store = PimdirStore::open(dir.path()).unwrap().for_source("imap");
    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    let probes: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name = 'probes'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(probes, 0, "the probes table is dropped");
    let trigger: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name = 'sources_stamp_coverage'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(trigger, 1);

    let loaded = store
        .load(
            &PimdirCollectionId::from("INBOX"),
            &io_pimdir::load::PimdirLoadScope::All,
        )
        .unwrap();
    assert_eq!(loaded.coverage, None, "a store from an earlier draft");
    assert_eq!(loaded.checkpoint, Some(PimdirCheckpoint(b"c1".to_vec())));
    assert_eq!(loaded.placements[0].level, PimdirLevel::Meta);
    assert_eq!(
        loaded.placements[0].summary, None,
        "a level 0 claim the next Meta upgrade revisits"
    );
}
