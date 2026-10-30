//! # Sync audit of 2026-10-30
//!
//! The findings of the release audit's sync half (io-pimdir cairn,
//! sync-audit-2026-10-30) a run against the real store reproduces.

use io_pimdir::{
    mutate::PimdirMutation,
    object::PimdirObject,
    placement::{PimdirHandle, PimdirStatus},
    remote::PimdirTier,
    sync::PimdirSyncOptions,
};

use crate::common::{Client, MemRemote, hash};

const BASE: &[u8] = b"the ancestor body";
const LOCAL: &[u8] = b"the local body";
const REMOTE: &[u8] = b"the remote body";

fn edit(client: &mut Client, body: &[u8]) {
    let object = PimdirObject {
        hash: hash(body),
        size: body.len(),
    };
    client
        .mutate(
            "inbox",
            PimdirMutation::Edit {
                overrule: false,
                if_match: None,
                handle: PimdirHandle::from("m1"),
                object,
                body: body.to_vec(),
                summary: None,
                sort_key: None,
            },
        )
        .unwrap();
}

/// A mutable client synced once over `m1` (`l1`, `BASE`), then edited
/// locally to `LOCAL`.
fn edited_client() -> Client {
    let mut remote = MemRemote::default();
    remote.mutable = true;
    remote.seed("inbox", "m1", "l1", &[], BASE);

    let mut client = Client::new(remote);
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    client
        .upgrade("inbox", vec![PimdirHandle::from("m1")], PimdirTier::Full)
        .unwrap();
    edit(&mut client, LOCAL);
    assert_eq!(
        client.storage().placement("inbox", "m1").status,
        PimdirStatus::Dirty
    );
    client
}

/// S7: a handle stating another identity retires the old binding (SYNC
/// §5), and a local edit the remote never saw beats that delete: the edit
/// is re-staged as a pending create, never dropped with the binding.
#[test]
fn a_restated_identity_keeps_the_local_edit_of_the_old_one() {
    let mut client = edited_client();
    client.remote_mut().seed("inbox", "m1", "l2", &[], REMOTE);

    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    let rows = client.storage().rows("inbox");
    assert!(
        rows.iter().any(|p| p.object == Some(hash(REMOTE))),
        "the new identity: {rows:#?}"
    );
    assert!(
        rows.iter().any(|p| p.object == Some(hash(LOCAL))),
        "the local edit survives: {rows:#?}"
    );
}

/// S16: a `Full` fetch answering no body raises nothing and fetched
/// nothing, so the report says neither.
#[test]
fn a_full_fetch_answering_no_body_counts_nothing() {
    let mut remote = MemRemote::default();
    remote.seed("inbox", "m1", "l1", &[], BASE);
    let mut client = Client::new(remote);
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    client.remote_mut().bodiless_full = true;

    let report = client
        .upgrade("inbox", vec![PimdirHandle::from("m1")], PimdirTier::Full)
        .unwrap();
    assert_eq!(client.storage().placement("inbox", "m1").object, None);
    assert_eq!((report.fetched, report.upgraded), (0, 0), "{report:?}");
}

/// S16: a push the connector left unanswered stays pending (SYNC §5), and
/// the report counts it rather than forgetting it.
#[test]
fn an_unanswered_push_is_counted() {
    let mut client = edited_client();
    client.remote_mut().unanswered = true;

    let report = client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    assert_eq!(report.pushed, 0);
    assert_eq!(report.unanswered, 1, "{report:?}");
    assert_eq!(
        client.storage().placement("inbox", "m1").status,
        PimdirStatus::Dirty,
        "still pending"
    );
}

/// S16: SYNC §5's push discipline: a create the remote rejects stays
/// pending, and every run says so, so a create refused run after run is
/// reported rather than pushed for ever in silence.
#[test]
fn a_rejected_create_is_reported_on_every_run() {
    use io_pimdir::placement::PimdirLinkId;

    // NOTE: the item another source holds is a pending create of this one
    // (the hub), which its server refuses.
    let mut first = MemRemote::default();
    first.seed("inbox", "m9", "l9", &[], BASE);
    let mut a = Client::new(first);
    a.sync("inbox", PimdirSyncOptions::default()).unwrap();
    a.upgrade("inbox", vec![PimdirHandle::from("m9")], PimdirTier::Full)
        .unwrap();
    let mut second = MemRemote::default();
    second.refused_appends.insert(PimdirLinkId::from("l9"));
    let mut client = a.sharing(second, "right");

    for run in 0..2 {
        let report = client.sync("inbox", PimdirSyncOptions::default()).unwrap();
        assert_eq!(report.rejected, 1, "run {run}: {report:?}");
        assert_eq!(report.rejected_creates.len(), 1, "run {run}: {report:?}");
    }
}

/// S9: a rekey re-stages an edited member the new listing no longer holds
/// as a pending create (SYNC §8), under the provisional handle its key
/// derives: the old handle is void, and the renumbering may give it to
/// another member.
#[test]
fn a_rekey_restages_an_uncarried_edit_under_a_provisional_handle() {
    let mut remote = MemRemote::default();
    remote.mutable = true;
    remote.seed("inbox", "v1-0", "l1", &[], BASE);
    remote.seed("inbox", "v1-1", "l2", &[], REMOTE);
    let mut client = Client::new(remote);
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    let object = PimdirObject {
        hash: hash(LOCAL),
        size: LOCAL.len(),
    };
    client
        .mutate(
            "inbox",
            PimdirMutation::Edit {
                overrule: false,
                if_match: None,
                handle: PimdirHandle::from("v1-0"),
                object,
                body: LOCAL.to_vec(),
                summary: None,
                sort_key: None,
            },
        )
        .unwrap();

    // NOTE: l1 is gone from the server and the renumbering hands its old
    // handle to l2.
    client.remote_mut().remove("inbox", "v1-0");
    client.remote_mut().renumber("inbox", 1);
    client.rekey("inbox").unwrap();

    let rows = client.storage().rows("inbox");
    let l2 = rows
        .iter()
        .find(|p| p.link_id.as_ref().is_some_and(|l| l.as_str() == "l2"))
        .unwrap_or_else(|| panic!("l2 kept: {rows:#?}"));
    assert_eq!(l2.handle.as_str(), "v1-0", "{rows:#?}");
    let l1 = rows
        .iter()
        .find(|p| p.link_id.as_ref().is_some_and(|l| l.as_str() == "l1"))
        .unwrap_or_else(|| panic!("the edit kept: {rows:#?}"));
    assert_eq!(l1.status, PimdirStatus::Created, "{rows:#?}");
    assert_eq!(l1.object, Some(hash(LOCAL)));
    assert_ne!(l1.handle.as_str(), "v1-0", "{rows:#?}");
}

/// S10: where one hint had several old copies at one revision, a rekey
/// matches a member to the copy holding the body it carried (SYNC §8), so
/// each copy's state stays with its own resource.
#[test]
fn a_rekey_matches_copies_of_one_hint_by_their_body() {
    let mut remote = MemRemote::default();
    remote.mutable = true;
    remote.seed("inbox", "m1", "l1", &[], BASE);
    remote.seed("inbox", "m2", "l1", &[], REMOTE);
    let mut client = Client::new(remote);
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    let before: Vec<_> = client
        .storage()
        .rows("inbox")
        .into_iter()
        .map(|p| (p.handle, p.link_id, p.object))
        .collect();

    client.rekey("inbox").unwrap();
    let after: Vec<_> = client
        .storage()
        .rows("inbox")
        .into_iter()
        .map(|p| (p.handle, p.link_id, p.object))
        .collect();
    assert_eq!(after, before, "each copy kept its own key and body");
}

/// A rekey whose new handle space reuses an old handle a gone member held
/// (a server renumbering from UID 1 after a UIDVALIDITY change) drops that
/// member `Deleted` before any upsert reuses its handle (SYNC §8), rather
/// than leaving it neither carried nor dropped.
#[test]
fn a_rekey_reusing_a_gone_members_handle_drops_it() {
    let mut remote = MemRemote::default();
    for n in 1..=3 {
        remote.seed("inbox", &format!("m{n}"), &format!("l{n}"), &[], BASE);
    }
    let mut client = Client::new(remote);
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();

    // NOTE: l1 and l2 are gone, l3 renumbered to m2, and m1 now names a
    // message the store never held.
    for n in 1..=3 {
        client.remote_mut().remove("inbox", &format!("m{n}"));
    }
    client.remote_mut().seed("inbox", "m1", "l9", &[], REMOTE);
    client.remote_mut().seed("inbox", "m2", "l3", &[], BASE);
    client.rekey("inbox").unwrap();

    let mut rows: Vec<(String, String)> = client
        .storage()
        .rows("inbox")
        .into_iter()
        .map(|p| {
            (
                p.handle.as_str().to_string(),
                p.link_id.unwrap().as_str().to_string(),
            )
        })
        .collect();
    rows.sort();
    assert_eq!(
        rows,
        [("m1".into(), "l9".into()), ("m2".into(), "l3".into())],
        "{rows:?}"
    );
    let gone: Vec<_> = client
        .storage()
        .retained("inbox")
        .into_iter()
        .map(|item| item.link_id.as_str().to_string())
        .collect();
    assert_eq!(gone, ["l1", "l2"], "the gone members are retained");
}

/// The mass-absence guard of a rekey (SYNC §8) counts a gone member whose
/// handle the new space reuses as absent.
#[test]
fn a_rekey_counts_a_gone_member_whose_handle_is_reused_as_absent() {
    let mut remote = MemRemote::default();
    for n in 0..25 {
        remote.seed("inbox", &format!("m{n}"), &format!("l{n}"), &[], BASE);
    }
    let mut client = Client::new(remote);
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();

    for n in 0..25 {
        client.remote_mut().remove("inbox", &format!("m{n}"));
    }
    client.remote_mut().seed("inbox", "m0", "l99", &[], REMOTE);
    let report = client.rekey("inbox").unwrap();
    let parked = report.parked.expect("the rekey parks");
    assert_eq!((parked.absent, parked.bound), (25, 25), "{parked:?}");
}
