//! Erasing an item (pimdir STORAGE §11.5): a purge removes a row, not its
//! bytes, and an erase runs the purge, the collector, the index refresh
//! with its thread prune, `optimize_text`, then a `TRUNCATE` checkpoint
//! of both files, so the item's key, subject and address, which pimdir.db
//! holds, and its body's words, which index.db holds, are in neither file,
//! their write-ahead logs included, while the handles that wrote them are
//! still open.

#![cfg(all(feature = "client", feature = "search"))]

use std::{fs, path::Path};

use io_pimdir::{
    change::{PimdirDropReason, PimdirWriteOp},
    client::{
        PimdirSourceStore, PimdirStore,
        search::{PimdirIndex, PimdirIndexRefreshOptions},
    },
    collection::PimdirCollectionId,
    object::PimdirObject,
    placement::{PimdirHandle, PimdirLevel, PimdirPlacement, PimdirSortKey, PimdirStatus},
    summary,
};

/// The words the erased mail alone holds: its key, its subject, its
/// sender's address and its body.
const ERASED: [&str; 4] = [
    "quokka-key",
    "Zanzibarian",
    "frangipani.sender",
    "marzipanorama",
];

/// A message, `id` naming its key, filed into INBOX as source `local`.
fn file(store: &mut PimdirSourceStore, id: &str, subject: &str, from: &str, text: &str) {
    let body = format!(
        "Message-ID: <{id}@example.org>\r\nSubject: {subject}\r\nFrom: Ann <{from}@example.org>\r\n\
         Date: Sat, 01 Aug 2026 10:00:00 +0000\r\n\r\n{text}\r\n"
    )
    .into_bytes();
    let derived = summary::derive("message/rfc822", &body).unwrap();
    let hash = store.hash(&body);
    store
        .write(vec![
            PimdirWriteOp::StoreObject {
                object: PimdirObject {
                    hash: hash.clone(),
                    size: body.len(),
                },
                body: Some(body),
            },
            PimdirWriteOp::UpsertPlacement(PimdirPlacement {
                sort_key: PimdirSortKey(derived.sort_key.as_str().into()),
                collection: PimdirCollectionId("INBOX".into()),
                handle: PimdirHandle(id.into()),
                link_id: Some(derived.link_id),
                level: PimdirLevel::Full,
                object: Some(hash),
                summary: derived.summary,
                flags: Default::default(),
                status: PimdirStatus::Clean,
                conflict_revision: None,
                conflict_object: None,
                base: None,
                origin: None,
            }),
        ])
        .unwrap();
}

/// How many times `word` stands in the bytes of the database `name` under
/// `dir` and of its write-ahead log.
fn count(dir: &Path, name: &str, word: &str) -> usize {
    let mut found = 0;
    for file in [name.to_string(), format!("{name}-wal")] {
        let bytes = fs::read(dir.join(file)).unwrap_or_default();
        found += bytes
            .windows(word.len())
            .filter(|window| window.eq_ignore_ascii_case(word.as_bytes()))
            .count();
    }
    found
}

/// A retained mail erased leaves no byte of its key, subject, address or
/// body words in either file, while a kept mail stays found.
#[test]
fn an_erased_mail_leaves_both_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    file(&mut store, "kept", "Kept", "kept.sender", "ordinary words");
    file(
        &mut store,
        ERASED[0],
        ERASED[1],
        ERASED[2],
        &format!("{} and more", ERASED[3]),
    );
    let mut index = PimdirIndex::open(dir.path()).unwrap();
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();

    for word in &ERASED[..3] {
        assert!(
            count(dir.path(), "pimdir.db", word) > 0,
            "{word} in the store"
        );
    }
    assert!(count(dir.path(), "index.db", ERASED[3]) > 0, "indexed");

    let seq = store
        .list_items("INBOX", None, 10)
        .unwrap()
        .into_iter()
        .find(|item| item.link_id.0.contains(ERASED[0]))
        .unwrap()
        .seq;
    store
        .write(vec![PimdirWriteOp::DropPlacement {
            collection: PimdirCollectionId("INBOX".into()),
            handle: PimdirHandle(ERASED[0].into()),
            reason: PimdirDropReason::Deleted,
        }])
        .unwrap();

    let erased = store.erase("INBOX", seq).unwrap();
    assert!(erased.erased);
    assert_eq!(erased.collected.blobs, 1, "its body unlinked");
    index.erase().unwrap();

    for name in ["pimdir.db", "index.db"] {
        for word in ERASED {
            assert_eq!(count(dir.path(), name, word), 0, "{word} left in {name}");
        }
        assert_eq!(
            fs::metadata(dir.path().join(format!("{name}-wal")))
                .map(|meta| meta.len())
                .unwrap_or_default(),
            0,
            "{name}'s WAL truncated"
        );
    }
    assert!(count(dir.path(), "pimdir.db", "kept.sender") > 0);
    assert!(count(dir.path(), "index.db", "ordinary") > 0);
}

/// A live item a source binds is its source's to remove: an erase purges
/// nothing, and the item stays.
#[test]
fn a_bound_live_item_is_not_erased() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("local");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    file(&mut store, "live", "Live", "live.sender", "words");
    let seq = store.list_items("INBOX", None, 10).unwrap()[0].seq;

    let erased = store.erase("INBOX", seq).unwrap();
    assert!(!erased.erased);
    assert_eq!(store.count_items("INBOX").unwrap(), 1);
}
