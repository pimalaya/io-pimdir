//! Helpers shared by the invariants_* tests: the rules pimdir's retired
//! checks/invariants.sh ran against the schema, ported to io-pimdir.

#![allow(dead_code)]

use std::path::Path;

use io_pimdir::{
    change::PimdirWriteOp,
    client::{PimdirSourceStore, PimdirStore},
    collection::PimdirCollectionId,
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
    summary::mail,
};

pub const MAIL: &str = "message/rfc822";

/// A store owned as `source`, with INBOX and Archive declared as mail.
pub fn mail_store(dir: &Path, source: &str) -> PimdirSourceStore {
    let store = PimdirStore::open(dir).unwrap().for_source(source);
    store.ensure_collection("INBOX", MAIL).unwrap();
    store.ensure_collection("Archive", MAIL).unwrap();
    store
}

/// A message under `id`, with a `From` when `from` names one.
pub fn message(id: &str, subject: &str, date: &str, from: Option<&str>) -> Vec<u8> {
    let from = from
        .map(|from| format!("From: {from}\r\n"))
        .unwrap_or_default();
    format!(
        "Message-ID: <{id}>\r\n{from}To: bob@example.org\r\nSubject: {subject}\r\n\
         Date: {date}\r\n\r\nbody of {id}\r\n"
    )
    .into_bytes()
}

/// The write storing `bytes` as a body.
pub fn body(store: &PimdirSourceStore, bytes: &[u8]) -> PimdirWriteOp {
    PimdirWriteOp::StoreObject {
        object: PimdirObject {
            hash: store.hash(bytes),
            size: bytes.len(),
        },
        body: Some(bytes.to_vec()),
    }
}

/// A clean placement of the message `bytes` at `Full`, its base the body.
pub fn held(
    store: &PimdirSourceStore,
    collection: &str,
    handle: &str,
    bytes: &[u8],
) -> PimdirPlacement {
    let derived = mail::derive(bytes);
    let hash = store.hash(bytes);
    PimdirPlacement {
        collection: PimdirCollectionId(collection.into()),
        handle: PimdirHandle(handle.into()),
        link_id: Some(derived.link_id),
        object: Some(hash.clone()),
        level: PimdirLevel::Full,
        summary: derived.summary,
        sort_key: derived.sort_key,
        flags: PimdirFlags::default(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: None,
            object: Some(hash),
        }),
        origin: None,
        server_object_id: None,
    }
}

/// A clean placement of the message `bytes` at `Meta`, holding no body.
pub fn meta(collection: &str, handle: &str, bytes: &[u8]) -> PimdirPlacement {
    let derived = mail::derive_meta(bytes, Some(bytes.len() as u64), None);
    PimdirPlacement {
        collection: PimdirCollectionId(collection.into()),
        handle: PimdirHandle(handle.into()),
        link_id: Some(derived.link_id),
        object: None,
        level: PimdirLevel::Meta,
        summary: derived.summary,
        sort_key: derived.sort_key,
        flags: PimdirFlags::default(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: None,
            object: None,
        }),
        origin: None,
        server_object_id: None,
    }
}

/// A bodiless placement keyed `link` with no summary.
pub fn bare(collection: &str, handle: &str, link: &str) -> PimdirPlacement {
    PimdirPlacement {
        collection: PimdirCollectionId(collection.into()),
        handle: PimdirHandle(handle.into()),
        link_id: Some(PimdirLinkId(link.into())),
        object: None,
        level: PimdirLevel::Meta,
        summary: None,
        sort_key: Default::default(),
        flags: PimdirFlags::default(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: None,
            object: None,
        }),
        origin: None,
        server_object_id: None,
    }
}

/// The write confirming `handle` of `collection` gone.
pub fn deleted(collection: &str, handle: &str) -> PimdirWriteOp {
    PimdirWriteOp::DropPlacement {
        collection: PimdirCollectionId(collection.into()),
        handle: PimdirHandle(handle.into()),
        reason: io_pimdir::change::PimdirDropReason::Deleted,
    }
}

/// A second connection on the store's database, foreign keys on.
pub fn peek(dir: &Path) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open(dir.join("pimdir.db")).unwrap();
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    conn
}

/// One integer a query answers.
pub fn int(conn: &rusqlite::Connection, query: &str) -> i64 {
    conn.query_row(query, [], |row| row.get(0)).unwrap()
}

/// The refcount of the body `hash`, `None` once its row is gone.
pub fn refcount(dir: &Path, hash: &PimdirHash) -> Option<i64> {
    use rusqlite::OptionalExtension;
    peek(dir)
        .query_row(
            "SELECT refcount FROM objects WHERE hash = ?1",
            [&hash.0],
            |row| row.get(0),
        )
        .optional()
        .unwrap()
}

/// Whether a write failed on a constraint, the message holding `what`.
pub fn refused(result: rusqlite::Result<usize>, what: &str) -> bool {
    match result {
        Err(rusqlite::Error::SqliteFailure(err, message)) => {
            err.code == rusqlite::ErrorCode::ConstraintViolation
                && message.is_some_and(|message| message.contains(what))
        }
        _ => false,
    }
}
