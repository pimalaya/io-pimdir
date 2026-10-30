//! Stand-ins recorded as a message's body enters the store (STORAGE
//! §14.3): one file per part carrying `Content-Disposition: attachment`,
//! at `Meta` in the account's attachments collection, referenced from
//! the message, in the transaction storing the body, whatever path
//! stores it, once.

use std::path::Path;

use io_pimdir::{
    change::PimdirWriteOp,
    client::{PimdirSourceStore, PimdirStore, producer::PimdirProducer},
    codec::PimdirAction,
    collection::PimdirCollectionId,
    hash::PimdirHashAlgo,
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
    reference::{PimdirEndpoint, PimdirReferenceOrigin, PimdirReferenceRole},
    remote::PimdirTier,
    summary::{
        PimdirSummary,
        file::{self, PimdirFileSummary},
        mail,
    },
};

const MAIL: &str = "message/rfc822";
const ACCOUNT: &str = "work";
const ATTACHMENTS: &str = "work/\u{1}attachments";

/// A message with a text body, an inline image, a PDF and a text file
/// named under RFC 2231, nested one level.
const BODY: &[u8] = b"Message-ID: <invoice-7@example.org>\r\n\
From: Billing <billing@example.org>\r\n\
To: bob@example.org\r\n\
Subject: Invoice\r\n\
Date: Mon, 05 Oct 2026 09:00:00 +0000\r\n\
Content-Type: multipart/mixed; boundary=\"outer\"\r\n\
\r\n\
--outer\r\n\
Content-Type: multipart/related; boundary=\"inner\"\r\n\
\r\n\
--inner\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>Attached.</p><img src=\"cid:logo\">\r\n\
--inner\r\n\
Content-Type: image/png\r\n\
Content-Disposition: inline; filename=\"logo.png\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
iVBORw0KGgo=\r\n\
--inner--\r\n\
--outer\r\n\
Content-Type: application/pdf; name=\"invoice.pdf\"\r\n\
Content-Disposition: attachment; filename=\"invoice.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
JVBERi0xLjQK\r\n\
--outer\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
Content-Disposition: attachment; filename*=UTF-8''d%C3%A9tail.txt\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\
\r\n\
total=3A 42=E2=82=AC\r\n\
--outer--\r\n";

const LINK: &str = "invoice-7@example.org";

fn hash_of(body: &[u8]) -> PimdirHash {
    PimdirHashAlgo::default().hash(body)
}

fn store_object(body: &[u8]) -> PimdirWriteOp {
    PimdirWriteOp::StoreObject {
        object: PimdirObject {
            hash: hash_of(body),
            size: body.len(),
        },
        body: Some(body.to_vec()),
    }
}

/// A clean placement of `link` under `handle` holding `body`, its
/// summary derived from it.
fn placement(collection: &str, handle: &str, link: &str, body: Option<&[u8]>) -> PimdirPlacement {
    let object = body.map(hash_of);
    let summary = mail::derive(body.unwrap_or(BODY)).summary;
    PimdirPlacement {
        sort_key: Default::default(),
        collection: PimdirCollectionId(collection.into()),
        handle: PimdirHandle(handle.into()),
        link_id: Some(PimdirLinkId(link.into())),
        object: object.clone(),
        level: if object.is_some() {
            PimdirLevel::Full
        } else {
            PimdirLevel::Meta
        },
        summary,
        flags: PimdirFlags::default(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: None,
            object,
        }),
        origin: None,
    }
}

fn opened(dir: &Path) -> PimdirSourceStore {
    let store = PimdirStore::open(dir)
        .unwrap()
        .for_account(ACCOUNT)
        .for_source("imap");
    store.ensure_collection("INBOX", MAIL).unwrap();
    store.ensure_collection("Archive", MAIL).unwrap();
    store
}

fn stored(store: &mut PimdirSourceStore, collection: &str, handle: &str, link: &str, body: &[u8]) {
    store
        .write(vec![
            store_object(body),
            PimdirWriteOp::UpsertPlacement(placement(collection, handle, link, Some(body))),
        ])
        .unwrap();
}

fn expected() -> Vec<PimdirFileSummary> {
    vec![
        PimdirFileSummary {
            name: "invoice.pdf".into(),
            media_type: Some("application/pdf".into()),
            size: Some(9),
            part: Some("2".into()),
        },
        PimdirFileSummary {
            name: "détail.txt".into(),
            media_type: Some("text/plain".into()),
            size: Some(12),
            part: Some("3".into()),
        },
    ]
}

/// The stand-ins of `link`: their keys, seqs and summaries, in order.
fn stand_ins(store: &PimdirSourceStore, link: &str) -> Vec<(String, i64, PimdirFileSummary)> {
    store
        .list_attachments(Some(ACCOUNT), &PimdirLinkId(link.into()))
        .unwrap()
        .into_iter()
        .map(|file| {
            assert_eq!(file.collection, ATTACHMENTS);
            assert_eq!(file.object, None, "a stand-in holds no body");
            (file.link_id.0, file.seq, file.summary.unwrap())
        })
        .collect()
}

/// Every row of the attachments collection, with its summary.
fn collection_rows(store: &PimdirSourceStore) -> Vec<String> {
    let mut rows: Vec<String> = store
        .list_items(ATTACHMENTS, None, 100)
        .unwrap()
        .into_iter()
        .map(|item| {
            format!(
                "{}|{}|{:?}|{:?}",
                item.link_id.0, item.seq, item.object, item.level
            )
        })
        .collect();
    rows.sort();
    rows
}

#[test]
fn a_stored_body_stands_for_its_attachments() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());

    stored(&mut store, "INBOX", "1", LINK, BODY);

    let found = stand_ins(&store, LINK);
    assert_eq!(
        found
            .iter()
            .map(|(key, _, _)| key.as_str())
            .collect::<Vec<_>>(),
        [
            "part:invoice-7@example.org#2",
            "part:invoice-7@example.org#3"
        ]
    );
    assert_eq!(
        found
            .into_iter()
            .map(|(_, _, summary)| summary)
            .collect::<Vec<_>>(),
        expected()
    );
    assert_eq!(mail::attachments::attachments(BODY), expected());

    // NOTE: the collection is the account's, found again by its role.
    let collection = store
        .list_collections_by_account(Some(ACCOUNT))
        .unwrap()
        .into_iter()
        .find(|collection| collection.id == ATTACHMENTS)
        .unwrap();
    assert_eq!(collection.kind, file::KIND);
    assert_eq!(collection.name, "attachments");
    assert_eq!(collection.role.as_deref(), Some("attachments"));

    let items = store.list_items(ATTACHMENTS, None, 10).unwrap();
    assert!(
        items
            .iter()
            .all(|item| item.level == PimdirLevel::Meta && item.object.is_none())
    );

    let references = store
        .references_from(&PimdirEndpoint {
            kind: MAIL.into(),
            link_id: PimdirLinkId(LINK.into()),
        })
        .unwrap();
    let attached: Vec<_> = references
        .iter()
        .filter(|reference| reference.role == PimdirReferenceRole::Attachment)
        .map(|reference| {
            assert_eq!(reference.origin, PimdirReferenceOrigin::Auto);
            assert_eq!(reference.to.kind, file::KIND);
            reference.to.link_id.0.as_str()
        })
        .collect();
    assert_eq!(
        attached,
        [
            "part:invoice-7@example.org#2",
            "part:invoice-7@example.org#3"
        ]
    );

    let listed = store
        .list_attachments_by_account(Some(ACCOUNT), None, 10)
        .unwrap();
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().all(|file| file.message.0 == LINK));
    let searched = store
        .search_files(&[ATTACHMENTS], "%invoice%", None, 10)
        .unwrap();
    assert_eq!(searched.len(), 1);
}

#[test]
fn a_body_entering_again_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());
    stored(&mut store, "INBOX", "1", LINK, BODY);
    let before = collection_rows(&store);
    let files = stand_ins(&store, LINK);
    let message = PimdirEndpoint {
        kind: MAIL.into(),
        link_id: PimdirLinkId(LINK.into()),
    };
    let references = store.references_from(&message).unwrap();

    // NOTE: the same body restated, then filed under a second label.
    stored(&mut store, "INBOX", "1", LINK, BODY);
    stored(&mut store, "Archive", "9", LINK, BODY);
    assert_eq!(collection_rows(&store), before);

    // NOTE: released, then fetched again.
    store.release_before(&["INBOX", "Archive"], None).unwrap();
    assert_eq!(
        store.list_items("INBOX", None, 10).unwrap()[0].object,
        None,
        "the body was released"
    );
    stored(&mut store, "INBOX", "1", LINK, BODY);
    assert_eq!(collection_rows(&store), before);
    assert_eq!(stand_ins(&store, LINK), files);
    assert_eq!(store.references_from(&message).unwrap(), references);
}

#[test]
fn only_a_body_entering_is_walked_and_nothing_is_backfilled() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());

    // NOTE: named at Meta first: no body, no stand-in.
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(placement(
            "INBOX", "1", LINK, None,
        ))])
        .unwrap();
    assert!(stand_ins(&store, LINK).is_empty());

    // NOTE: the body arrives: that is the instant they are recorded.
    stored(&mut store, "INBOX", "1", LINK, BODY);
    assert_eq!(stand_ins(&store, LINK).len(), 2);

    // NOTE: a person unlinks both, which collects them; the body, held
    // already, filed again under another label brings nothing back.
    let message = PimdirEndpoint {
        kind: MAIL.into(),
        link_id: PimdirLinkId(LINK.into()),
    };
    for (key, _, _) in stand_ins(&store, LINK) {
        let file = PimdirEndpoint {
            kind: file::KIND.into(),
            link_id: PimdirLinkId(key),
        };
        store
            .remove_reference(&message, &file, &PimdirReferenceRole::Attachment)
            .unwrap()
            .unwrap();
    }
    assert!(stand_ins(&store, LINK).is_empty());
    assert!(collection_rows(&store).is_empty());
    stored(&mut store, "Archive", "9", LINK, BODY);
    assert!(stand_ins(&store, LINK).is_empty());
}

#[test]
fn no_stand_in_for_a_derived_key_inline_parts_or_another_kind() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());
    store.ensure_collection("Cards", "text/vcard").unwrap();

    // NOTE: the same parts, no Message-ID: keyed alt:, naming no identity.
    let anonymous: Vec<u8> = BODY
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.starts_with(b"Message-ID:"))
        .collect::<Vec<_>>()
        .join(&b'\n');
    let link = mail::derive(&anonymous).link_id;
    assert!(link.is_derived());
    stored(&mut store, "INBOX", "2", link.as_str(), &anonymous);
    assert!(stand_ins(&store, link.as_str()).is_empty());

    // NOTE: inline parts and a body with none disposed as attachments.
    let inline = b"Message-ID: <inline@example.org>\r\n\
Content-Type: multipart/related; boundary=b\r\n\r\n\
--b\r\nContent-Type: text/html\r\n\r\n<img src=\"cid:x\">\r\n\
--b\r\nContent-Type: image/png\r\nContent-Disposition: inline; filename=x.png\r\n\r\nPNG\r\n\
--b--\r\n";
    stored(&mut store, "INBOX", "3", "inline@example.org", inline);
    assert!(stand_ins(&store, "inline@example.org").is_empty());

    // NOTE: a body of another kind is never walked as a message.
    store
        .write(vec![
            store_object(BODY),
            PimdirWriteOp::UpsertPlacement(placement("Cards", "c", "card-1", Some(BODY))),
        ])
        .unwrap();

    assert!(
        store
            .list_collections_by_account(Some(ACCOUNT))
            .unwrap()
            .iter()
            .all(|collection| collection.id != ATTACHMENTS),
        "nothing to stand for, no collection"
    );
}

#[test]
fn a_queued_add_stands_for_its_attachments_when_drained() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());

    let mut producer = PimdirProducer::open(dir.path(), "smtp").unwrap();
    let object = producer.stage_body(BODY).unwrap();
    let hash = object.hash.clone();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::Add {
                link_id: Some(PimdirLinkId(LINK.into())),
                flags: PimdirFlags::from_iter(["\\Draft"]),
                object: Some(hash.clone()),
                supersedes: None,
            },
            Some(&object),
        )
        .unwrap();

    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));
    assert_eq!(
        stand_ins(&store, LINK)
            .into_iter()
            .map(|(_, _, summary)| summary)
            .collect::<Vec<_>>(),
        expected()
    );
}

#[test]
fn a_store_of_one_account_files_them_under_no_account() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("imap");
    store.ensure_collection("INBOX", MAIL).unwrap();

    stored(&mut store, "INBOX", "1", LINK, BODY);

    let files = store
        .list_attachments(None, &PimdirLinkId(LINK.into()))
        .unwrap();
    assert_eq!(files.len(), 2);
    assert!(
        files
            .iter()
            .all(|file| file.collection == "\u{1}attachments")
    );
    let collection = store
        .list_collections_by_account(None)
        .unwrap()
        .into_iter()
        .find(|collection| collection.id == "\u{1}attachments")
        .unwrap();
    assert_eq!(collection.role.as_deref(), Some("attachments"));
}

/// An invitation: a `multipart/mixed` whose calendar part is inline and
/// base64, so the mark read without the body guesses one attachment and
/// no invitation, and the body gives none and a `UID`.
const INVITE: &[u8] = b"Message-ID: <invite-3@example.org>\r\n\
From: Alice <alice@example.org>\r\n\
Subject: Stand-up\r\n\
Date: Mon, 05 Oct 2026 09:00:00 +0000\r\n\
Content-Type: multipart/mixed; boundary=\"b\"\r\n\
\r\n\
--b\r\n\
Content-Type: text/plain\r\n\
\r\n\
Join us.\r\n\
--b\r\n\
Content-Type: text/calendar; method=REQUEST\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
QkVHSU46VkNBTEVOREFSDQpCRUdJTjpWRVZFTlQNClVJRDpzdGFuZC11cA0KIEBleGFtcGxlLm9y\r\n\
Zw0KRU5EOlZFVkVOVA0KRU5EOlZDQUxFTkRBUg0K\r\n\
--b--\r\n";

const INVITE_LINK: &str = "invite-3@example.org";
const UID: &str = "stand-up@example.org";

/// The summary a listing reads of `body` without it, stating `size`.
fn listed(
    collection: &str,
    handle: &str,
    link: &str,
    body: &[u8],
    size: Option<u64>,
) -> PimdirPlacement {
    let mut placement = placement(collection, handle, link, None);
    placement.summary = mail::derive_meta(body, size, None).summary;
    placement
}

/// The size, mark and invitation of the row of `link` in `collection`,
/// as a load reads it.
fn columns(
    store: &mut PimdirSourceStore,
    collection: &str,
    link: &str,
) -> (Option<u64>, Option<bool>, Option<String>) {
    let loaded = store.open_collection(collection).unwrap();
    let placement = loaded
        .placements
        .into_iter()
        .find(|placement| {
            placement
                .link_id
                .as_ref()
                .is_some_and(|key| key.as_str() == link)
        })
        .unwrap();
    match placement.summary {
        Some(PimdirSummary::Mail(mail)) => (mail.size, mail.attachment, mail.invitation),
        other => panic!("a mail row, not {other:?}"),
    }
}

#[test]
fn a_body_read_without_it_states_what_its_parts_give() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());

    // NOTE: the derivation agrees with the walk on both.
    let derived = match mail::derive(INVITE).summary {
        Some(PimdirSummary::Mail(mail)) => mail,
        _ => unreachable!("a message derives a mail summary"),
    };
    assert_eq!(derived.attachment, Some(false));
    assert_eq!(derived.invitation.as_deref(), Some(UID));

    // NOTE: a writer streaming the body states the header block's guess,
    // as neverest's header capture does.
    let mut streamed = listed("INBOX", "1", INVITE_LINK, INVITE, Some(INVITE.len() as u64));
    streamed.object = Some(hash_of(INVITE));
    streamed.level = PimdirLevel::Full;
    streamed.base.as_mut().unwrap().object = Some(hash_of(INVITE));
    assert!(matches!(
        &streamed.summary,
        Some(PimdirSummary::Mail(mail)) if mail.attachment == Some(true) && mail.invitation.is_none()
    ));
    store
        .write(vec![
            store_object(INVITE),
            PimdirWriteOp::UpsertPlacement(streamed),
        ])
        .unwrap();

    assert_eq!(
        columns(&mut store, "INBOX", INVITE_LINK),
        (Some(INVITE.len() as u64), Some(false), Some(UID.into()))
    );
}

#[test]
fn a_copy_of_a_held_body_takes_the_columns_its_body_gave() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());
    stored(&mut store, "INBOX", "1", LINK, BODY);
    stored(&mut store, "INBOX", "2", INVITE_LINK, INVITE);
    let before = collection_rows(&store);

    // NOTE: an IMAP COPY into Archive, the store linking the held body
    // to the listed member without a fetch: its listing guessed no
    // attachment for one, one for the other, and stated no size.
    for (handle, link, body) in [("8", LINK, BODY), ("9", INVITE_LINK, INVITE)] {
        let mut copy = listed("Archive", handle, link, body, None);
        if let Some(PimdirSummary::Mail(mail)) = &mut copy.summary {
            mail.attachment = Some(link == INVITE_LINK);
        }
        copy.object = Some(hash_of(body));
        copy.level = PimdirLevel::Full;
        copy.base.as_mut().unwrap().object = Some(hash_of(body));
        store
            .write(vec![PimdirWriteOp::UpsertPlacement(copy)])
            .unwrap();
    }

    assert_eq!(
        columns(&mut store, "Archive", LINK),
        (Some(BODY.len() as u64), Some(true), None)
    );
    assert_eq!(
        columns(&mut store, "Archive", INVITE_LINK),
        (Some(INVITE.len() as u64), Some(false), Some(UID.into()))
    );
    assert_eq!(collection_rows(&store), before, "a copy enters nothing");
}

#[test]
fn a_copy_of_a_body_held_with_no_mark_takes_the_columns_a_walk_gives() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());
    stored(&mut store, "INBOX", "1", LINK, BODY);
    stored(&mut store, "INBOX", "2", INVITE_LINK, INVITE);

    // NOTE: the rows an older writer left, which read neither column off
    // the body and recorded no stand-in.
    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute_batch(
        "UPDATE mail_summary SET attachment = NULL, invitation = NULL;
         DELETE FROM item_reference WHERE role = 'attachment';",
    )
    .unwrap();
    drop(conn);
    assert!(stand_ins(&store, LINK).is_empty());
    let before = collection_rows(&store);
    let held = |store: &mut PimdirSourceStore| {
        [LINK, INVITE_LINK].map(|link| columns(store, "INBOX", link))
    };
    let old = held(&mut store);
    assert_eq!(
        old,
        [
            (Some(BODY.len() as u64), None, None),
            (Some(INVITE.len() as u64), None, None),
        ]
    );

    // NOTE: the listing guessed the opposite mark for each.
    for (handle, link, body) in [("8", LINK, BODY), ("9", INVITE_LINK, INVITE)] {
        let mut copy = listed("Archive", handle, link, body, None);
        if let Some(PimdirSummary::Mail(mail)) = &mut copy.summary {
            mail.attachment = Some(link == INVITE_LINK);
        }
        copy.object = Some(hash_of(body));
        copy.level = PimdirLevel::Full;
        copy.base.as_mut().unwrap().object = Some(hash_of(body));
        store
            .write(vec![PimdirWriteOp::UpsertPlacement(copy)])
            .unwrap();
    }

    assert_eq!(
        columns(&mut store, "Archive", LINK),
        (Some(BODY.len() as u64), Some(true), None)
    );
    assert_eq!(
        columns(&mut store, "Archive", INVITE_LINK),
        (Some(INVITE.len() as u64), Some(false), Some(UID.into()))
    );
    assert!(stand_ins(&store, LINK).is_empty(), "no stand-in recorded");
    assert_eq!(collection_rows(&store), before);
    assert_eq!(held(&mut store), old, "the held rows are left as they are");
}

/// A remote an upgrade linking from the store never reaches.
struct Unreached;

impl io_pimdir::remote::PimdirRemote for Unreached {
    type Error = std::convert::Infallible;

    fn enumerate(
        &mut self,
        _: &PimdirCollectionId,
        _: io_pimdir::remote::PimdirEnumerate,
    ) -> Result<io_pimdir::remote::PimdirEnumerated, Self::Error> {
        unreachable!("an upgrade never enumerates")
    }

    fn fetch(
        &mut self,
        _: &PimdirCollectionId,
        _: Vec<PimdirHandle>,
        _: PimdirTier,
    ) -> Result<Vec<io_pimdir::remote::PimdirFetchedItem>, Self::Error> {
        unreachable!("a held body is linked, never fetched")
    }

    fn push(
        &mut self,
        _: &PimdirCollectionId,
        _: Vec<io_pimdir::change::PimdirChange>,
    ) -> Result<Vec<io_pimdir::remote::PimdirPushResult>, Self::Error> {
        unreachable!("an upgrade never pushes")
    }
}

#[test]
fn an_upgrade_linking_a_held_body_takes_the_columns_its_body_gave() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());
    stored(&mut store, "INBOX", "1", INVITE_LINK, INVITE);

    // NOTE: the second label, listed at Meta with the server's size.
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(listed(
            "Archive",
            "9",
            INVITE_LINK,
            INVITE,
            Some(INVITE.len() as u64),
        ))])
        .unwrap();
    assert_eq!(
        columns(&mut store, "Archive", INVITE_LINK),
        (Some(INVITE.len() as u64), Some(true), None),
        "the listing's guess"
    );

    let report = store
        .upgrade(
            "Archive",
            vec![PimdirHandle::from("9")],
            PimdirTier::Full,
            &mut Unreached,
        )
        .unwrap();
    assert_eq!((report.upgraded, report.deduped), (1, 1));

    let item = store.list_items("Archive", None, 10).unwrap();
    assert_eq!(item[0].object, Some(hash_of(INVITE)));
    assert_eq!(
        columns(&mut store, "Archive", INVITE_LINK),
        (Some(INVITE.len() as u64), Some(false), Some(UID.into()))
    );
}
