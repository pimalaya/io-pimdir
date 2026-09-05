//! # Hub properties
//!
//! Property-based safety net over the hub axis: generated op sequences
//! over three sources bound to one shared store.
//!
//! Five laws are asserted: every source ends on one body, a source never
//! diverges from itself, a genuine divergence between two sources is
//! reported rather than silently resolved, no staged body is lost without
//! a strictly later action taking its place, and an edit beats a delete
//! across sources: a server deleting a body it never held is offered the
//! item back.

use std::collections::BTreeMap;

use io_pimdir::{
    hub::{PimdirHub, PimdirSourceId},
    mutate::PimdirMutation,
    object::{PimdirHash, PimdirObject},
    placement::{PimdirFlags, PimdirHandle, PimdirLinkId},
    remote::PimdirTier,
    sync::{PimdirSyncOptions, PimdirSyncReport},
};
use proptest::{prelude::*, test_runner::TestCaseError};

use crate::common::{Client, MemRemote, hash};

/// How many sources the cluster runs.
///
/// Three rather than two, so a divergence between two of them has a
/// bystander whose binding it must not disturb.
const SOURCES: usize = 3;

/// One step of the hub scenario.
///
/// Source and item picks are indices resolved modulo the live sets at
/// execution time, so every generated op is valid by construction and
/// shrinking stays meaningful.
#[derive(Clone, Debug)]
enum HubOp {
    /// Stage a content edit on the i-th shared item from the s-th source.
    Edit(usize, usize, u8),
    /// Replace the i-th item's flags from the s-th source.
    SetFlags(usize, usize, PimdirFlags),
    /// Delete the i-th item on the s-th source.
    Remove(usize, usize),
    /// A new member arrives on the s-th source's server.
    ServerAdd(usize, u8),
    /// Stage a locally-authored item on the s-th source, a pending create.
    Add(usize, u8),
    /// The i-th item vanishes from the s-th source's server.
    ServerRemove(usize, usize),
    /// Sync and hydrate the s-th source.
    Sync(usize),
    /// The s-th source renumbers its handle space and the replica rebuilds.
    Bump(usize),
}

/// A small flag universe, so the sets overlap and the merge has work.
fn arb_flags() -> impl Strategy<Value = PimdirFlags> {
    proptest::collection::btree_set(
        prop_oneof![Just("seen"), Just("flagged"), Just("draft")],
        0..3,
    )
    .prop_map(PimdirFlags::from_iter)
}

/// Weighted toward edits, since a divergence needs two of them on one item.
fn arb_hub_op() -> impl Strategy<Value = HubOp> {
    prop_oneof![
        4 => (any::<usize>(), any::<usize>(), any::<u8>())
            .prop_map(|(s, i, n)| HubOp::Edit(s, i, n)),
        1 => (any::<usize>(), any::<usize>(), arb_flags())
            .prop_map(|(s, i, f)| HubOp::SetFlags(s, i, f)),
        1 => (any::<usize>(), any::<usize>()).prop_map(|(s, i)| HubOp::Remove(s, i)),
        1 => (any::<usize>(), any::<u8>()).prop_map(|(s, n)| HubOp::ServerAdd(s, n)),
        1 => (any::<usize>(), any::<u8>()).prop_map(|(s, n)| HubOp::Add(s, n)),
        1 => (any::<usize>(), any::<usize>()).prop_map(|(s, i)| HubOp::ServerRemove(s, i)),
        2 => any::<usize>().prop_map(HubOp::Sync),
        1 => any::<usize>().prop_map(HubOp::Bump),
    ]
}

/// Several sources over one store, each with its own server.
struct Cluster {
    sources: Vec<Client>,
}

impl Cluster {
    /// A cluster of [`SOURCES`] mutable-content sources, one member each.
    fn new() -> Self {
        let mut sources: Vec<Client> = Vec::new();
        for source in 0..SOURCES {
            let mut remote = MemRemote::default();
            remote.mutable = true;
            remote.seed(
                "inbox",
                &format!("h{source}"),
                &format!("msg-{source}"),
                &[],
                format!("seeded on s{source}").as_bytes(),
            );
            let name = format!("s{source}");
            let client = match sources.first() {
                None => Client::with_source(remote, &name),
                Some(first) => first.sharing(remote, &name),
            };
            sources.push(client);
        }

        Self { sources }
    }

    /// The inbox hub, every source included.
    fn hub(&self) -> PimdirHub {
        self.sources[0].hub("inbox")
    }

    /// Syncs and hydrates one source.
    ///
    /// Hydration is not optional: a pulled row carries no link id, so the
    /// hub cannot key it, and a body the hub does not hold is a member it
    /// cannot offer another source.
    fn sync(&mut self, source: usize) {
        let client = &mut self.sources[source];
        if client.sync("inbox", PimdirSyncOptions::default()).is_err() {
            return;
        }
        let Ok(opened) = client.open("inbox") else {
            return;
        };
        let handles = opened.placements.iter().map(|p| p.handle.clone()).collect();
        let _ = client.upgrade("inbox", handles, PimdirTier::Full);
    }

    /// Rounds over every source until the hub stops changing.
    fn quiesce(&mut self) -> Result<(), TestCaseError> {
        for round in 0..16 {
            let before = self.hub();
            for source in 0..SOURCES {
                self.sync(source);
            }
            if self.hub() == before {
                return Ok(());
            }
            prop_assert!(round < 15, "the hub never settled");
        }
        Ok(())
    }

    /// The links the hub holds, in order: what an item index picks from.
    fn links(&self) -> Vec<PimdirLinkId> {
        self.hub().items.keys().cloned().collect()
    }

    /// The handle a source binds a link under, if it holds the item.
    fn handle(&self, source: usize, link: &PimdirLinkId) -> Option<PimdirHandle> {
        let hub = self.hub();
        let item = hub.items.get(link)?;
        let binding = item
            .sources
            .get(&PimdirSourceId::from(format!("s{source}")))?;

        Some(binding.handle.clone())
    }

    /// Whether the source has reconciled `link` with its own server.
    ///
    /// A binding without a base is a pending create, which a rebuild
    /// leaves where it stands rather than carrying it onto the new
    /// handle space.
    fn synced(&self, source: usize, link: &PimdirLinkId) -> bool {
        let hub = self.hub();
        let Some(item) = hub.items.get(link) else {
            return false;
        };
        let source = PimdirSourceId::from(format!("s{source}"));

        item.sources
            .get(&source)
            .is_some_and(|binding| binding.base.is_some())
    }

    /// The body a source last synced with its own server for `link`.
    ///
    /// An edit restating it stages nothing at all.
    fn synced_object(&self, source: usize, link: &PimdirLinkId) -> Option<PimdirHash> {
        let hub = self.hub();
        let item = hub.items.get(link)?;
        let source = PimdirSourceId::from(format!("s{source}"));

        item.sources.get(&source)?.base.as_ref()?.object.clone()
    }

    /// The object a source's server holds under `handle`.
    ///
    /// The fake remote records a pushed body as the object's hash written
    /// out and a seeded one as the bytes themselves, so both spellings
    /// resolve to the same object here.
    fn server_object(&self, source: usize, handle: &PimdirHandle) -> Option<PimdirHash> {
        let body = self.sources[source]
            .remote()
            .items
            .get(&"inbox".into())?
            .get(handle)
            .map(|item| item.body.clone())?;
        let written = PimdirHash::from(String::from_utf8_lossy(&body).into_owned());
        let indexed = self.sources[source]
            .store
            .object_size(written.as_str())
            .expect("the index reads")
            .is_some();

        match indexed {
            true => Some(written),
            false => Some(hash(&body)),
        }
    }

    /// The flags a source's server holds under `handle`.
    fn server_flags(&self, source: usize, handle: &PimdirHandle) -> Option<PimdirFlags> {
        self.sources[source]
            .remote()
            .items
            .get(&"inbox".into())?
            .get(handle)
            .map(|item| item.flags.clone())
    }
}

/// What the ops asked of one shared item, and what the hub owes for it.
///
/// Kept from the ops alone rather than from the bindings, so a binding
/// recording the wrong agreement point is a failure, not an agreement.
#[derive(Default)]
struct Owed {
    /// The body the hub shares for the item.
    body: Option<PimdirHash>,
    /// The source that staged that body, `None` for the seeded one.
    author: Option<usize>,
    /// The shared body each source last agreed with.
    ///
    /// A source whose agreement point is the current shared body has seen
    /// it, so its next edit is made on top of it; one that is behind
    /// stages a body the other never saw.
    agreed: BTreeMap<usize, Option<PimdirHash>>,
    /// Set once two sources staged different bodies, the second unseen.
    diverged: bool,
    /// Set when an edit landed on the item after that divergence.
    resolved: bool,
    /// Set by a delete, cleared by any later live write.
    ///
    /// A server-side delete sets it only when that server held the shared
    /// body: a body it never saw is an edit, and an edit beats a delete
    /// across sources, the deleting source being offered the item back.
    removed: bool,
}

impl Owed {
    /// Whether the source has seen the body the hub currently shares.
    fn seen(&self, source: usize) -> bool {
        self.agreed.get(&source) == Some(&self.body)
    }

    /// Records that a live write of this source landed on the shared body.
    ///
    /// A tombstone adopts no content and moves no agreement point, so it
    /// never comes through here.
    fn agree(&mut self, source: usize) {
        self.agreed.insert(source, self.body.clone());
    }
}

/// The ledger of what every generated op asked for, per shared item.
type Ledger = BTreeMap<PimdirLinkId, Owed>;

fn nth<T: Clone>(values: &[T], i: usize) -> Option<T> {
    match values.is_empty() {
        true => None,
        false => values.get(i % values.len()).cloned(),
    }
}

/// Whether the hub still holds the item as live.
fn live(cluster: &Cluster, link: &PimdirLinkId) -> bool {
    cluster
        .hub()
        .items
        .get(link)
        .is_some_and(|item| !item.deleted)
}

/// Whether the hub reads the item as a cross-source divergence.
fn conflicted(cluster: &Cluster, link: &PimdirLinkId) -> bool {
    cluster
        .hub()
        .items
        .get(link)
        .is_some_and(|item| item.conflicted)
}

/// Runs the hub scenario, then the convergence and ledger assertions.
fn check_hub_model(ops: Vec<HubOp>) -> Result<(), TestCaseError> {
    let mut cluster = Cluster::new();
    cluster.quiesce()?;

    let mut ledger: Ledger = cluster
        .hub()
        .items
        .iter()
        .map(|(link, item)| {
            let agreed = (0..SOURCES).map(|s| (s, item.object.clone())).collect();
            let owed = Owed {
                body: item.object.clone(),
                agreed,
                ..Owed::default()
            };
            (link.clone(), owed)
        })
        .collect();
    let mut arrivals = 0usize;
    let mut authored = 0usize;
    let mut bumps = 0usize;

    for op in ops {
        match op {
            HubOp::Edit(s, i, n) => {
                let source = s % SOURCES;
                let Some(link) = nth(&cluster.links(), i) else {
                    continue;
                };
                let Some(handle) = cluster.handle(source, &link) else {
                    continue;
                };
                let body = format!("edit-{n}-{}", link.as_str()).into_bytes();
                let object = PimdirObject {
                    hash: hash(&body),
                    size: body.len(),
                };
                let owed = ledger.entry(link.clone()).or_default();
                let stages = cluster.synced_object(source, &link) != Some(object.hash.clone());
                let diverging = stages
                    && !owed.seen(source)
                    && owed.body.as_ref().is_some_and(|held| held != &object.hash);
                let first = owed.diverged;
                let was_conflicted = conflicted(&cluster, &link);
                let alone = owed.author.is_none_or(|author| author == source);
                let held = owed.body.clone();

                let staged = cluster.sources[source].mutate(
                    "inbox",
                    PimdirMutation::Edit {
                        handle,
                        object: object.clone(),
                        body,
                        summary: None,
                        sort_key: None,
                    },
                );
                if staged.is_err() {
                    continue;
                }

                if diverging && !first {
                    let hub = cluster.hub();
                    let item = hub.items.get(&link).expect("a hubbed item");
                    prop_assert!(
                        item.conflicted,
                        "two sources diverged on {link:?} and the hub resolved it silently: {item:?}",
                    );
                    prop_assert_eq!(
                        item.conflict_object.as_ref(),
                        Some(&object.hash),
                        "the diverging body is what the conflict records",
                    );
                    prop_assert_eq!(
                        item.object.clone(),
                        held.clone(),
                        "and the body it diverged from is kept",
                    );
                } else if !was_conflicted && alone {
                    prop_assert!(
                        !conflicted(&cluster, &link),
                        "s{source} was read as diverging from itself on {link:?}",
                    );
                }

                let adopted = live(&cluster, &link);
                let owed = ledger.entry(link).or_default();
                owed.resolved = owed.diverged;
                owed.diverged |= diverging;
                owed.removed &= !stages;

                // NOTE: a divergence leaves the shared body in place, as
                // the hub's manual policy does
                if stages && !diverging && owed.body.as_ref() != Some(&object.hash) {
                    owed.body = Some(object.hash);
                    owed.author = Some(source);
                }
                if adopted {
                    owed.agree(source);
                }
            }
            HubOp::SetFlags(s, i, flags) => {
                let source = s % SOURCES;
                let Some(link) = nth(&cluster.links(), i) else {
                    continue;
                };
                let Some(handle) = cluster.handle(source, &link) else {
                    continue;
                };
                let staged = cluster.sources[source]
                    .mutate("inbox", PimdirMutation::SetFlags { handle, flags });
                if staged.is_ok() && live(&cluster, &link) {
                    ledger.entry(link).or_default().agree(source);
                }
            }
            HubOp::Remove(s, i) => {
                let source = s % SOURCES;
                let Some(link) = nth(&cluster.links(), i) else {
                    continue;
                };
                let Some(handle) = cluster.handle(source, &link) else {
                    continue;
                };
                let staged =
                    cluster.sources[source].mutate("inbox", PimdirMutation::Remove(handle));
                if staged.is_ok() {
                    ledger.entry(link).or_default().removed = true;
                }
            }
            HubOp::ServerAdd(s, n) => {
                arrivals += 1;
                let source = s % SOURCES;
                cluster.sources[source].remote_mut().seed(
                    "inbox",
                    &format!("srv-{arrivals}"),
                    &format!("lnk-{arrivals}"),
                    &[],
                    format!("arrival-{n}").as_bytes(),
                );
            }
            HubOp::ServerRemove(s, i) => {
                let source = s % SOURCES;
                let Some(link) = nth(&cluster.links(), i) else {
                    continue;
                };
                let Some(handle) = cluster.handle(source, &link) else {
                    continue;
                };
                if cluster.server_object(source, &handle).is_none() {
                    continue;
                }
                let shared = cluster
                    .hub()
                    .items
                    .get(&link)
                    .and_then(|item| item.object.clone());
                let agreed = shared.is_none() || cluster.synced_object(source, &link) == shared;
                cluster.sources[source]
                    .remote_mut()
                    .remove("inbox", handle.as_str());
                // NOTE: a delete of a body the server never held is
                // overtaken by the edit it missed, so it stands only when
                // the server was up to date; a local delete already staged
                // stands regardless
                ledger.entry(link).or_default().removed |= agreed;
            }
            HubOp::Add(s, n) => {
                authored += 1;
                let source = s % SOURCES;
                let link = PimdirLinkId::from(format!("new-{authored}"));
                let body = format!("authored-{n}-{authored}").into_bytes();
                let object = PimdirObject {
                    hash: hash(&body),
                    size: body.len(),
                };
                let staged = cluster.sources[source].mutate(
                    "inbox",
                    PimdirMutation::Add {
                        handle: PimdirHandle::from(format!("tmp-{authored}")),
                        link_id: link.clone(),
                        flags: PimdirFlags::default(),
                        object: object.clone(),
                        body,
                        summary: None,
                        sort_key: Default::default(),
                    },
                );
                if staged.is_ok() {
                    let owed = ledger.entry(link).or_default();
                    owed.body = Some(object.hash);
                    owed.author = Some(source);
                    owed.agree(source);
                }
            }
            HubOp::Bump(s) => {
                let source = s % SOURCES;
                bumps += 1;

                // NOTE: only spine rows are rebuilt and fold the shared
                // body back; a pending create stays put and moves no
                // agreement point
                let carried: Vec<PimdirLinkId> = cluster
                    .links()
                    .into_iter()
                    .filter(|link| live(&cluster, link) && cluster.synced(source, link))
                    .collect();

                cluster.sources[source]
                    .remote_mut()
                    .renumber("inbox", bumps);
                let _ = cluster.sources[source].rekey("inbox");

                for link in carried {
                    ledger.entry(link).or_default().agree(source);
                }
            }
            HubOp::Sync(s) => {
                let source = s % SOURCES;
                cluster.sync(source);
                let live: Vec<PimdirLinkId> = cluster
                    .links()
                    .into_iter()
                    .filter(|link| live(&cluster, link))
                    .collect();
                for link in live {
                    ledger.entry(link).or_default().agree(source);
                }
            }
        }
    }

    cluster.quiesce()?;

    let shared = cluster.hub();
    for (link, item) in &shared.items {
        if item.deleted {
            continue;
        }
        for (source, binding) in &item.sources {
            let source: usize = source.as_str()[1..].parse().expect("a seeded source id");
            prop_assert_eq!(
                cluster.server_object(source, &binding.handle),
                item.object.clone(),
                "s{} diverges from the shared body of {:?}",
                source,
                link,
            );
            prop_assert_eq!(
                cluster.server_flags(source, &binding.handle),
                Some(item.flags.clone()),
                "s{} diverges from the shared flags of {:?}",
                source,
                link,
            );
        }
    }

    for (link, owed) in &ledger {
        let item = shared.items.get(link);
        if owed.removed {
            prop_assert!(
                item.is_none_or(|item| item.deleted),
                "the delete of {link:?} was undone: {item:?}",
            );
            continue;
        }
        if owed.diverged {
            prop_assert!(
                owed.resolved || item.is_none_or(|item| item.conflicted),
                "the divergence on {link:?} was resolved by nobody: {item:?}",
            );
            continue;
        }
        let Some(staged) = &owed.body else {
            continue;
        };
        let item = item.expect("the edited item is still hubbed");
        prop_assert!(
            !item.deleted,
            "the edit staged on {link:?} did not beat the delete: {item:?}",
        );
        prop_assert_eq!(
            item.object.as_ref(),
            Some(staged),
            "the body staged on {:?} never became the shared one",
            link,
        );
    }

    for source in 0..SOURCES {
        let report = cluster.sources[source]
            .sync("inbox", PimdirSyncOptions::default())
            .expect("a quiescent sync");
        prop_assert_eq!(
            report,
            PimdirSyncReport::default(),
            "s{} is not settled",
            source,
        );
    }
    Ok(())
}

proptest! {
    /// The sources converge on one body per item, whatever the ops.
    ///
    /// No source is read as diverging from itself, a genuine divergence
    /// between two sources is reported rather than resolved, and no
    /// staged body goes missing.
    #[test]
    fn hub_interleavings_converge_across_sources(
        ops in proptest::collection::vec(arb_hub_op(), 0..25),
    ) {
        check_hub_model(ops)?;
    }
}

/// A rebuild leaves a pending create where it stands.
///
/// It folds nothing back for that item, so the source has still never
/// seen what the hub shares for it: its next edit is a divergence, and
/// the hub reports one.
#[test]
fn a_bump_leaves_a_pending_create_where_it_stands() {
    check_hub_model(vec![
        HubOp::ServerAdd(0, 0),
        HubOp::Add(16006928215587417568, 0),
        HubOp::Sync(9611068653841254363),
        HubOp::ServerAdd(2130484249608440551, 0),
        HubOp::Sync(17313447479995814678),
        HubOp::Edit(1380695254562016356, 822504535492686069, 0),
        HubOp::Add(0, 0),
        HubOp::Bump(684435770336584210),
        HubOp::Edit(6237312104416555141, 9826492742111347426, 1),
    ])
    .unwrap();
}

/// A rebuild resurrecting a shared body as a pending create agrees with it.
///
/// The member is gone from the source it was bound to, and the row
/// resurrected holds the body the hub shares: the source has seen it, so
/// its next edit is a fast-forward rather than a divergence.
#[test]
fn a_bump_resurrecting_a_shared_body_agrees_with_it() {
    check_hub_model(vec![
        HubOp::ServerAdd(8558649505050902843, 0),
        HubOp::Sync(7222315568090575898),
        HubOp::ServerAdd(11050822385035315969, 0),
        HubOp::Edit(4148971933495768475, 1879388323994680017, 0),
        HubOp::ServerRemove(3602211543412188046, 11828013346212412693),
        HubOp::Bump(5310140231620834954),
        HubOp::Edit(5787938633598682036, 1166168132446768777, 1),
    ])
    .unwrap();
}
