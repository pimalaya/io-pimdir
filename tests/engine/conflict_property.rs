//! # Conflict properties
//!
//! Property-based safety net over the conflict lifecycle: generated
//! edits, server edits, deletes, refused pushes and resolutions over two
//! sources sharing one store.
//!
//! Three laws are asserted: the sides converge or something is reported,
//! no body a side held is dropped in silence, and a resolution settles
//! exactly the divergence it was computed against, never one that moved
//! underneath it.

use std::collections::{BTreeMap, BTreeSet};

use io_pimdir::{
    hub::{PimdirBinding, PimdirHub, PimdirSourceId},
    mutate::PimdirMutation,
    object::{PimdirHash, PimdirObject},
    placement::{PimdirFlags, PimdirHandle, PimdirLinkId, PimdirPlacement, PimdirStatus},
    remote::PimdirTier,
    sync::{PimdirSyncOptions, PimdirSyncReport},
};
use proptest::{prelude::*, test_runner::TestCaseError};

use crate::common::{Client, MemRemote, hash};

/// Two sources, the smallest cluster where a divergence can arise.
const SOURCES: usize = 2;

/// One step of the conflict scenario.
///
/// Source and item picks are indices resolved modulo the live sets at
/// execution time, so every generated op is valid by construction and
/// shrinking stays meaningful.
#[derive(Clone, Debug)]
enum ConflictOp {
    /// Stage a local content edit on the i-th item, from the s-th source.
    Edit(usize, usize, u8),
    /// Replace the i-th item's flags from the s-th source.
    SetFlags(usize, usize, PimdirFlags),
    /// Delete the i-th item on the s-th source.
    Remove(usize, usize),
    /// A content edit on the s-th source's own server, behind the replica.
    ServerEdit(usize, usize, u8),
    /// The s-th source's server starts refusing the i-th item as an append.
    Refuse(usize, usize),
    /// Resolve the i-th item's conflict on the s-th source.
    ///
    /// The choice picks one of the three bodies the conflict holds, or a
    /// hand-merged fourth.
    Resolve(usize, usize, u8),
    /// Sync and hydrate the s-th source.
    Sync(usize),
}

/// A small flag universe, so the sets overlap and the merge has work.
fn arb_flags() -> impl Strategy<Value = PimdirFlags> {
    proptest::collection::btree_set(prop_oneof![Just("seen"), Just("flagged")], 0..3)
        .prop_map(PimdirFlags::from_iter)
}

/// Weighted toward edits and resolutions.
///
/// A conflict needs a local edit and a server edit on one item with no
/// sync between them, and an unresolved conflict is a dead end for every
/// op after it.
fn arb_conflict_op() -> impl Strategy<Value = ConflictOp> {
    prop_oneof![
        4 => (any::<usize>(), any::<usize>(), any::<u8>())
            .prop_map(|(s, i, n)| ConflictOp::Edit(s, i, n)),
        4 => (any::<usize>(), any::<usize>(), any::<u8>())
            .prop_map(|(s, i, n)| ConflictOp::ServerEdit(s, i, n)),
        3 => (any::<usize>(), any::<usize>(), any::<u8>())
            .prop_map(|(s, i, n)| ConflictOp::Resolve(s, i, n)),
        1 => (any::<usize>(), any::<usize>(), arb_flags())
            .prop_map(|(s, i, f)| ConflictOp::SetFlags(s, i, f)),
        1 => (any::<usize>(), any::<usize>()).prop_map(|(s, i)| ConflictOp::Remove(s, i)),
        1 => (any::<usize>(), any::<usize>()).prop_map(|(s, i)| ConflictOp::Refuse(s, i)),
        2 => any::<usize>().prop_map(ConflictOp::Sync),
    ]
}

/// Two sources over one store, each with its own mutable-content server.
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
    /// A pulled row carries no link id until a meta fetch resolves it,
    /// and a body the hub does not hold it cannot offer.
    fn sync(&mut self, source: usize) -> Option<PimdirSyncReport> {
        let client = &mut self.sources[source];
        let report = client.sync("inbox", PimdirSyncOptions::default()).ok()?;
        let opened = client.open("inbox").ok()?;
        let handles = opened.placements.iter().map(|p| p.handle.clone()).collect();
        let _ = client.upgrade("inbox", handles, PimdirTier::Full);

        Some(report)
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

    /// The placement a source projects for `link`.
    fn placement(&mut self, source: usize, link: &PimdirLinkId) -> Option<PimdirPlacement> {
        let opened = self.sources[source].open("inbox").ok()?;
        opened
            .placements
            .into_iter()
            .find(|p| p.link_id.as_ref() == Some(link))
    }

    /// The source's binding of `link`: its base, and whether it is stuck.
    fn binding(&self, source: usize, link: &PimdirLinkId) -> Option<PimdirBinding> {
        let hub = self.hub();
        let item = hub.items.get(link)?;
        let source = PimdirSourceId::from(format!("s{source}"));

        item.sources.get(&source).cloned()
    }

    /// The body a source last synced with its own server for `link`.
    ///
    /// An edit restating it stages nothing, so it claims nothing either.
    fn synced_object(&self, source: usize, link: &PimdirLinkId) -> Option<PimdirHash> {
        let hub = self.hub();
        let item = hub.items.get(link)?;
        let source = PimdirSourceId::from(format!("s{source}"));

        item.sources.get(&source)?.base.as_ref()?.object.clone()
    }

    /// The bytes the store holds for an object.
    fn body(&self, object: &PimdirHash) -> Option<Vec<u8>> {
        self.sources[0].storage().body(object)
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

    /// The content revision a source's server reports for `handle`.
    fn server_revision(&self, source: usize, handle: &PimdirHandle) -> Option<String> {
        self.sources[source]
            .remote()
            .items
            .get(&"inbox".into())?
            .get(handle)
            .map(|item| item.rev.to_string())
    }

    /// Whether the hub reads the item as a cross-source divergence.
    fn conflicted(&self, link: &PimdirLinkId) -> bool {
        self.hub()
            .items
            .get(link)
            .is_some_and(|item| item.conflicted)
    }

    /// Whether the hub still holds the item as live.
    fn live(&self, link: &PimdirLinkId) -> bool {
        self.hub().items.get(link).is_some_and(|item| !item.deleted)
    }

    /// Whether a body is still held anywhere.
    ///
    /// The places are the shared body, either conflict record, every
    /// source's projection and every source's server.
    fn holds(&mut self, body: &PimdirHash) -> bool {
        let recorded = self.hub().items.values().any(|item| {
            item.object.as_ref() == Some(body)
                || item.conflict_object.as_ref() == Some(body)
                || item
                    .sources
                    .values()
                    .any(|binding| binding.conflict_object.as_ref() == Some(body))
        });
        if recorded {
            return true;
        }

        for source in 0..SOURCES {
            let Ok(opened) = self.sources[source].open("inbox") else {
                continue;
            };
            let held = opened.placements.iter().any(|p| {
                p.object.as_ref() == Some(body) || p.conflict_object.as_ref() == Some(body)
            });
            if held {
                return true;
            }
            let handles: Vec<PimdirHandle> = self.sources[source]
                .remote()
                .handles("inbox")
                .into_iter()
                .collect();
            if handles
                .iter()
                .any(|handle| self.server_object(source, handle).as_ref() == Some(body))
            {
                return true;
            }
        }

        false
    }
}

fn nth<T: Clone>(values: &[T], i: usize) -> Option<T> {
    match values.is_empty() {
        true => None,
        false => values.get(i % values.len()).cloned(),
    }
}

/// The object and bytes of a generated body.
fn object_of(body: &[u8]) -> PimdirObject {
    PimdirObject {
        hash: hash(body),
        size: body.len(),
    }
}

/// What the ops asked of one item.
#[derive(Default)]
struct Owed {
    /// The last body a local edit staged.
    ///
    /// `None` once a server-side edit spoke after it or a later body
    /// superseded the claim.
    body: Option<PimdirHash>,
    /// Set by a delete.
    removed: bool,
    /// Sources whose own server changed the item, not yet folded in.
    ///
    /// A body staged while one is pending is owed by nobody: the fold
    /// decides between the two, and which way is the conflict axis.
    outstanding: BTreeSet<usize>,
}

/// Records that `source` folded its server's state in.
fn folded(ledger: &mut BTreeMap<PimdirLinkId, Owed>, source: usize) {
    for owed in ledger.values_mut() {
        owed.outstanding.remove(&source);
    }
}

/// Resolves a conflict on `source` and asserts the resolution law.
///
/// The base left must be exactly the observed remote state, and the sync
/// that follows must settle exactly that divergence: one the remote moved
/// past since is a fresh divergence to report, never a state to overwrite.
fn resolve(
    cluster: &mut Cluster,
    source: usize,
    link: &PimdirLinkId,
    choice: u8,
) -> Result<bool, TestCaseError> {
    let Some(placement) = cluster.placement(source, link) else {
        return Ok(false);
    };
    if placement.status != PimdirStatus::Conflict {
        return Ok(false);
    }

    let handle = placement.handle.clone();
    let observed = placement.conflict_revision.clone();
    let diverging = placement.conflict_object.clone();
    let merged = format!("merged-{choice}-{}", link.as_str()).into_bytes();
    let ancestor = placement.base.as_ref().and_then(|b| b.object.clone());

    let picked = match choice % 4 {
        0 => ancestor.and_then(|object| cluster.body(&object)),
        1 => placement.object.as_ref().and_then(|o| cluster.body(o)),
        2 => diverging.as_ref().and_then(|o| cluster.body(o)),
        _ => None,
    };
    let body = picked.unwrap_or(merged);
    let object = object_of(&body);

    let staged = cluster.sources[source].mutate(
        "inbox",
        PimdirMutation::Edit {
            handle: handle.clone(),
            object: object.clone(),
            body,
            summary: None,
            sort_key: None,
        },
    );
    if staged.is_err() {
        return Ok(false);
    }

    let binding = cluster
        .binding(source, link)
        .expect("the resolved item is still bound");
    prop_assert!(
        !binding.conflicted,
        "the resolution of {link:?} left s{source} conflicted: {binding:?}",
    );
    prop_assert_eq!(
        binding.base.as_ref().and_then(|b| b.revision.clone()),
        observed.clone(),
        "the base of the resolution is not the revision it was computed against",
    );
    prop_assert_eq!(
        binding.base.as_ref().and_then(|b| b.object.clone()),
        diverging.clone(),
        "the base of the resolution is not the body it was computed against",
    );

    // NOTE: read before the sync, since the push moves the revision itself
    let held = cluster.server_object(source, &handle);
    let current = cluster.server_revision(source, &handle);
    let (Some(observed), Some(current)) = (observed, current) else {
        cluster.sync(source);
        return Ok(true);
    };

    cluster.sync(source);

    if !cluster.live(link) {
        return Ok(true);
    }
    let Some(binding) = cluster.binding(source, link) else {
        return Ok(true);
    };

    if current != observed {
        // NOTE: the remote moved under the decision: reporting it anew or
        // adopting what it holds now is fine, overwriting it is not
        prop_assert_eq!(
            cluster.server_object(source, &handle),
            held,
            "a resolution overwrote a remote edit nobody has seen on {:?}",
            link,
        );
        if diverging.as_ref() != Some(&object.hash) {
            prop_assert!(
                binding.conflicted || cluster.conflicted(link),
                "the divergence that moved under the resolution of {link:?} went unreported",
            );
        }
        return Ok(true);
    }

    if !cluster.conflicted(link) {
        prop_assert_eq!(
            cluster.server_object(source, &binding.handle),
            Some(object.hash.clone()),
            "the resolution of {:?} never reached s{}",
            link,
            source,
        );
    }

    Ok(true)
}

/// Runs the conflict scenario, then the convergence and no-loss laws.
fn check_conflict_model(ops: Vec<ConflictOp>) -> Result<(), TestCaseError> {
    let mut cluster = Cluster::new();
    cluster.quiesce()?;

    let mut ledger: BTreeMap<PimdirLinkId, Owed> = BTreeMap::new();

    for op in ops {
        match op {
            ConflictOp::Edit(s, i, n) => {
                let source = s % SOURCES;
                let Some(link) = nth(&cluster.links(), i) else {
                    continue;
                };
                let Some(placement) = cluster.placement(source, &link) else {
                    continue;
                };
                let body = format!("edit-{n}-{}", link.as_str()).into_bytes();
                let object = object_of(&body);
                let stages = cluster.synced_object(source, &link) != Some(object.hash.clone());

                let staged = cluster.sources[source].mutate(
                    "inbox",
                    PimdirMutation::Edit {
                        handle: placement.handle,
                        object: object.clone(),
                        body,
                        summary: None,
                        sort_key: None,
                    },
                );
                if staged.is_ok() && stages {
                    let owed = ledger.entry(link).or_default();
                    let contested = !owed.outstanding.is_empty();
                    owed.body = (!contested).then_some(object.hash);
                    owed.removed = false;
                }
            }
            ConflictOp::SetFlags(s, i, flags) => {
                let source = s % SOURCES;
                let Some(link) = nth(&cluster.links(), i) else {
                    continue;
                };
                let Some(placement) = cluster.placement(source, &link) else {
                    continue;
                };
                let _ = cluster.sources[source].mutate(
                    "inbox",
                    PimdirMutation::SetFlags {
                        handle: placement.handle,
                        flags,
                    },
                );
            }
            ConflictOp::Remove(s, i) => {
                let source = s % SOURCES;
                let Some(link) = nth(&cluster.links(), i) else {
                    continue;
                };
                let Some(placement) = cluster.placement(source, &link) else {
                    continue;
                };
                let staged = cluster.sources[source]
                    .mutate("inbox", PimdirMutation::Remove(placement.handle));
                if staged.is_ok() {
                    ledger.entry(link).or_default().removed = true;
                }
            }
            ConflictOp::ServerEdit(s, i, n) => {
                let source = s % SOURCES;
                let Some(link) = nth(&cluster.links(), i) else {
                    continue;
                };
                let Some(binding) = cluster.binding(source, &link) else {
                    continue;
                };
                if cluster.server_object(source, &binding.handle).is_none() {
                    continue;
                }
                let body = format!("server-{n}-{}", link.as_str()).into_bytes();
                cluster.sources[source]
                    .remote_mut()
                    .edit("inbox", binding.handle.as_str(), &body);
                let owed = ledger.entry(link).or_default();
                owed.body = None;
                owed.outstanding.insert(source);
            }
            ConflictOp::Refuse(s, i) => {
                let source = s % SOURCES;
                let Some(link) = nth(&cluster.links(), i) else {
                    continue;
                };
                cluster.sources[source]
                    .remote_mut()
                    .refused_appends
                    .insert(link);
            }
            ConflictOp::Resolve(s, i, choice) => {
                let source = s % SOURCES;
                let Some(link) = nth(&cluster.links(), i) else {
                    continue;
                };
                if resolve(&mut cluster, source, &link, choice)? {
                    ledger.entry(link).or_default().body = None;
                    folded(&mut ledger, source);
                }
            }
            ConflictOp::Sync(s) => {
                let source = s % SOURCES;
                cluster.sync(source);
                folded(&mut ledger, source);
            }
        }
    }

    cluster.quiesce()?;

    let mut reports = Vec::new();
    for source in 0..SOURCES {
        reports.push(cluster.sync(source));
    }

    let shared = cluster.hub();
    for (link, item) in &shared.items {
        if item.deleted {
            continue;
        }
        for (source, binding) in &item.sources {
            let source: usize = source.as_str()[1..].parse().expect("a seeded source id");
            let converged = cluster.server_object(source, &binding.handle) == item.object;
            let reported = item.conflicted
                || binding.conflicted
                || reports[source] != Some(PimdirSyncReport::default());
            prop_assert!(
                converged || reported,
                "s{source} silently diverges from the shared body of {link:?}: {binding:?}",
            );
        }
    }

    let owed: Vec<(PimdirLinkId, PimdirHash)> = ledger
        .iter()
        .filter(|(_, owed)| !owed.removed)
        .filter_map(|(link, owed)| Some((link.clone(), owed.body.clone()?)))
        .collect();
    for (link, body) in owed {
        prop_assert!(
            cluster.holds(&body),
            "the body staged on {link:?} was dropped by nobody's decision",
        );
    }

    Ok(())
}

proptest! {
    /// Sources converge on the shared body or the disagreement is reported.
    ///
    /// No staged body is dropped without a later decision taking its
    /// place, and every resolution settles exactly the divergence it was
    /// computed against.
    #[test]
    fn conflict_interleavings_are_reported_resolved_or_kept(
        ops in proptest::collection::vec(arb_conflict_op(), 0..20),
    ) {
        check_conflict_model(ops)?;
    }
}

/// A delete on one source settles nothing between another and its server.
///
/// The divergence has to survive the tombstone the hub projects, or the
/// source reads as in sync at a revision its server has moved past, with
/// nothing pending and an enumeration that never mentions the item again.
#[test]
fn a_delete_elsewhere_does_not_swallow_a_local_divergence() {
    check_conflict_model(vec![
        ConflictOp::ServerEdit(96784193793256460, 7818446706866281235, 0),
        ConflictOp::Edit(0, 3312690796538023945, 0),
        ConflictOp::ServerEdit(14275505265697211509, 3501573715564205099, 0),
        ConflictOp::Sync(7502546344675836157),
        ConflictOp::Remove(0, 4952521292402537855),
        ConflictOp::Edit(1726695831431374949, 303849847995653935, 0),
        ConflictOp::Sync(692029577223606944),
        ConflictOp::Resolve(13028676742246650254, 3798237393719705107, 52),
    ])
    .unwrap();
}
