//! # Sync option matrix model
//!
//! Property-based safety net over the sync options, and over the shapes
//! a placement may never be left in.
//!
//! The other models run one set of options. The options are what a
//! consumer tunes, and they interact: a source that may not remove still
//! edits, a refused delete undoes a tombstone carrying other intent, a
//! conflict policy decides content while the flag axis merges.
//!
//! It asserts not convergence, which most of the matrix cannot reach,
//! but the shapes a row may never be written in. A `Clean` row pointing
//! at a body its base does not hold matters most: nothing derives a push
//! for it again, and the replica keeps content the source never heard of.

use std::collections::BTreeSet;

use io_pimdir::{
    mutate::PimdirMutation,
    object::PimdirObject,
    placement::{PimdirFlags, PimdirHandle, PimdirLevel, PimdirPlacement, PimdirStatus},
    remote::PimdirTier,
    sync::{
        PimdirConflictPolicy, PimdirDeletePolicy, PimdirPushRights, PimdirSyncEvent,
        PimdirSyncOptions,
    },
};
use proptest::{prelude::*, test_runner::TestCaseError};

use crate::common::{Client, MemRemote, hash};

/// One step of the option matrix model.
#[derive(Clone, Debug)]
enum PolicyOp {
    SetFlags(usize, PimdirFlags),
    Remove(usize),
    Edit(usize, u8),
    /// Edit a row the user already deleted, which revives it.
    ///
    /// What resolving a hub projection of an item deleted on another
    /// source looks like from here.
    EditDeleted(usize, u8),
    Move(usize),
    ServerSetFlags(usize, PimdirFlags),
    ServerEdit(usize, u8),
    ServerRemove(usize),
    ServerAdd(u8),
    Upgrade(usize),
    /// Reconcile the inbox under generated options, every run tuned anew.
    Sync(PimdirSyncOptions),
    SyncArchive,
}

fn arb_flags() -> impl Strategy<Value = PimdirFlags> {
    proptest::collection::btree_set(prop_oneof![Just("seen"), Just("flagged")], 0..3)
        .prop_map(PimdirFlags::from_iter)
}

fn arb_rights() -> impl Strategy<Value = PimdirPushRights> {
    (any::<bool>(), any::<bool>(), any::<bool>(), any::<bool>()).prop_map(
        |(flags, content, add, remove)| PimdirPushRights {
            flags,
            content,
            add,
            remove,
        },
    )
}

fn arb_opts() -> impl Strategy<Value = PimdirSyncOptions> {
    (
        any::<bool>(),
        arb_rights(),
        prop_oneof![
            Just(PimdirDeletePolicy::Revert),
            Just(PimdirDeletePolicy::Keep)
        ],
        prop_oneof![
            Just(PimdirConflictPolicy::Manual),
            Just(PimdirConflictPolicy::PreferLocal),
            Just(PimdirConflictPolicy::PreferRemote),
            Just(PimdirConflictPolicy::KeepBoth),
        ],
        any::<bool>(),
    )
        .prop_map(|(push, rights, delete, conflict, full)| PimdirSyncOptions {
            push,
            rights,
            delete,
            conflict,
            full,
        })
}

fn arb_policy_op() -> impl Strategy<Value = PolicyOp> {
    prop_oneof![
        1 => (any::<usize>(), arb_flags()).prop_map(|(i, f)| PolicyOp::SetFlags(i, f)),
        2 => any::<usize>().prop_map(PolicyOp::Remove),
        3 => (any::<usize>(), any::<u8>()).prop_map(|(i, n)| PolicyOp::Edit(i, n)),
        1 => (any::<usize>(), any::<u8>()).prop_map(|(i, n)| PolicyOp::EditDeleted(i, n)),
        1 => any::<usize>().prop_map(PolicyOp::Move),
        1 => (any::<usize>(), arb_flags()).prop_map(|(i, f)| PolicyOp::ServerSetFlags(i, f)),
        3 => (any::<usize>(), any::<u8>()).prop_map(|(i, n)| PolicyOp::ServerEdit(i, n)),
        1 => any::<usize>().prop_map(PolicyOp::ServerRemove),
        1 => any::<u8>().prop_map(PolicyOp::ServerAdd),
        2 => any::<usize>().prop_map(PolicyOp::Upgrade),
        4 => arb_opts().prop_map(PolicyOp::Sync),
        1 => Just(PolicyOp::SyncArchive),
    ]
}

fn nth<T: Clone>(values: &BTreeSet<T>, i: usize) -> Option<T> {
    match values.is_empty() {
        true => None,
        false => values.iter().nth(i % values.len()).cloned(),
    }
}

fn rows(client: &Client) -> Vec<PimdirPlacement> {
    client.storage().placements().into_values().collect()
}

/// The rows a user already deleted, which a hub still projects for edits.
fn deleted(client: &Client, collection: &str) -> BTreeSet<PimdirHandle> {
    client
        .storage()
        .rows(collection)
        .into_iter()
        .filter(|p| p.status == PimdirStatus::Tombstone)
        .map(|p| p.handle)
        .collect()
}

/// The named, live rows: what a consumer can act on.
fn live(client: &Client, collection: &str) -> BTreeSet<PimdirHandle> {
    client
        .storage()
        .rows(collection)
        .into_iter()
        .filter(|p| p.status != PimdirStatus::Tombstone && p.link_id.is_some())
        .map(|p| p.handle)
        .collect()
}

/// Every live row, probes included: what an upgrade may raise.
fn every(client: &Client, collection: &str) -> Vec<PimdirHandle> {
    client
        .storage()
        .rows(collection)
        .into_iter()
        .filter(|p| p.status != PimdirStatus::Tombstone)
        .map(|p| p.handle)
        .collect()
}

fn on_server(client: &Client, collection: &str) -> BTreeSet<PimdirHandle> {
    client.remote().handles(collection)
}

/// Names every inbox member, so the local ops have rows to act on.
fn hydrate(client: &mut Client) {
    let handles = every(client, "inbox");
    let _ = client.upgrade("inbox", handles, PimdirTier::Meta);
}

/// The shapes no verb may leave a row in, whatever the options were.
fn well_formed(client: &Client, when: &str) -> Result<(), TestCaseError> {
    for row in rows(client) {
        prop_assert!(
            row.staged_edit().is_none() || row.status != PimdirStatus::Clean,
            "{when}: a clean row holds a staged body: {row:?}",
        );
        prop_assert!(
            row.level < PimdirLevel::Full || row.object.is_some(),
            "{when}: a full row holds no body: {row:?}",
        );
        prop_assert!(
            row.status != PimdirStatus::Created || row.base.is_none(),
            "{when}: a create carries a base: {row:?}",
        );
        prop_assert!(
            row.conflict_object.is_none() || row.conflict_revision.is_some(),
            "{when}: a conflict body outlives its revision: {row:?}",
        );
        // NOTE: a tombstone keeps the divergence it is deleting, so a
        // refused delete restores the conflict rather than settling it.
        prop_assert!(
            row.conflict_revision.is_none()
                || matches!(row.status, PimdirStatus::Conflict | PimdirStatus::Tombstone),
            "{when}: an unconflicted row tracks a conflict revision: {row:?}",
        );
        prop_assert!(
            row.origin.is_none()
                || matches!(row.status, PimdirStatus::Created | PimdirStatus::Tombstone),
            "{when}: a settled row carries a move destination: {row:?}",
        );
    }

    Ok(())
}

/// Runs the option matrix model.
fn check_policy_model(ops: Vec<PolicyOp>) -> Result<(), TestCaseError> {
    let mut remote = MemRemote::default();
    remote.mutable = true;
    remote.seed("inbox", "m1", "l1", &[], b"one");
    remote.seed("inbox", "m2", "l2", &["seen"], b"two");
    remote.seed("inbox", "m3", "l3", &["flagged"], b"three");
    remote.seed("inbox", "m4", "l4", &[], b"four");

    let mut client = Client::new(remote);
    let writable = PimdirSyncOptions::default();
    client.sync("inbox", writable).unwrap();
    hydrate(&mut client);
    well_formed(&client, "after the seeding sync")?;

    let mut arrivals = 0usize;
    let mut placeholders = 0usize;

    for op in ops {
        match op {
            PolicyOp::SetFlags(i, flags) => {
                if let Some(handle) = nth(&live(&client, "inbox"), i) {
                    let _ = client.mutate("inbox", PimdirMutation::SetFlags { handle, flags });
                }
            }
            PolicyOp::Remove(i) => {
                if let Some(handle) = nth(&live(&client, "inbox"), i) {
                    let _ = client.mutate("inbox", PimdirMutation::Remove(handle));
                }
            }
            PolicyOp::Edit(i, n) => {
                if let Some(handle) = nth(&live(&client, "inbox"), i) {
                    let body = format!("local-{n}-{}", handle.as_str()).into_bytes();
                    let _ = client.mutate(
                        "inbox",
                        PimdirMutation::Edit {
                            handle,
                            object: PimdirObject {
                                hash: hash(&body),
                                size: body.len(),
                            },
                            body,
                            summary: None,
                            sort_key: None,
                        },
                    );
                }
            }
            PolicyOp::EditDeleted(i, n) => {
                if let Some(handle) = nth(&deleted(&client, "inbox"), i) {
                    let body = format!("revived-{n}-{}", handle.as_str()).into_bytes();
                    let _ = client.mutate(
                        "inbox",
                        PimdirMutation::Edit {
                            handle,
                            object: PimdirObject {
                                hash: hash(&body),
                                size: body.len(),
                            },
                            body,
                            summary: None,
                            sort_key: None,
                        },
                    );
                }
            }
            PolicyOp::Move(i) => {
                if let Some(handle) = nth(&live(&client, "inbox"), i) {
                    placeholders += 1;
                    let _ = client.mutate(
                        "inbox",
                        PimdirMutation::Move {
                            handle,
                            target: "archive".into(),
                            placeholder: PimdirHandle::from(format!("move-{placeholders}")),
                        },
                    );
                }
            }
            PolicyOp::ServerSetFlags(i, flags) => {
                if let Some(handle) = nth(&on_server(&client, "inbox"), i) {
                    let flags: Vec<&str> = flags
                        .known()
                        .into_iter()
                        .flatten()
                        .map(|f| f.as_str())
                        .collect();
                    client
                        .remote_mut()
                        .set_flags("inbox", handle.as_str(), &flags);
                }
            }
            PolicyOp::ServerEdit(i, n) => {
                if let Some(handle) = nth(&on_server(&client, "inbox"), i) {
                    let body = format!("server-{n}").into_bytes();
                    client.remote_mut().edit("inbox", handle.as_str(), &body);
                }
            }
            PolicyOp::ServerRemove(i) => {
                if let Some(handle) = nth(&on_server(&client, "inbox"), i) {
                    client.remote_mut().remove("inbox", handle.as_str());
                }
            }
            PolicyOp::ServerAdd(n) => {
                arrivals += 1;
                let handle = format!("srv-{arrivals}");
                let link = format!("lnk-{arrivals}");
                let body = format!("new-{n}").into_bytes();
                client
                    .remote_mut()
                    .seed("inbox", &handle, &link, &[], &body);
            }
            PolicyOp::Upgrade(i) => {
                if let Some(handle) = nth(&live(&client, "inbox"), i) {
                    let _ = client.upgrade("inbox", vec![handle], PimdirTier::Full);
                }
            }
            PolicyOp::Sync(opts) => {
                let report = client.sync("inbox", opts).unwrap();
                let conflicted = report
                    .events
                    .iter()
                    .filter(|event| matches!(event, PimdirSyncEvent::Conflicted(_)))
                    .count();
                prop_assert_eq!(
                    report.conflicts,
                    conflicted,
                    "the counters summarise the events: {:?}",
                    report,
                );
                hydrate(&mut client);
            }
            PolicyOp::SyncArchive => {
                client.sync("archive", writable).unwrap();
            }
        }

        well_formed(&client, "mid-sequence")?;
    }

    for _ in 0..3 {
        client.sync("inbox", writable).unwrap();
        client.sync("archive", writable).unwrap();
    }
    well_formed(&client, "after a writable quiescence")?;

    Ok(())
}

proptest! {
    /// No option leaves a row in a shape the engine cannot act on again.
    #[test]
    fn no_option_leaves_an_unreadable_row(
        ops in proptest::collection::vec(arb_policy_op(), 0..24),
    ) {
        check_policy_model(ops)?;
    }
}
