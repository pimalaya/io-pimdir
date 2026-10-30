//! An overruling edit (SYNC §7, §9) over an item conflict whose diverging
//! body nobody has read: there is nothing to keep, and settling would
//! choose against a body nobody read, so the write is refused, staging
//! nothing, until the upgrade fetches it. The binding-level case is vector
//! 101.

use io_pimdir::{
    change::PimdirWriteOp,
    client::{PimdirRunError, PimdirStore},
    collection::PimdirCollectionId,
    mutate::PimdirMutation,
    object::PimdirObject,
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
};

const CARD: &[u8] = b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:c1\r\nFN:Jo\r\nEND:VCARD\r\n";
const EDITED: &[u8] = b"BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:c1\r\nFN:Joe\r\nEND:VCARD\r\n";

#[test]
fn an_overruling_edit_over_an_unread_item_conflict_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("b");
    store.ensure_collection("Contacts", "text/vcard").unwrap();
    let hash = store.hash(CARD);
    store
        .write(vec![
            PimdirWriteOp::StoreObject {
                object: PimdirObject {
                    hash: hash.clone(),
                    size: CARD.len(),
                },
                body: Some(CARD.to_vec()),
            },
            PimdirWriteOp::UpsertPlacement(PimdirPlacement {
                sort_key: Default::default(),
                collection: PimdirCollectionId("Contacts".into()),
                handle: PimdirHandle("b.vcf".into()),
                link_id: Some(PimdirLinkId("urn:uuid:c1".into())),
                object: Some(hash.clone()),
                level: PimdirLevel::Full,
                summary: None,
                flags: PimdirFlags::default(),
                status: PimdirStatus::Clean,
                conflict_revision: None,
                conflict_object: None,
                base: Some(PimdirBase {
                    flags: PimdirFlags::default(),
                    revision: Some("r1".into()),
                    object: Some(hash),
                }),
                origin: None,
            }),
        ])
        .unwrap();
    // NOTE: an item conflict recorded before its incoming body was read.
    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute(
        "UPDATE items SET conflicted = 1, conflict_object = NULL",
        [],
    )
    .unwrap();
    drop(conn);

    let edited = store.hash(EDITED);
    let mutation = PimdirMutation::Edit {
        handle: PimdirHandle("b.vcf".into()),
        object: PimdirObject {
            hash: edited.clone(),
            size: EDITED.len(),
        },
        body: EDITED.to_vec(),
        summary: None,
        sort_key: None,
        if_match: None,
        overrule: true,
    };
    let Err(PimdirRunError::Store(err)) = store.mutate("Contacts", mutation) else {
        panic!("an overrule over an unread body was applied");
    };
    assert_eq!(err.code(), "conflict-unread");

    let item = store.get_item("Contacts", 1).unwrap().unwrap();
    assert_ne!(item.object, Some(edited), "nothing staged");
    assert_eq!(store.list_item_conflicts(None).unwrap().len(), 1);
}
