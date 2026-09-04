//! # Identity properties
//!
//! Property-based safety net over one law: no two live rows of one
//! collection share a key.
//!
//! A store keyed by identity keeps one of two rows sharing a key, and
//! the dropped one takes its body, summary and pending edits with it. So
//! every verb assigning a key runs against the law: the upgrade minting a
//! second copy, the rebuild, a keep-both fork and a staged copy.
//!
//! The models are duplicate-heavy on purpose: four hints over five
//! members make the collision the common case, and two hints are spelled
//! like the keys the engine mints for itself.

use std::collections::{BTreeMap, BTreeSet};

use io_pimdir::{
    mutate::PimdirMutation,
    object::PimdirObject,
    placement::{PimdirHandle, PimdirLinkId, PimdirPlacement, PimdirStatus},
    remote::PimdirTier,
    sync::{PimdirConflictPolicy, PimdirSyncOptions},
};
use proptest::{prelude::*, test_runner::TestCaseError};

use crate::common::{Client, MemRemote, hash};

/// The two collections the models act on, copies and moves between them.
const COLLECTIONS: [&str; 2] = ["inbox", "archive"];

/// One step of the duplicate-identity model.
///
/// Handle picks are indices resolved modulo the live set at execution
/// time, so every generated op is valid by construction and shrinking
/// stays meaningful.
#[derive(Clone, Debug)]
enum IdOp {
    /// A member arrives upstream under a hint the collection already uses.
    ServerAdd(usize, u8),
    /// A member is expunged upstream.
    ServerRemove(usize),
    /// Copy the i-th inbox member into the archive.
    Copy(usize),
    /// Move the i-th inbox member into the archive.
    Move(usize),
    /// Stage a locally-authored member under one of the hints.
    Add(usize, u8),
    /// Hydrate every member of a collection, resolving identities.
    Hydrate(bool),
    /// The server renumbers the inbox and the replica rebuilds onto it.
    Bump,
    /// Sync one collection.
    Sync(bool),
}

/// The identities the model uses.
///
/// Two are spelled like the keys the engine mints for itself, which a
/// source is free to do, and neither may let one row take another's key.
const HINTS: [&str; 4] = ["msg-a", "msg-b", "dup:msg-a#u2", "keepboth\u{1}u1"];

/// Picks a hint, four of them for five members.
fn hint(which: usize) -> &'static str {
    HINTS[which % HINTS.len()]
}

fn arb_id_op() -> impl Strategy<Value = IdOp> {
    prop_oneof![
        3 => (any::<usize>(), any::<u8>()).prop_map(|(h, n)| IdOp::ServerAdd(h, n)),
        1 => any::<usize>().prop_map(IdOp::ServerRemove),
        2 => any::<usize>().prop_map(IdOp::Copy),
        2 => any::<usize>().prop_map(IdOp::Move),
        2 => (any::<usize>(), any::<u8>()).prop_map(|(h, n)| IdOp::Add(h, n)),
        3 => any::<bool>().prop_map(IdOp::Hydrate),
        1 => Just(IdOp::Bump),
        3 => any::<bool>().prop_map(IdOp::Sync),
    ]
}

fn collection(second: bool) -> &'static str {
    COLLECTIONS[usize::from(second)]
}

fn nth<T: Clone>(values: &BTreeSet<T>, i: usize) -> Option<T> {
    match values.is_empty() {
        true => None,
        false => values.iter().nth(i % values.len()).cloned(),
    }
}

fn rows(client: &Client, collection: &str) -> Vec<PimdirPlacement> {
    client.storage().rows(collection)
}

/// The named, live rows: what a consumer can act on.
fn live(client: &Client, collection: &str) -> BTreeSet<PimdirHandle> {
    rows(client, collection)
        .into_iter()
        .filter(|p| p.status != PimdirStatus::Tombstone && p.link_id.is_some())
        .map(|p| p.handle)
        .collect()
}

/// Every row, probes included: what an upgrade may raise.
fn every(client: &Client, collection: &str) -> Vec<PimdirHandle> {
    rows(client, collection)
        .into_iter()
        .filter(|p| p.status != PimdirStatus::Tombstone)
        .map(|p| p.handle)
        .collect()
}

fn on_server(client: &Client, collection: &str) -> BTreeSet<PimdirHandle> {
    client.remote().handles(collection)
}

/// The law: no two live rows of one collection carry one key.
///
/// Tombstones are exempt: a row on its way out holds no key against a
/// create, so an `Add` may re-create an identity deleted before the
/// remove was pushed.
fn one_key_per_row(client: &Client, when: &str) -> Result<(), TestCaseError> {
    for collection in COLLECTIONS {
        let mut holders: BTreeMap<PimdirLinkId, Vec<PimdirHandle>> = BTreeMap::new();
        for placement in rows(client, collection) {
            if placement.status == PimdirStatus::Tombstone {
                continue;
            }
            if let Some(link) = placement.link_id {
                holders.entry(link).or_default().push(placement.handle);
            }
        }
        for (link, handles) in holders {
            prop_assert!(
                handles.len() < 2,
                "{when}: {collection} holds {link:?} on {handles:?}",
            );
        }
    }

    Ok(())
}

/// Every body a row points at is held by the store.
fn every_body_is_held(client: &Client, when: &str) -> Result<(), TestCaseError> {
    for placement in COLLECTIONS.iter().flat_map(|c| rows(client, c)) {
        if let Some(object) = &placement.object {
            prop_assert!(
                client.storage().body(object).is_some(),
                "{when}: {placement:?} points at an unheld body",
            );
        }
    }

    Ok(())
}

fn seeded() -> Client {
    let mut remote = MemRemote::default();
    remote.seed("inbox", "u1", "msg-a", &[], b"one");
    remote.seed("inbox", "u2", "msg-a", &[], b"two");
    remote.seed("inbox", "u3", "msg-b", &[], b"three");
    remote.seed("archive", "a1", "msg-b", &[], b"archived");

    Client::new(remote)
}

/// Runs the duplicate-identity model, asserting the law after every step.
fn check_identity_model(ops: Vec<IdOp>) -> Result<(), TestCaseError> {
    let mut client = seeded();
    let opts = PimdirSyncOptions {
        full: true,
        ..Default::default()
    };
    for collection in COLLECTIONS {
        client.sync(collection, opts).unwrap();
    }

    let mut arrivals = 0usize;
    let mut placeholders = 0usize;
    let mut bumps = 0usize;

    for op in ops {
        match op {
            IdOp::ServerAdd(which, n) => {
                arrivals += 1;
                let handle = format!("srv-{arrivals}");
                let body = format!("arrival-{n}").into_bytes();
                client
                    .remote_mut()
                    .seed("inbox", &handle, hint(which), &[], &body);
            }
            IdOp::ServerRemove(i) => {
                if let Some(handle) = nth(&on_server(&client, "inbox"), i) {
                    client.remote_mut().remove("inbox", handle.as_str());
                }
            }
            IdOp::Copy(i) => {
                if let Some(handle) = nth(&live(&client, "inbox"), i) {
                    placeholders += 1;
                    let _ = client.mutate(
                        "inbox",
                        PimdirMutation::Copy {
                            handle,
                            target: "archive".into(),
                            placeholder: PimdirHandle::from(format!("copy-{placeholders}")),
                        },
                    );
                }
            }
            IdOp::Move(i) => {
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
            IdOp::Add(which, n) => {
                placeholders += 1;
                let body = format!("authored-{n}").into_bytes();
                let _ = client.mutate(
                    "inbox",
                    PimdirMutation::Add {
                        handle: PimdirHandle::from(format!("add-{placeholders}")),
                        link_id: PimdirLinkId::from(hint(which)),
                        flags: Default::default(),
                        object: PimdirObject {
                            hash: hash(&body),
                            size: body.len(),
                        },
                        body,
                        summary: None,
                        sort_key: Default::default(),
                    },
                );
            }
            IdOp::Hydrate(second) => {
                let collection = collection(second);
                let handles = every(&client, collection);
                let _ = client.upgrade(collection, handles, PimdirTier::Full);
            }
            IdOp::Bump => {
                bumps += 1;
                client.remote_mut().renumber("inbox", bumps);
                client.rekey("inbox").unwrap();
            }
            IdOp::Sync(second) => {
                client.sync(collection(second), opts).unwrap();
            }
        }

        one_key_per_row(&client, "mid-sequence")?;
        every_body_is_held(&client, "mid-sequence")?;
    }

    for _ in 0..3 {
        for collection in COLLECTIONS {
            client.sync(collection, opts).unwrap();
            let handles = every(&client, collection);
            let _ = client.upgrade(collection, handles, PimdirTier::Full);
        }
    }
    one_key_per_row(&client, "after quiescence")?;
    every_body_is_held(&client, "after quiescence")?;

    for collection in COLLECTIONS {
        let local: BTreeSet<PimdirHandle> = every(&client, collection).into_iter().collect();
        let server = on_server(&client, collection);
        let missing: Vec<&PimdirHandle> = server.difference(&local).collect();
        prop_assert!(
            missing.is_empty(),
            "{collection}: no row for {missing:?} (local {local:?})",
        );
    }

    Ok(())
}

proptest! {
    /// Two live rows of one collection never share a key, whatever the ops.
    ///
    /// Also, no row points at an unheld body, and every member the source
    /// holds keeps a row of its own.
    #[test]
    fn duplicate_identities_never_share_a_key(
        ops in proptest::collection::vec(arb_id_op(), 0..20),
    ) {
        check_identity_model(ops)?;
    }
}

/// One step of the keep-both model, where the merge forks rows itself.
#[derive(Clone, Debug)]
enum ForkOp {
    /// Edit the i-th member to one of three bodies.
    LocalEdit(usize, u8),
    /// A server-side content change.
    ServerEdit(usize, u8),
    /// Sync the inbox.
    Sync,
}

fn arb_fork_op() -> impl Strategy<Value = ForkOp> {
    prop_oneof![
        3 => (any::<usize>(), 0u8..3).prop_map(|(i, n)| ForkOp::LocalEdit(i, n)),
        3 => (any::<usize>(), 0u8..3).prop_map(|(i, n)| ForkOp::ServerEdit(i, n)),
        2 => Just(ForkOp::Sync),
    ]
}

proptest! {
    /// Two keep-both forks over one body in one run are two members.
    ///
    /// Keying the fork on the body alone would give them one key, and the
    /// second fork would take the first's row.
    #[test]
    fn keep_both_forks_never_share_a_key(
        ops in proptest::collection::vec(arb_fork_op(), 0..16),
    ) {
        let mut remote = MemRemote::default();
        remote.mutable = true;
        remote.seed("inbox", "u1", "msg-a", &[], b"one");
        remote.seed("inbox", "u2", "msg-b", &[], b"two");
        remote.seed("inbox", "u3", "msg-c", &[], b"three");

        let mut client = Client::new(remote);
        let opts = PimdirSyncOptions {
            conflict: PimdirConflictPolicy::KeepBoth,
            ..Default::default()
        };
        client.sync("inbox", opts).unwrap();
        let handles = every(&client, "inbox");
        client.upgrade("inbox", handles, PimdirTier::Full).unwrap();

        for op in ops {
            match op {
                ForkOp::LocalEdit(i, n) => {
                    if let Some(handle) = nth(&live(&client, "inbox"), i) {
                        let body = format!("local-{n}").into_bytes();
                        let _ = client.mutate("inbox", PimdirMutation::Edit {
                            handle,
                            object: PimdirObject { hash: hash(&body), size: body.len() },
                            body,
                            summary: None,
                            sort_key: None,
                        });
                    }
                }
                ForkOp::ServerEdit(i, n) => {
                    if let Some(handle) = nth(&on_server(&client, "inbox"), i) {
                        let body = format!("server-{n}").into_bytes();
                        client.remote_mut().edit("inbox", handle.as_str(), &body);
                    }
                }
                ForkOp::Sync => {
                    client.sync("inbox", opts).unwrap();
                }
            }

            one_key_per_row(&client, "mid-sequence")?;
        }

        for _ in 0..3 {
            client.sync("inbox", opts).unwrap();
        }
        one_key_per_row(&client, "after quiescence")?;
    }
}
