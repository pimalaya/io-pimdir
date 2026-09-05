//! # Hostile source model
//!
//! Property-based safety net over a source that does not keep its word:
//! the engine may not depend on any seam contract holding.
//!
//! Every contract (a sorted listing naming each handle once, a fetch
//! answering the handles given, a push reporting each change) is about
//! somebody else's code, and trusting one turns a wrong answer into a
//! corrupted replica.
//!
//! The in-memory server sits behind a layer mangling what it reports in
//! the ways a real one plausibly does, and the model asserts the engine
//! keeps its own laws regardless: every verb returns, no row is left in a
//! shape no later run can act on, no two live rows share a key.

use std::{collections::BTreeSet, convert::Infallible};

use io_pimdir::{
    change::PimdirChange,
    collection::{PimdirCheckpoint, PimdirCollectionId},
    mutate::PimdirMutation,
    object::PimdirObject,
    placement::{PimdirFlags, PimdirHandle, PimdirLinkId, PimdirPlacement, PimdirStatus},
    remote::{
        PimdirFetchedBody, PimdirFetchedItem, PimdirPushOutcome, PimdirPushResult, PimdirRemote,
        PimdirRemoteSnapshot, PimdirTier,
    },
    sync::PimdirSyncOptions,
};
use proptest::{prelude::*, test_runner::TestCaseError};

use crate::common::{Client, MemRemote, duplicate_key, hash, malformed};

/// How a source breaks its side of the contract, one field per way.
#[derive(Clone, Copy, Debug)]
struct Chaos {
    /// Report the members in reverse handle order.
    unsorted: bool,
    /// List the first member twice.
    repeat: bool,
    /// Report a member vanished while still listing it.
    phantom_vanished: bool,
    /// Answer a fetch for every handle but the first.
    skip_fetch: bool,
    /// Answer a fetch for a handle nobody asked about.
    ghost_fetch: bool,
    /// Resolve every member to one identity, as one `UID` on every resource.
    one_identity: bool,
    /// Return a body at the summary tier.
    meta_body: bool,
    /// Report a push result for a handle nobody pushed.
    ghost_push: bool,
    /// Report the first push result twice.
    repeat_push: bool,
    /// Report nothing for the first change pushed.
    silent_push: bool,
    /// Enumerate with no markers read, as a handles-only listing does.
    blind_flags: bool,
}

fn arb_chaos() -> impl Strategy<Value = Chaos> {
    proptest::collection::vec(any::<bool>(), 11).prop_map(|flags| Chaos {
        unsorted: flags[0],
        repeat: flags[1],
        phantom_vanished: flags[2],
        skip_fetch: flags[3],
        ghost_fetch: flags[4],
        one_identity: flags[5],
        meta_body: flags[6],
        ghost_push: flags[7],
        repeat_push: flags[8],
        silent_push: flags[9],
        blind_flags: flags[10],
    })
}

/// The in-memory server behind a layer that mangles what it reports.
struct HostileRemote {
    inner: MemRemote,
    chaos: Chaos,
}

impl PimdirRemote for HostileRemote {
    type Error = Infallible;

    fn enumerate(
        &mut self,
        collection: &PimdirCollectionId,
        cursor: Option<PimdirCheckpoint>,
    ) -> Result<PimdirRemoteSnapshot, Infallible> {
        let mut snapshot = self.inner.enumerate(collection, cursor)?;

        if self.chaos.unsorted {
            snapshot.items.reverse();
        }
        if self.chaos.repeat
            && let Some(first) = snapshot.items.first().cloned()
        {
            snapshot.items.push(first);
        }
        if self.chaos.phantom_vanished
            && let Some(first) = snapshot.items.first()
        {
            snapshot.vanished.push(first.handle.clone());
        }
        if self.chaos.blind_flags {
            for item in &mut snapshot.items {
                item.flags = PimdirFlags::Unknown;
            }
        }

        Ok(snapshot)
    }

    fn fetch(
        &mut self,
        collection: &PimdirCollectionId,
        handles: Vec<PimdirHandle>,
        tier: PimdirTier,
    ) -> Result<Vec<PimdirFetchedItem>, Infallible> {
        let mut items = self.inner.fetch(collection, handles, tier)?;

        if self.chaos.skip_fetch && !items.is_empty() {
            items.remove(0);
        }
        if self.chaos.one_identity {
            for item in &mut items {
                item.link_id = PimdirLinkId::from("one-identity");
            }
        }
        if self.chaos.meta_body {
            for item in &mut items {
                item.body.get_or_insert(PimdirFetchedBody::Inline {
                    hash: hash(b"a body no tier asked for"),
                    bytes: b"a body no tier asked for".to_vec(),
                });
            }
        }
        if self.chaos.ghost_fetch {
            items.push(PimdirFetchedItem {
                handle: PimdirHandle::from("ghost"),
                link_id: PimdirLinkId::from("ghost"),
                summary: None,
                sort_key: Default::default(),
                body: None,
                revision: None,
            });
        }

        Ok(items)
    }

    fn push(
        &mut self,
        collection: &PimdirCollectionId,
        changes: Vec<PimdirChange>,
    ) -> Result<Vec<PimdirPushResult>, Infallible> {
        let mut results = self.inner.push(collection, changes)?;

        if self.chaos.silent_push && !results.is_empty() {
            results.remove(0);
        }
        if self.chaos.repeat_push
            && let Some(first) = results.first().cloned()
        {
            results.push(first);
        }
        if self.chaos.ghost_push {
            results.push(PimdirPushResult {
                handle: PimdirHandle::from("ghost"),
                outcome: PimdirPushOutcome::Accepted,
                assigned: Some(PimdirHandle::from("ghost-assigned")),
                revision: None,
            });
        }

        Ok(results)
    }
}

type Hostile = Client<HostileRemote>;

/// One step of the hostile model: the local vocabulary over a bad source.
#[derive(Clone, Debug)]
enum HostileOp {
    Edit(usize, u8),
    Remove(usize),
    Copy(usize),
    ServerEdit(usize, u8),
    ServerRemove(usize),
    ServerAdd(u8),
    Hydrate,
    Summarise,
    Sync,
    SyncArchive,
    Rekey,
}

fn arb_hostile_op() -> impl Strategy<Value = HostileOp> {
    prop_oneof![
        2 => (any::<usize>(), any::<u8>()).prop_map(|(i, n)| HostileOp::Edit(i, n)),
        1 => any::<usize>().prop_map(HostileOp::Remove),
        1 => any::<usize>().prop_map(HostileOp::Copy),
        2 => (any::<usize>(), any::<u8>()).prop_map(|(i, n)| HostileOp::ServerEdit(i, n)),
        1 => any::<usize>().prop_map(HostileOp::ServerRemove),
        1 => any::<u8>().prop_map(HostileOp::ServerAdd),
        2 => Just(HostileOp::Hydrate),
        1 => Just(HostileOp::Summarise),
        3 => Just(HostileOp::Sync),
        1 => Just(HostileOp::SyncArchive),
        1 => Just(HostileOp::Rekey),
    ]
}

fn nth<T: Clone>(values: &BTreeSet<T>, i: usize) -> Option<T> {
    match values.is_empty() {
        true => None,
        false => values.iter().nth(i % values.len()).cloned(),
    }
}

/// The named, live rows: what a consumer can act on.
fn live(client: &Hostile, collection: &str) -> BTreeSet<PimdirHandle> {
    client
        .storage()
        .rows(collection)
        .into_iter()
        .filter(|p| p.status != PimdirStatus::Tombstone && p.link_id.is_some())
        .map(|p| p.handle)
        .collect()
}

/// Every live row, probes included: what an upgrade may raise.
fn every(client: &Hostile, collection: &str) -> Vec<PimdirHandle> {
    client
        .storage()
        .rows(collection)
        .into_iter()
        .filter(|p| p.status != PimdirStatus::Tombstone)
        .map(|p| p.handle)
        .collect()
}

fn on_server(client: &Hostile, collection: &str) -> BTreeSet<PimdirHandle> {
    client.remote().inner.handles(collection)
}

fn rows(client: &Hostile) -> Vec<PimdirPlacement> {
    client.storage().placements().into_values().collect()
}

fn intact(client: &Hostile, when: &str) -> Result<(), TestCaseError> {
    let rows = rows(client);
    for row in &rows {
        if let Some(broken) = malformed(row) {
            return Err(TestCaseError::fail(format!("{when}: {broken}")));
        }
    }
    if let Some(broken) = duplicate_key(&rows) {
        return Err(TestCaseError::fail(format!("{when}: {broken}")));
    }

    Ok(())
}

proptest! {
    /// Whatever the source gets wrong, the engine keeps its own laws.
    ///
    /// Every verb returns, every row stays in a shape a later run can act
    /// on, and no two live rows of a collection share a key.
    #[test]
    fn a_lying_source_never_corrupts_the_replica(
        chaos in arb_chaos(),
        ops in proptest::collection::vec(arb_hostile_op(), 0..20),
    ) {
        let mut inner = MemRemote::default();
        inner.mutable = true;
        inner.seed("inbox", "u1", "msg-a", &[], b"one");
        inner.seed("inbox", "u2", "msg-b", &["seen"], b"two");
        inner.seed("inbox", "u3", "msg-c", &[], b"three");

        let remote = HostileRemote { inner, chaos };
        let mut client = Client::new(remote);
        let opts = PimdirSyncOptions::default();
        client.sync("inbox", opts).map_err(TestCaseError::fail)?;
        intact(&client, "after the seeding sync")?;

        let mut arrivals = 0usize;
        let mut placeholders = 0usize;
        let mut bumps = 0usize;

        for op in ops {
            match op {
                HostileOp::Edit(i, n) => {
                    if let Some(handle) = nth(&live(&client, "inbox"), i) {
                        let body = format!("local-{n}-{}", handle.as_str()).into_bytes();
                        let _ = client.mutate("inbox", PimdirMutation::Edit {
                            handle,
                            object: PimdirObject { hash: hash(&body), size: body.len() },
                            body,
                            summary: None,
                            sort_key: None,
                        });
                    }
                }
                HostileOp::Remove(i) => {
                    if let Some(handle) = nth(&live(&client, "inbox"), i) {
                        let _ = client.mutate("inbox", PimdirMutation::Remove(handle));
                    }
                }
                HostileOp::Copy(i) => {
                    if let Some(handle) = nth(&live(&client, "inbox"), i) {
                        placeholders += 1;
                        let _ = client.mutate("inbox", PimdirMutation::Copy {
                            handle,
                            target: "archive".into(),
                            placeholder: PimdirHandle::from(format!("copy-{placeholders}")),
                        });
                    }
                }
                HostileOp::ServerEdit(i, n) => {
                    if let Some(handle) = nth(&on_server(&client, "inbox"), i) {
                        let body = format!("server-{n}").into_bytes();
                        client.remote_mut().inner.edit("inbox", handle.as_str(), &body);
                    }
                }
                HostileOp::ServerRemove(i) => {
                    if let Some(handle) = nth(&on_server(&client, "inbox"), i) {
                        client.remote_mut().inner.remove("inbox", handle.as_str());
                    }
                }
                HostileOp::ServerAdd(n) => {
                    arrivals += 1;
                    let handle = format!("srv-{arrivals}");
                    let link = format!("lnk-{arrivals}");
                    let body = format!("new-{n}").into_bytes();
                    client.remote_mut().inner.seed("inbox", &handle, &link, &[], &body);
                }
                HostileOp::Hydrate => {
                    let handles = every(&client, "inbox");
                    client.upgrade("inbox", handles, PimdirTier::Full).map_err(TestCaseError::fail)?;
                }
                HostileOp::Summarise => {
                    let handles = every(&client, "inbox");
                    client.upgrade("inbox", handles, PimdirTier::Meta).map_err(TestCaseError::fail)?;
                }
                HostileOp::Sync => {
                    client.sync("inbox", opts).map_err(TestCaseError::fail)?;
                }
                HostileOp::SyncArchive => {
                    client.sync("archive", opts).map_err(TestCaseError::fail)?;
                }
                HostileOp::Rekey => {
                    bumps += 1;
                    client.remote_mut().inner.renumber("inbox", bumps);
                    client.rekey("inbox").map_err(TestCaseError::fail)?;
                }
            }

            intact(&client, "mid-sequence")?;
        }

        for _ in 0..3 {
            client.sync("inbox", opts).map_err(TestCaseError::fail)?;
            client.sync("archive", opts).map_err(TestCaseError::fail)?;
        }
        intact(&client, "after quiescence")?;
    }
}
