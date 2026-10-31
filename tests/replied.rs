//! Which of a set of message ids some live mail replies to (STORAGE §14.1,
//! `list_replied`): a waiting task's source mail answered (pimdir GUIDE.md
//! §17), the person's own replies aside.

#![cfg(feature = "client")]

use io_pimdir::{
    change::PimdirWriteOp,
    client::{PimdirStore, reader::PimdirReader},
    collection::PimdirCollectionId,
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
    summary::{PimdirSummary, mail::PimdirMailSummary},
};

/// A bound mail `id` in `collection`, named without its body, replying to
/// `parent`.
fn reply(collection: &str, handle: &str, id: &str, parent: &str) -> PimdirWriteOp {
    PimdirWriteOp::UpsertPlacement(PimdirPlacement {
        sort_key: Default::default(),
        collection: PimdirCollectionId(collection.into()),
        handle: PimdirHandle(handle.into()),
        link_id: Some(PimdirLinkId(id.into())),
        object: None,
        level: PimdirLevel::Meta,
        summary: Some(PimdirSummary::Mail(PimdirMailSummary {
            message_id: Some(id.into()),
            in_reply_to: vec![parent.into()],
            subject: "Re: the lease".into(),
            ..Default::default()
        })),
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
    })
}

#[test]
fn a_reply_answers_its_parent_and_the_person_s_own_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("imap");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    store.ensure_collection("Sent", "message/rfc822").unwrap();
    store.set_collection_role("Sent", Some("sent")).unwrap();
    store
        .write(vec![
            reply("INBOX", "1", "r1@example.org", "asked@example.org"),
            reply("Sent", "2", "r2@example.org", "mine@example.org"),
        ])
        .unwrap();

    let reader = PimdirReader::open(dir.path()).unwrap();
    let ids = [
        "<asked@example.org>".to_string(),
        " mine@example.org ".to_string(),
        "never@example.org".to_string(),
        "asked@example.org".to_string(),
        String::new(),
    ];
    assert_eq!(reader.replied(&ids).unwrap(), ["asked@example.org"]);
    assert!(reader.replied(&[]).unwrap().is_empty());
}
