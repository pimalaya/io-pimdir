//! A placement whose local body a run keeps keeps that body's summary and
//! sort key (SYNC §5): a listing naming the member at its base revision
//! describes the server's body, not the edit, whether the run pushes the
//! edit or keeps it pending.

use io_pimdir::{
    mutate::PimdirMutation,
    object::PimdirObject,
    placement::{PimdirHandle, PimdirSortKey},
    summary::{PimdirSummary, mail::PimdirMailSummary},
    sync::PimdirSyncOptions,
};

use crate::common::{Client, MemRemote, hash};

/// The subject and sort key the placement under `handle` holds.
fn described(client: &Client, handle: &str) -> (String, String) {
    let placement = client.storage().placement("book", handle);
    let subject = match &placement.summary {
        Some(PimdirSummary::Mail(mail)) => mail.subject.clone(),
        other => format!("{other:?}"),
    };
    (subject, placement.sort_key.0)
}

/// What the edit states.
fn the_edit() -> (String, String) {
    ("the edit's summary".into(), "edited-key".into())
}

/// A mutable member, synced, then edited offline: the edit states its
/// body's summary and sort key.
fn edited() -> Client {
    let mut remote = MemRemote::default();
    remote.mutable = true;
    remote.seed("book", "c1.vcf", "uid:c1", &[], b"FN:Alice");
    let mut client = Client::new(remote);
    client.sync("book", PimdirSyncOptions::default()).unwrap();

    let body = b"FN:Alice Edited".to_vec();
    client
        .mutate(
            "book",
            PimdirMutation::Edit {
                handle: PimdirHandle::from("c1.vcf"),
                object: PimdirObject {
                    hash: hash(&body),
                    size: body.len(),
                },
                body,
                summary: Some(PimdirSummary::Mail(PimdirMailSummary {
                    subject: "the edit's summary".into(),
                    ..Default::default()
                })),
                sort_key: Some(PimdirSortKey::from("edited-key")),
                if_match: None,
                overrule: false,
                resolves: None,
            },
        )
        .unwrap();
    assert_eq!(described(&client, "c1.vcf"), the_edit());
    client
}

/// A run keeping the edit unpushed, its round listing the member at the
/// unchanged revision.
#[test]
fn a_kept_edit_keeps_its_summary_under_a_full_listing() {
    let mut client = edited();
    let opts = PimdirSyncOptions {
        push: false,
        full: true,
        ..Default::default()
    };
    client.sync("book", opts).unwrap();
    assert_eq!(described(&client, "c1.vcf"), the_edit());
}

/// A run pushing the edit, its round listing the member at the base
/// revision first.
#[test]
fn a_pushed_edit_keeps_its_summary_under_a_full_listing() {
    let mut client = edited();
    let opts = PimdirSyncOptions {
        full: true,
        ..Default::default()
    };
    client.sync("book", opts).unwrap();
    assert_eq!(described(&client, "c1.vcf"), the_edit());
}

/// A rekey carrying the edit onto its new handle, the listing at the
/// unchanged revision.
#[test]
fn a_rekeyed_edit_keeps_its_summary() {
    let mut client = edited();
    client.remote_mut().renumber("book", 1);
    client.rekey("book").unwrap();
    assert_eq!(described(&client, "v1-0"), the_edit());
}
