//! Retention, eviction and release rules ported from pimdir's retired
//! checks/invariants.sh (STORAGE §11): what a delete keeps in the trash,
//! what the owner's eviction and release leave in place.

#![cfg(feature = "client")]

mod invariants_common;

use io_pimdir::{
    change::PimdirWriteOp,
    hub::PimdirSourceId,
    object::PimdirHash,
    placement::{PimdirLinkId, PimdirStatus},
    reference::{PimdirEndpoint, PimdirReferenceOrigin, PimdirReferenceRole},
};

use invariants_common::*;

const AUG: &str = "Sat, 1 Aug 2026 10:00:00 +0000";
const OCT: &str = "Mon, 5 Oct 2026 10:00:00 +0000";
const SEP: &str = "2026-09-01T00:00:00Z";

fn seq(store: &io_pimdir::client::PimdirSourceStore, collection: &str, link: &str) -> i64 {
    store.seq_for_link(collection, link).unwrap().unwrap()
}

/// L389: a deleted row a source still binds is not purged.
#[test]
fn l389_a_still_bound_tombstone_is_not_purged() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    let a = message("a@x", "A", AUG, Some("alice@x"));
    store
        .write(vec![
            body(&store, &a),
            PimdirWriteOp::UpsertPlacement(held(&store, "INBOX", "1", &a)),
        ])
        .unwrap();
    let seq = seq(&store, "INBOX", "a@x");

    let mut tombstone = held(&store, "INBOX", "1", &a);
    tombstone.status = PimdirStatus::Tombstone;
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(tombstone)])
        .unwrap();
    let rows = |dir: &std::path::Path| {
        int(
            &peek(dir),
            &format!("SELECT count(*) FROM items WHERE collection = 'INBOX' AND seq = {seq}"),
        )
    };
    assert_eq!(
        int(
            &peek(dir.path()),
            "SELECT count(*) FROM items WHERE deleted = 1 AND retained_at IS NULL"
        ),
        1,
        "the row is a tombstone its source still binds"
    );

    assert!(!store.purge("INBOX", seq).unwrap(), "nothing purged");
    assert_eq!(rows(dir.path()), 1, "the held tombstone stays");
}

/// L408: a holder with another body does not count as held elsewhere.
#[test]
fn l408_a_holder_with_another_body_is_no_move() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    let mine = message("a@x", "Mine", AUG, Some("alice@x"));
    let other = message("a@x", "Other", AUG, Some("alice@x"));
    let moved = message("b@x", "Moved", AUG, Some("alice@x"));
    store
        .write(vec![
            body(&store, &mine),
            body(&store, &moved),
            PimdirWriteOp::UpsertPlacement(held(&store, "INBOX", "1", &mine)),
            PimdirWriteOp::UpsertPlacement(held(&store, "INBOX", "2", &moved)),
        ])
        .unwrap();
    store
        .write(vec![
            body(&store, &other),
            PimdirWriteOp::UpsertPlacement(held(&store, "Archive", "9", &other)),
            PimdirWriteOp::UpsertPlacement(held(&store, "Archive", "8", &moved)),
        ])
        .unwrap();

    store
        .write(vec![deleted("INBOX", "1"), deleted("INBOX", "2")])
        .unwrap();

    let retained = store.list_retained("INBOX", None, 10).unwrap();
    let links: Vec<&str> = retained.iter().map(|item| item.link_id.as_str()).collect();
    assert_eq!(
        links,
        ["a@x"],
        "another body elsewhere keeps the row retained, the same body purges it"
    );
}

/// L411: a bodiless holder does not take the only body of the retiring row.
#[test]
fn l411_a_bodiless_holder_does_not_take_the_only_body() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    let a = message("a@x", "A", AUG, Some("alice@x"));
    let b = message("b@x", "B", AUG, Some("alice@x"));
    store
        .write(vec![
            body(&store, &a),
            body(&store, &b),
            PimdirWriteOp::UpsertPlacement(held(&store, "INBOX", "1", &a)),
            PimdirWriteOp::UpsertPlacement(held(&store, "INBOX", "2", &b)),
        ])
        .unwrap();
    store
        .write(vec![
            PimdirWriteOp::UpsertPlacement(meta("Archive", "9", &a)),
            PimdirWriteOp::UpsertPlacement(held(&store, "Archive", "8", &b)),
        ])
        .unwrap();

    store
        .write(vec![deleted("INBOX", "1"), deleted("INBOX", "2")])
        .unwrap();

    let retained = store.list_retained("INBOX", None, 10).unwrap();
    let links: Vec<&str> = retained.iter().map(|item| item.link_id.as_str()).collect();
    assert_eq!(links, ["a@x"], "the only body stays in the trash");
    let item = &retained[0];
    assert_eq!(item.object, Some(store.hash(&a)), "with its body");
}

/// L436: a holder stating no sender beside a stated one is another message.
#[test]
fn l436_a_holder_stating_no_sender_is_another_message() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    let stated = message("a@x", "Notice", AUG, Some("cron@x"));
    let unstated = message("a@x", "Notice", AUG, None);
    let same = message("b@x", "Notice", AUG, Some("cron@x"));
    store
        .write(vec![
            PimdirWriteOp::UpsertPlacement(meta("INBOX", "1", &stated)),
            PimdirWriteOp::UpsertPlacement(meta("INBOX", "2", &same)),
        ])
        .unwrap();
    store
        .write(vec![
            PimdirWriteOp::UpsertPlacement(meta("Archive", "9", &unstated)),
            PimdirWriteOp::UpsertPlacement(meta("Archive", "8", &same)),
        ])
        .unwrap();

    store
        .write(vec![deleted("INBOX", "1"), deleted("INBOX", "2")])
        .unwrap();

    let retained = store.list_retained("INBOX", None, 10).unwrap();
    let links: Vec<&str> = retained.iter().map(|item| item.link_id.as_str()).collect();
    assert_eq!(
        links,
        ["a@x"],
        "a NULL sender does not agree with a stated one, an agreeing holder does"
    );
}

/// L439: an unknown date and sender agree with unknown ones.
#[test]
fn l439_an_unknown_date_and_sender_match_unknown_ones() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    let undated = message("a@x", "Notice", "someday", None);
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(meta(
            "INBOX", "1", &undated,
        ))])
        .unwrap();
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(meta(
            "Archive", "9", &undated,
        ))])
        .unwrap();

    store.write(vec![deleted("INBOX", "1")]).unwrap();

    assert_eq!(store.count_retained("INBOX").unwrap(), 0, "a move");
}

/// L446: a tombstone holds nothing: the retiring row stays retained.
#[test]
fn l446_a_tombstone_holds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    let a = message("a@x", "A", AUG, Some("alice@x"));
    let b = message("b@x", "B", AUG, Some("alice@x"));
    store
        .write(vec![
            body(&store, &a),
            body(&store, &b),
            PimdirWriteOp::UpsertPlacement(held(&store, "INBOX", "1", &a)),
            PimdirWriteOp::UpsertPlacement(held(&store, "INBOX", "2", &b)),
        ])
        .unwrap();
    let mut tombstone = held(&store, "Archive", "9", &a);
    tombstone.status = PimdirStatus::Tombstone;
    store
        .write(vec![
            PimdirWriteOp::UpsertPlacement(tombstone),
            PimdirWriteOp::UpsertPlacement(held(&store, "Archive", "8", &b)),
        ])
        .unwrap();

    store
        .write(vec![deleted("INBOX", "1"), deleted("INBOX", "2")])
        .unwrap();

    let retained = store.list_retained("INBOX", None, 10).unwrap();
    let links: Vec<&str> = retained.iter().map(|item| item.link_id.as_str()).collect();
    assert_eq!(links, ["a@x"], "a tombstone elsewhere is no holder");
}

/// L1148: an evicted body is released to the collector, the holder of the
/// same key in another collection untouched.
#[test]
fn l1148_an_eviction_releases_the_body_and_spares_the_holder_elsewhere() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    let old = message("old@x", "Old", AUG, Some("alice@x"));
    store
        .write(vec![
            body(&store, &old),
            PimdirWriteOp::UpsertPlacement(held(&store, "INBOX", "1", &old)),
        ])
        .unwrap();
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(meta(
            "Archive", "1", &old,
        ))])
        .unwrap();
    let hash = store.hash(&old);
    assert!(refcount(dir.path(), &hash).unwrap() > 0);

    let report = store.evict_before("INBOX", SEP).unwrap();
    assert_eq!(report.seqs.len(), 1);
    assert_eq!(refcount(dir.path(), &hash), Some(0), "released");
    assert!(store.refcount_drift().unwrap().is_empty());
    assert_eq!(store.count_items("Archive").unwrap(), 1, "untouched");
    assert_eq!(store.count_items("INBOX").unwrap(), 0);
    assert_eq!(store.collect_garbage().unwrap().objects, 1);
}

/// L1162: a mail a person's reference names, from either end, is not
/// evicted; a rule's reference keeps nothing.
#[test]
fn l1162_a_user_reference_keeps_a_mail_from_eviction() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    let mails: Vec<Vec<u8>> = ["kept", "target", "told"]
        .iter()
        .map(|name| message(&format!("{name}@x"), name, AUG, Some("alice@x")))
        .collect();
    let other = message("other@x", "Other", OCT, Some("alice@x"));
    let mut ops: Vec<PimdirWriteOp> = mails
        .iter()
        .enumerate()
        .map(|(n, bytes)| {
            PimdirWriteOp::UpsertPlacement(meta("INBOX", &format!("{}", n + 1), bytes))
        })
        .collect();
    ops.push(PimdirWriteOp::UpsertPlacement(meta("INBOX", "9", &other)));
    store.write(ops).unwrap();

    let end = |link: &str| PimdirEndpoint {
        kind: MAIL.into(),
        link_id: PimdirLinkId(link.into()),
    };
    let related = PimdirReferenceRole::Related;
    let user = PimdirReferenceOrigin::User;
    store
        .add_reference(&end("kept@x"), &end("other@x"), &related, user)
        .unwrap();
    store
        .add_reference(&end("other@x"), &end("target@x"), &related, user)
        .unwrap();
    store
        .add_reference(
            &end("told@x"),
            &end("other@x"),
            &PimdirReferenceRole::Sender,
            PimdirReferenceOrigin::Auto,
        )
        .unwrap();
    let told = seq(&store, "INBOX", "told@x");

    let report = store.evict_before("INBOX", SEP).unwrap();
    assert_eq!(report.seqs, [told], "only the mail no person names goes");
    assert!(store.seq_for_link("INBOX", "kept@x").unwrap().is_some());
    assert!(store.seq_for_link("INBOX", "target@x").unwrap().is_some());
}

/// L1168: a mail no source binds is not evicted, no relisting restoring it.
#[test]
fn l1168_an_unbound_mail_is_not_evicted() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    let local = message("local@x", "Local", AUG, Some("alice@x"));
    let bound = message("bound@x", "Bound", AUG, Some("alice@x"));
    store
        .write(vec![
            body(&store, &local),
            body(&store, &bound),
            PimdirWriteOp::UpsertPlacement(held(&store, "INBOX", "1", &local)),
            PimdirWriteOp::UpsertPlacement(held(&store, "INBOX", "2", &bound)),
        ])
        .unwrap();
    let removed = peek(dir.path())
        .execute(
            "DELETE FROM bindings WHERE item = \
             (SELECT id FROM items WHERE link_id = 'local@x')",
            [],
        )
        .unwrap();
    assert_eq!(removed, 1);
    let bound_seq = seq(&store, "INBOX", "bound@x");

    let report = store.evict_before("INBOX", SEP).unwrap();
    assert_eq!(report.seqs, [bound_seq], "the bound twin goes");
    assert!(
        store.seq_for_link("INBOX", "local@x").unwrap().is_some(),
        "the unbound mail stays"
    );
}

struct Released {
    dir: tempfile::TempDir,
    store: io_pimdir::client::PimdirSourceStore,
}

fn hash_of(store: &io_pimdir::client::PimdirSourceStore, name: &str) -> PimdirHash {
    store.hash(&message(&format!("{name}@x"), name, AUG, Some("alice@x")))
}

/// An INBOX of five bodied mails: `old` owes nothing, `conflicted` holds a
/// diverging body, `edited` a body its base lacks, `created` is a pending
/// create, `recent` is newer than the cutoff.
fn released() -> Released {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "imap");
    let bytes = |name: &str, date: &str| message(&format!("{name}@x"), name, date, Some("alice@x"));
    let old = bytes("old", AUG);
    let conflicted = bytes("conflicted", AUG);
    let theirs = bytes("theirs", AUG);
    let edited = bytes("edited", AUG);
    let before = bytes("before", AUG);
    let created = bytes("created", AUG);
    let recent = bytes("recent", OCT);
    let mut ops: Vec<PimdirWriteOp> = [
        &old,
        &conflicted,
        &theirs,
        &edited,
        &before,
        &created,
        &recent,
    ]
    .iter()
    .map(|bytes| body(&store, bytes))
    .collect();

    ops.push(PimdirWriteOp::UpsertPlacement(held(
        &store, "INBOX", "1", &old,
    )));
    let mut placement = held(&store, "INBOX", "2", &conflicted);
    placement.status = PimdirStatus::Conflict;
    placement.conflict_revision = Some("r-theirs".into());
    placement.conflict_object = Some(store.hash(&theirs));
    ops.push(PimdirWriteOp::UpsertPlacement(placement));
    let mut placement = held(&store, "INBOX", "3", &edited);
    placement.status = PimdirStatus::Dirty;
    placement.base.as_mut().unwrap().object = Some(store.hash(&before));
    ops.push(PimdirWriteOp::UpsertPlacement(placement));
    let mut placement = held(&store, "INBOX", "\u{1}created", &created);
    placement.status = PimdirStatus::Created;
    placement.base = None;
    ops.push(PimdirWriteOp::UpsertPlacement(placement));
    ops.push(PimdirWriteOp::UpsertPlacement(held(
        &store, "INBOX", "5", &recent,
    )));
    store.write(ops).unwrap();
    let rebased = peek(dir.path())
        .execute(
            "UPDATE bindings SET base_object = \
             (SELECT id FROM objects WHERE hash = ?1) \
             WHERE item = (SELECT id FROM items WHERE link_id = 'edited@x')",
            [&store.hash(&before).0],
        )
        .unwrap();
    assert_eq!(rebased, 1, "a base the local edit left behind");
    store.recompute_refcounts().unwrap();
    Released { dir, store }
}

fn binding(
    store: &io_pimdir::client::PimdirSourceStore,
    link: &str,
) -> io_pimdir::hub::PimdirBinding {
    store
        .item_bindings("INBOX", link)
        .unwrap()
        .into_iter()
        .find(|(id, _)| id == &PimdirSourceId("imap".into()))
        .map(|(_, binding)| binding)
        .unwrap()
}

/// L1224: a release keeps the body of a conflicted, a locally edited, a
/// pending-create and a newer item.
#[test]
fn l1224_a_release_keeps_the_bodies_still_needed() {
    let Released {
        dir: _dir,
        mut store,
    } = released();
    let old = seq(&store, "INBOX", "old@x");

    let report = store.release_before(&["INBOX"], Some(SEP)).unwrap();
    assert_eq!(report.seqs, [old], "only the body nothing needs");

    for link in ["conflicted@x", "edited@x", "created@x", "recent@x"] {
        let item = store
            .get_item("INBOX", seq(&store, "INBOX", link))
            .unwrap()
            .unwrap();
        assert!(item.object.is_some(), "{link} keeps its body");
    }
    let item = store.get_item("INBOX", old).unwrap().unwrap();
    assert_eq!(item.object, None, "old let go of its body");
}

/// L1227: a release keeps the bases of a conflicted and a newer item.
#[test]
fn l1227_a_release_keeps_the_bases_of_what_it_skips() {
    let Released {
        dir: _dir,
        mut store,
    } = released();
    store.release_before(&["INBOX"], Some(SEP)).unwrap();

    for link in ["conflicted@x", "recent@x"] {
        let base = binding(&store, link).base.unwrap();
        assert!(base.object.is_some(), "{link} keeps its base body");
    }
    let base = binding(&store, "old@x").base.expect("still based");
    assert_eq!(base.object, None, "the released base lets go of the body");
}

/// L1232: a release clears the agreement point naming the released body
/// and keeps one naming another.
#[test]
fn l1232_a_release_clears_only_the_released_shared_object() {
    let Released {
        dir: _dir,
        mut store,
    } = released();
    let old = hash_of(&store, "old");
    let edited_before = binding(&store, "edited@x").shared_object;
    assert_eq!(binding(&store, "old@x").shared_object, Some(old));
    assert!(edited_before.is_some());

    store.release_before(&["INBOX"], Some(SEP)).unwrap();

    assert_eq!(binding(&store, "old@x").shared_object, None);
    assert_eq!(binding(&store, "edited@x").shared_object, edited_before);
}

/// L1237: after a release the counts agree, the released body is garbage
/// and the bodies still named keep their pins.
#[test]
fn l1237_released_bodies_fall_to_the_collector_alone() {
    let Released { dir, mut store } = released();
    let old = hash_of(&store, "old");
    let recent = store.hash(&message("recent@x", "recent", OCT, Some("alice@x")));

    store.release_before(&["INBOX"], Some(SEP)).unwrap();

    assert!(store.refcount_drift().unwrap().is_empty());
    assert_eq!(refcount(dir.path(), &old), Some(0));
    assert!(refcount(dir.path(), &recent).unwrap() > 0);
    let collected = store.collect_garbage().unwrap();
    assert_eq!(collected.objects, 1, "the released body alone");
    assert_eq!(refcount(dir.path(), &old), None);
}

/// L1241: a second release over the same set finds nothing.
#[test]
fn l1241_a_second_release_finds_nothing() {
    let Released {
        dir: _dir,
        mut store,
    } = released();
    assert_eq!(
        store
            .release_before(&["INBOX"], Some(SEP))
            .unwrap()
            .seqs
            .len(),
        1
    );
    let again = store.release_before(&["INBOX"], Some(SEP)).unwrap();
    assert!(again.seqs.is_empty());
}

/// L451: a bodiless retiring row of a kind other than mail is held by any
/// live holder of its key in the account.
#[test]
fn l451_a_bodiless_card_is_held_by_any_live_holder() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mail_store(dir.path(), "dav");
    store.ensure_collection("Cards", "text/vcard").unwrap();
    store.ensure_collection("Cards2", "text/vcard").unwrap();
    store
        .write(vec![
            PimdirWriteOp::UpsertPlacement(bare("Cards", "u.vcf", "u")),
            PimdirWriteOp::UpsertPlacement(bare("Cards", "v.vcf", "v")),
        ])
        .unwrap();
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(bare(
            "Cards2", "u.vcf", "u",
        ))])
        .unwrap();

    store
        .write(vec![deleted("Cards", "u.vcf"), deleted("Cards", "v.vcf")])
        .unwrap();

    let retained = store.list_retained("Cards", None, 10).unwrap();
    let links: Vec<&str> = retained.iter().map(|item| item.link_id.as_str()).collect();
    assert_eq!(links, ["v"], "u moved, v held nowhere else");
}
