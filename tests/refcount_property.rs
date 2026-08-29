//! The refcount invariant and the collector's two directions, as
//! properties over random operation sequences (spec §5, §7).
//!
//! `objects.refcount` is what stands between a body and the collector, so
//! a miscount is not a bookkeeping slip: too low and a live body is
//! unlinked, too high and it is held for ever. The store maintains the
//! count incrementally on the write path and recomputes it from the five
//! pointer columns in the repair, which are two independent
//! implementations of one fact. `REFCOUNT_DRIFT` compares them, so a
//! random sequence of writes, drains, purges, rekeys and collections
//! whose drift stays empty is a differential test of the write path
//! against the recomputation.
//!
//! Two further laws ride the same sequences. No live body is ever
//! collected: every hash any of the five columns names still has its row
//! and its blob. And nothing is leaked: after a collection no object row
//! is left at zero references and no blob file is left without a row.
//! Asserting one direction alone passes on a collector that takes
//! everything, or on one that takes nothing.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use io_pimdir::{PimdirError, PimdirProducer, PimdirSourceStore, PimdirStore, codec::PimdirAction};
use io_replica::{
    change::{ReplicaDropReason, ReplicaWriteOp},
    client::ReplicaStorage,
    collection::ReplicaCollectionId,
    object::{ReplicaHash, ReplicaObject},
    placement::{
        ReplicaBase, ReplicaFlags, ReplicaHandle, ReplicaLevel, ReplicaLinkId, ReplicaMeta,
        ReplicaPlacement, ReplicaSortKey, ReplicaStatus,
    },
};
use proptest::{
    prelude::*,
    strategy::ValueTree,
    test_runner::{Config, FileFailurePersistence, TestRunner},
};
use rusqlite::Connection;

/// The collections a sequence may touch.
const COLLECTIONS: [&str; 2] = ["INBOX", "Archive"];
/// The sources a sequence may act as.
const SOURCES: [&str; 2] = ["left", "right"];
/// The identities a sequence may file items under.
const LINKS: [&str; 3] = ["mid:a", "mid:b", "mid:c"];
/// The bodies a sequence may store, distinct so their hashes are.
const BODIES: [&[u8]; 4] = [b"alpha", b"beta", b"gamma", b"delta"];

/// A cutoff after every stamp SQLite can write in this decade, so the
/// time-based purge takes the whole retained set.
const AFTER_EVERYTHING: &str = "9999-01-01T00:00:00.000Z";

/// One step of a generated sequence, as indices into the constants above.
///
/// Indices rather than values so the generator stays small and shrinks
/// meaningfully: a failing case reduces to the shortest prefix that still
/// breaks an invariant.
#[derive(Clone, Debug)]
enum Op {
    /// Write one placement for one source, with the bodies it names.
    Upsert {
        collection: usize,
        source: usize,
        link: usize,
        object: Option<usize>,
        base_object: Option<Option<usize>>,
        conflict_object: Option<usize>,
        status: usize,
        level: usize,
    },
    /// Drop the handle a source holds for an identity, as a delete or as
    /// a supersede.
    Drop {
        collection: usize,
        source: usize,
        link: usize,
        deleted: bool,
    },
    /// Rebuild one identity's handle space: the licensed rebind, through
    /// the write that bumps the collection generation.
    Rekey {
        collection: usize,
        source: usize,
        link: usize,
        object: Option<usize>,
    },
    /// Append one action to the queue as a producer, body first.
    ///
    /// `kind` picks among the format's six action kinds and an
    /// owner-defined intent this store cannot apply; `pick` chooses which
    /// stored item the kinds addressing one act on.
    Enqueue {
        collection: usize,
        source: usize,
        link: usize,
        object: Option<usize>,
        kind: usize,
        pick: usize,
    },
    /// Write three placements of one collection in one batch, so the
    /// per-hash refcount delta is computed over a diff naming several
    /// items rather than one.
    Batch {
        collection: usize,
        source: usize,
        placements: Vec<(usize, Option<usize>)>,
    },
    /// Drop an identity from every source at once, which is what leaves
    /// an item held by nobody and retires it (spec §11).
    Retire { collection: usize, link: usize },
    /// Apply a collection's pending actions as one source.
    Drain { collection: usize, source: usize },
    /// Cancel one queued or parked row.
    CancelAction { pick: usize },
    /// Purge one retained item.
    Purge { collection: usize, pick: usize },
    /// Purge every retained item, store-wide.
    PurgeAll,
    /// Run the collector.
    Collect,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        // NOTE: weighted towards the write path, which is what moves a
        // refcount; the reclaiming verbs need something to reclaim, so
        // over-generating them empties the store and the properties pass
        // over nothing. `interesting_states_are_reached` is what keeps
        // this honest.
        8 => (
            0..COLLECTIONS.len(),
            0..SOURCES.len(),
            0..LINKS.len(),
            proptest::option::of(0..BODIES.len()),
            proptest::option::of(proptest::option::of(0..BODIES.len())),
            proptest::option::of(0..BODIES.len()),
            0usize..5,
            0usize..3,
        )
            .prop_map(
                |(
                    collection,
                    source,
                    link,
                    object,
                    base_object,
                    conflict_object,
                    status,
                    level,
                )| Op::Upsert {
                    collection,
                    source,
                    link,
                    object,
                    base_object,
                    conflict_object,
                    status,
                    level,
                }
            ),
        2 => (
            0..COLLECTIONS.len(),
            0..SOURCES.len(),
            0..LINKS.len(),
            any::<bool>()
        )
            .prop_map(|(collection, source, link, deleted)| Op::Drop {
                collection,
                source,
                link,
                deleted
            }),
        1 => (
            0..COLLECTIONS.len(),
            0..SOURCES.len(),
            0..LINKS.len(),
            proptest::option::of(0..BODIES.len())
        )
            .prop_map(|(collection, source, link, object)| Op::Rekey {
                collection,
                source,
                link,
                object
            }),
        4 => (
            0..COLLECTIONS.len(),
            0..SOURCES.len(),
            0..LINKS.len(),
            proptest::option::of(0..BODIES.len()),
            0usize..7,
            0usize..4,
        )
            .prop_map(
                |(collection, source, link, object, kind, pick)| Op::Enqueue {
                    collection,
                    source,
                    link,
                    object,
                    kind,
                    pick,
                }
            ),
        3 => (
            0..COLLECTIONS.len(),
            0..SOURCES.len(),
            proptest::collection::vec(
                (0..LINKS.len(), proptest::option::of(0..BODIES.len())),
                1..4,
            ),
        )
            .prop_map(|(collection, source, placements)| Op::Batch {
                collection,
                source,
                placements,
            }),
        3 => (0..COLLECTIONS.len(), 0..LINKS.len())
            .prop_map(|(collection, link)| Op::Retire { collection, link }),
        3 => (0..COLLECTIONS.len(), 0..SOURCES.len())
            .prop_map(|(collection, source)| Op::Drain { collection, source }),
        1 => (0usize..4).prop_map(|pick| Op::CancelAction { pick }),
        2 => (0..COLLECTIONS.len(), 0usize..4)
            .prop_map(|(collection, pick)| Op::Purge { collection, pick }),
        1 => Just(Op::PurgeAll),
        3 => Just(Op::Collect),
    ]
}

fn sequence() -> impl Strategy<Value = Vec<Op>> {
    proptest::collection::vec(op(), 1..24)
}

/// What a sequence reached, counted so a vacuous generator is visible.
///
/// A property over sequences that never conflict, never collect anything
/// and never purge anything passes for the wrong reason, and nothing in
/// the assertion says so.
#[derive(Clone, Copy, Debug, Default)]
struct Reached {
    /// A binding was written with its conflict flag set.
    conflict_set: usize,
    /// A binding that had been conflicted stopped being so.
    conflict_resolved: usize,
    /// An item was left carrying the cross-source divergence (spec §10),
    /// which is the other conflict axis and the other conflict pin.
    item_conflict_set: usize,
    /// A body whose only reference was an item's `conflict_object`: the
    /// state in which a miscount on that limb loses the diverging body.
    item_conflict_only_pin: usize,
    /// A collection dropped at least one object row or blob.
    collected_something: usize,
    /// A purge removed at least one retained item.
    purged_something: usize,
    /// An item was retired by its last binding going.
    retained_something: usize,
    /// A queue row was applied by a drain.
    drain_applied: usize,
    /// A write was refused as a rebind, which is a legitimate outcome.
    rebind_refused: usize,
}

impl Reached {
    fn merge(&mut self, other: Reached) {
        self.conflict_set += other.conflict_set;
        self.conflict_resolved += other.conflict_resolved;
        self.item_conflict_set += other.item_conflict_set;
        self.item_conflict_only_pin += other.item_conflict_only_pin;
        self.collected_something += other.collected_something;
        self.purged_something += other.purged_something;
        self.retained_something += other.retained_something;
        self.drain_applied += other.drain_applied;
        self.rebind_refused += other.rebind_refused;
    }
}

/// A store under test, plus the handle space the model hands out.
///
/// A handle is derived from the identity and a per-binding generation
/// rather than generated, so an ordinary write never carries a second
/// handle for a binding that already holds one: that is the refused
/// rebind (spec §10), and generating it everywhere would turn most of a
/// sequence into refusals instead of writes.
struct Harness {
    dir: tempfile::TempDir,
    sources: Vec<PimdirSourceStore>,
    generations: BTreeMap<(usize, usize, usize), u32>,
    reached: Reached,
}

impl Harness {
    fn open() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut sources = Vec::new();
        for source in SOURCES {
            let store = PimdirStore::open(dir.path()).unwrap().for_source(source);
            for collection in COLLECTIONS {
                store
                    .ensure_collection(collection, "message/rfc822")
                    .unwrap();
            }
            sources.push(store);
        }

        Self {
            dir,
            sources,
            generations: BTreeMap::new(),
            reached: Reached::default(),
        }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn hash(&self, body: usize) -> ReplicaHash {
        self.sources[0].hash(BODIES[body])
    }

    fn handle(&self, collection: usize, source: usize, link: usize) -> ReplicaHandle {
        let generation = self
            .generations
            .get(&(collection, source, link))
            .copied()
            .unwrap_or(0);
        ReplicaHandle(format!("h{generation}-{}", LINKS[link]))
    }

    /// The `StoreObject` ops a placement's hashes need, bytes included, so
    /// every row this batch writes has its blob on disk first (spec §14).
    fn bodies(&self, wanted: &[Option<usize>]) -> Vec<ReplicaWriteOp> {
        let mut seen = BTreeSet::new();
        let mut ops = Vec::new();
        for body in wanted.iter().flatten() {
            if !seen.insert(*body) {
                continue;
            }
            ops.push(ReplicaWriteOp::StoreObject {
                object: ReplicaObject {
                    hash: self.hash(*body),
                    size: BODIES[*body].len(),
                },
                body: Some(BODIES[*body].to_vec()),
            });
        }
        ops
    }
}

/// Runs one op against the store, reporting what it reached.
///
/// A rebind refusal is an outcome rather than a failure (spec §10): the
/// write is refused whole and the store is left as it was, which the
/// invariants then check. Any other error fails the sequence.
fn run(harness: &mut Harness, op: &Op) -> Reached {
    let mut reached = Reached::default();

    match *op {
        Op::Upsert {
            collection,
            source,
            link,
            object,
            base_object,
            conflict_object,
            status,
            level,
        } => {
            let status = [
                ReplicaStatus::Clean,
                ReplicaStatus::Dirty,
                ReplicaStatus::Created,
                ReplicaStatus::Conflict,
                ReplicaStatus::Tombstone,
            ][status];
            let level = [ReplicaLevel::Probed, ReplicaLevel::Meta, ReplicaLevel::Full][level];
            let conflicted_before = is_conflicted(harness.path(), collection, source, link);

            let mut batch = harness.bodies(&[object, base_object.flatten(), conflict_object]);
            batch.push(ReplicaWriteOp::UpsertPlacement(ReplicaPlacement {
                collection: ReplicaCollectionId(COLLECTIONS[collection].into()),
                handle: harness.handle(collection, source, link),
                link_id: Some(ReplicaLinkId(LINKS[link].into())),
                object: object.map(|body| harness.hash(body)),
                level,
                meta: Some(ReplicaMeta(r#"{"v":1}"#.into())),
                sort_key: ReplicaSortKey(format!("k-{}", LINKS[link])),
                flags: ReplicaFlags::default(),
                status,
                conflict_revision: (status == ReplicaStatus::Conflict).then(|| "r".to_string()),
                conflict_object: conflict_object.map(|body| harness.hash(body)),
                base: base_object.map(|body| ReplicaBase {
                    flags: ReplicaFlags::default(),
                    revision: Some("r-base".into()),
                    object: body.map(|body| harness.hash(body)),
                }),
                origin: None,
            }));

            match harness.sources[source].write(batch) {
                Ok(()) => {}
                Err(PimdirError::Rebind { .. }) => {
                    reached.rebind_refused += 1;
                    return reached;
                }
                Err(err) => panic!("unexpected write error: {err}"),
            }

            let conflicted_after = is_conflicted(harness.path(), collection, source, link);
            if conflicted_after && !conflicted_before {
                reached.conflict_set += 1;
            }
            if conflicted_before && !conflicted_after {
                reached.conflict_resolved += 1;
            }
        }

        Op::Drop {
            collection,
            source,
            link,
            deleted,
        } => {
            let retained_before = retained_rows(harness.path());
            let batch = vec![ReplicaWriteOp::DropPlacement {
                collection: ReplicaCollectionId(COLLECTIONS[collection].into()),
                handle: harness.handle(collection, source, link),
                reason: match deleted {
                    true => ReplicaDropReason::Deleted,
                    false => ReplicaDropReason::Superseded,
                },
            }];
            match harness.sources[source].write(batch) {
                Ok(()) => {}
                Err(PimdirError::Rebind { .. }) => reached.rebind_refused += 1,
                Err(err) => panic!("unexpected drop error: {err}"),
            }
            reached.retained_something +=
                usize::from(retained_rows(harness.path()) > retained_before);
        }

        Op::Batch {
            collection,
            source,
            ref placements,
        } => {
            let wanted: Vec<Option<usize>> = placements.iter().map(|(_, object)| *object).collect();
            let mut batch = harness.bodies(&wanted);
            for (link, object) in placements {
                batch.push(ReplicaWriteOp::UpsertPlacement(ReplicaPlacement {
                    collection: ReplicaCollectionId(COLLECTIONS[collection].into()),
                    handle: harness.handle(collection, source, *link),
                    link_id: Some(ReplicaLinkId(LINKS[*link].into())),
                    object: object.map(|body| harness.hash(body)),
                    level: ReplicaLevel::Full,
                    meta: Some(ReplicaMeta(r#"{"v":1}"#.into())),
                    sort_key: ReplicaSortKey(format!("k-{}", LINKS[*link])),
                    flags: ReplicaFlags::default(),
                    status: ReplicaStatus::Clean,
                    conflict_revision: None,
                    conflict_object: None,
                    base: Some(ReplicaBase {
                        flags: ReplicaFlags::default(),
                        revision: None,
                        object: object.map(|body| harness.hash(body)),
                    }),
                    origin: None,
                }));
            }

            match harness.sources[source].write(batch) {
                Ok(()) => {}
                Err(PimdirError::Rebind { .. }) => reached.rebind_refused += 1,
                Err(err) => panic!("unexpected batch error: {err}"),
            }
        }

        Op::Retire { collection, link } => {
            let retained_before = retained_rows(harness.path());
            for source in 0..SOURCES.len() {
                let batch = vec![ReplicaWriteOp::DropPlacement {
                    collection: ReplicaCollectionId(COLLECTIONS[collection].into()),
                    handle: harness.handle(collection, source, link),
                    reason: ReplicaDropReason::Deleted,
                }];
                match harness.sources[source].write(batch) {
                    Ok(()) => {}
                    Err(PimdirError::Rebind { .. }) => reached.rebind_refused += 1,
                    Err(err) => panic!("unexpected retire error: {err}"),
                }
            }
            reached.retained_something +=
                usize::from(retained_rows(harness.path()) > retained_before);
        }

        Op::Rekey {
            collection,
            source,
            link,
            object,
        } => {
            let old = harness.handle(collection, source, link);
            *harness
                .generations
                .entry((collection, source, link))
                .or_insert(0) += 1;
            let new = harness.handle(collection, source, link);

            let mut batch = vec![ReplicaWriteOp::DropPlacement {
                collection: ReplicaCollectionId(COLLECTIONS[collection].into()),
                handle: old,
                reason: ReplicaDropReason::Superseded,
            }];
            batch.extend(harness.bodies(&[object]));
            batch.push(ReplicaWriteOp::UpsertPlacement(ReplicaPlacement {
                collection: ReplicaCollectionId(COLLECTIONS[collection].into()),
                handle: new,
                link_id: Some(ReplicaLinkId(LINKS[link].into())),
                object: object.map(|body| harness.hash(body)),
                level: ReplicaLevel::Full,
                meta: Some(ReplicaMeta(r#"{"v":1}"#.into())),
                sort_key: ReplicaSortKey(format!("k-{}", LINKS[link])),
                flags: ReplicaFlags::default(),
                status: ReplicaStatus::Clean,
                conflict_revision: None,
                conflict_object: None,
                base: Some(ReplicaBase {
                    flags: ReplicaFlags::default(),
                    revision: None,
                    object: object.map(|body| harness.hash(body)),
                }),
                origin: None,
            }));

            match harness.sources[source].write_rekeyed(COLLECTIONS[collection], batch) {
                Ok(_) => {}
                Err(PimdirError::Rebind { .. }) => reached.rebind_refused += 1,
                Err(err) => panic!("unexpected rekey error: {err}"),
            }
        }

        Op::Enqueue {
            collection,
            source,
            link,
            object,
            kind,
            pick,
        } => {
            // NOTE: a producer writes the body before the row that pins
            // it (spec §15.1), so it is staged here and the enqueue is
            // what takes the reference.
            let hash = object.map(|body| harness.hash(body));
            let size = object.map(|body| {
                let blobs = harness.sources[0].blobs();
                let mut writer = blobs.writer().unwrap();
                std::io::Write::write_all(&mut writer, BODIES[body]).unwrap();
                writer.commit(&harness.hash(body)).unwrap()
            });

            let seqs: Vec<i64> = harness.sources[0]
                .list_items(COLLECTIONS[collection], None, 64)
                .unwrap()
                .into_iter()
                .map(|item| item.seq)
                .collect();
            let seq = seqs.get(pick % seqs.len().max(1)).copied();
            let elsewhere = ReplicaCollectionId(COLLECTIONS[(collection + 1) % 2].into());

            let action = match (kind, seq) {
                (1, Some(seq)) => Some(PimdirAction::SetFlags {
                    seq,
                    flags: ReplicaFlags::from_iter(["\\Seen"]),
                }),
                (2, Some(seq)) => Some(PimdirAction::Remove { seq }),
                (3, Some(seq)) => Some(PimdirAction::Move { seq, to: elsewhere }),
                (4, Some(seq)) => Some(PimdirAction::Copy { seq, to: elsewhere }),
                (5, Some(seq)) => hash.clone().map(|object| PimdirAction::Update {
                    seq,
                    object,
                    meta: Some(ReplicaMeta(r#"{"v":1}"#.into())),
                }),
                // NOTE: an owner-defined intent this store cannot apply.
                // Its drain skips the row, which must leave the pin the
                // enqueue took exactly where it was.
                (6, _) => Some(PimdirAction::Unknown {
                    kind: "submit".into(),
                    payload: r#"{"v":1}"#.into(),
                    object_hash: hash.clone(),
                }),
                _ => Some(PimdirAction::Add {
                    link_id: Some(ReplicaLinkId(LINKS[link].into())),
                    flags: ReplicaFlags::default(),
                    object: hash.clone(),
                    meta: Some(ReplicaMeta(r#"{"v":1}"#.into())),
                    handle: Some(harness.handle(collection, source, link)),
                }),
            };

            if let Some(action) = action {
                let mut producer = PimdirProducer::open(harness.path(), "proptest").unwrap();
                producer
                    .enqueue(
                        COLLECTIONS[collection],
                        &action,
                        size,
                        "2026-08-29T00:00:00.000Z",
                    )
                    .unwrap();
            }
        }

        Op::Drain { collection, source } => {
            match harness.sources[source].drain_collection(COLLECTIONS[collection]) {
                Ok(report) => reached.drain_applied += usize::from(report.applied > 0),
                Err(PimdirError::Rebind { .. }) => reached.rebind_refused += 1,
                Err(err) => panic!("unexpected drain error: {err}"),
            }
        }

        Op::CancelAction { pick } => {
            let mut ids: Vec<i64> = Vec::new();
            for collection in COLLECTIONS {
                ids.extend(
                    harness.sources[0]
                        .pending_actions(collection)
                        .unwrap()
                        .into_iter()
                        .map(|action| action.id),
                );
            }
            ids.extend(
                harness.sources[0]
                    .parked_actions()
                    .unwrap()
                    .into_iter()
                    .map(|action| action.id),
            );
            if !ids.is_empty() {
                harness.sources[0]
                    .drop_action(ids[pick % ids.len()])
                    .unwrap();
            }
        }

        Op::Purge { collection, pick } => {
            let id = ReplicaCollectionId(COLLECTIONS[collection].into());
            let retained = harness.sources[0].list_retained(&id, Some(0), 64).unwrap();
            if !retained.is_empty() {
                let seq = retained[pick % retained.len()].seq;
                if harness.sources[0].purge(&id, seq).unwrap() {
                    reached.purged_something += 1;
                }
            }
        }

        Op::PurgeAll => {
            let report = harness.sources[0]
                .purge_retained_before(AFTER_EVERYTHING)
                .unwrap();
            reached.purged_something += usize::from(report.items > 0);
        }

        Op::Collect => {
            let report = harness.sources[0].collect_garbage().unwrap();
            reached.collected_something += usize::from(report.objects > 0 || report.blobs > 0);
        }
    }

    let (item_conflicted, alone) = item_conflict_state(harness.path());
    reached.item_conflict_set += usize::from(item_conflicted);
    reached.item_conflict_only_pin += usize::from(alone);
    reached
}

/// Opens a second connection on the store's database, so a test can ask
/// the questions the crate's read surface does not expose.
fn peek(dir: &Path) -> Connection {
    Connection::open(dir.join("pimdir.db")).unwrap()
}

/// The hashes the five pointer columns name (spec §5), which is exactly
/// the set the collector must not touch.
fn reachable(dir: &Path) -> BTreeSet<String> {
    let conn = peek(dir);
    let mut stmt = conn
        .prepare(
            "SELECT object_hash FROM items WHERE object_hash IS NOT NULL \
             UNION SELECT conflict_object FROM items WHERE conflict_object IS NOT NULL \
             UNION SELECT base_object FROM bindings WHERE base_object IS NOT NULL \
             UNION SELECT conflict_object FROM bindings WHERE conflict_object IS NOT NULL \
             UNION SELECT object_hash FROM queue WHERE object_hash IS NOT NULL",
        )
        .unwrap();
    stmt.query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Every hash the object index holds, with the count its pointers justify.
fn indexed(dir: &Path) -> BTreeMap<String, i64> {
    let conn = peek(dir);
    let mut stmt = conn
        .prepare(
            "WITH refs(hash) AS ( \
               SELECT object_hash FROM items WHERE object_hash IS NOT NULL \
               UNION ALL SELECT conflict_object FROM items WHERE conflict_object IS NOT NULL \
               UNION ALL SELECT base_object FROM bindings WHERE base_object IS NOT NULL \
               UNION ALL SELECT conflict_object FROM bindings WHERE conflict_object IS NOT NULL \
               UNION ALL SELECT object_hash FROM queue WHERE object_hash IS NOT NULL \
             ) \
             SELECT o.hash, (SELECT count(*) FROM refs WHERE refs.hash = o.hash) FROM objects o",
        )
        .unwrap();
    stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

/// Whether any item currently carries the cross-source divergence (spec
/// §10), and whether any body is held by that column and nothing else.
///
/// The second is the state a miscount on the `items.conflict_object` limb
/// destroys: with no other pointer at the hash the body sits at refcount
/// zero from the moment it lands, and the collector is licensed to take
/// it. A sequence that never reaches it proves nothing about that limb.
fn item_conflict_state(dir: &Path) -> (bool, bool) {
    let conn = peek(dir);
    let conflicted: i64 = conn
        .query_row(
            "SELECT count(*) FROM items WHERE conflicted = 1 AND conflict_object IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let alone: i64 = conn
        .query_row(
            "SELECT count(*) FROM items i WHERE i.conflict_object IS NOT NULL \
               AND NOT EXISTS (SELECT 1 FROM items x WHERE x.object_hash = i.conflict_object) \
               AND NOT EXISTS (SELECT 1 FROM bindings b \
                 WHERE b.base_object = i.conflict_object \
                    OR b.conflict_object = i.conflict_object) \
               AND NOT EXISTS (SELECT 1 FROM queue q WHERE q.object_hash = i.conflict_object)",
            [],
            |row| row.get(0),
        )
        .unwrap();

    (conflicted > 0, alone > 0)
}

/// Whether one source's binding of one identity is currently conflicted.
fn is_conflicted(dir: &Path, collection: usize, source: usize, link: usize) -> bool {
    peek(dir)
        .query_row(
            "SELECT conflicted FROM bindings \
             WHERE collection = ?1 AND link_id = ?2 AND source = ?3",
            rusqlite::params![COLLECTIONS[collection], LINKS[link], SOURCES[source]],
            |row| row.get::<_, i64>(0),
        )
        .map(|conflicted| conflicted != 0)
        .unwrap_or(false)
}

fn retained_rows(dir: &Path) -> usize {
    peek(dir)
        .query_row(
            "SELECT count(*) FROM items WHERE retained_at IS NOT NULL",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap() as usize
}

fn blob_path(dir: &Path, hash: &str) -> PathBuf {
    dir.join("objects")
        .join(&hash[0..2])
        .join(&hash[2..4])
        .join(hash)
}

/// Every body the blob tree holds, by name.
fn blobs_on_disk(dir: &Path) -> BTreeSet<String> {
    fn walk(dir: &Path, found: &mut BTreeSet<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.map(Result::unwrap) {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if entry.metadata().unwrap().is_dir() {
                walk(&entry.path(), found);
            } else {
                found.insert(name);
            }
        }
    }

    let mut found = BTreeSet::new();
    walk(&dir.join("objects"), &mut found);
    found
}

/// The invariants that hold after every operation, whatever it was.
///
/// Failing with the op that broke them rather than at the end of the
/// sequence: a drift introduced by one write and observed twenty writes
/// later names the wrong statement.
fn check_always(harness: &Harness, step: usize, op: &Op) {
    let dir = harness.path();

    // The differential half: the incremental count the write path keeps
    // and the recomputation over the five pointer columns are two
    // implementations of one fact, so a disagreement means one is wrong.
    let drift = harness.sources[0].refcount_drift().unwrap();
    assert!(
        drift.is_empty(),
        "step {step} ({op:?}) left refcount drift: {drift:?}"
    );

    // No live body is missing: every hash a pointer column names has its
    // row and its bytes. The row is the foreign keys' business, the
    // bytes are the collector's.
    let indexed = indexed(dir);
    for hash in reachable(dir) {
        assert!(
            indexed.contains_key(&hash),
            "step {step} ({op:?}): {hash} is referenced with no object row"
        );
        assert!(
            blob_path(dir, &hash).is_file(),
            "step {step} ({op:?}): {hash} is referenced with no blob"
        );
    }

    // A committed row never points at a body that is not there (spec
    // §14): every batch writes its bodies before the transaction that
    // indexes them, so an indexed hash always has its file.
    let on_disk = blobs_on_disk(dir);
    for hash in indexed.keys() {
        assert!(
            on_disk.contains(hash),
            "step {step} ({op:?}): object row {hash} has no blob"
        );
    }
}

/// The invariants a collection settles, in both directions.
///
/// Only after a `Collect`: between collections an unreferenced object is
/// unreferenced and not deleted (spec §5), so its row and its bytes are
/// expected to still be there.
fn check_after_collect(harness: &Harness, step: usize) {
    let dir = harness.path();

    for (hash, references) in indexed(dir) {
        assert!(
            references > 0,
            "step {step}: the collector left {hash} at {references} references"
        );
    }

    let indexed: BTreeSet<String> = indexed(dir).into_keys().collect();
    for hash in blobs_on_disk(dir) {
        assert!(
            indexed.contains(&hash),
            "step {step}: the collector left blob {hash} with no object row"
        );
    }
}

proptest! {
    #![proptest_config(Config {
        cases: 96,
        // NOTE: an integration test is its own crate root, so proptest's
        // default source-relative persistence finds no lib.rs and drops
        // the seed of a failure on the floor. Named here, a case that
        // fails once is replayed by every later run.
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/refcount_property.proptest-regressions",
        ))),
        ..Config::default()
    })]

    /// The core property: no sequence of store operations leaves the
    /// incremental refcount disagreeing with the recomputation, and no
    /// collection takes a body a pointer column still names.
    #[test]
    fn a_sequence_keeps_the_refcount_invariant(ops in sequence()) {
        let mut harness = Harness::open();
        check_always(&harness, 0, &Op::Collect);

        for (step, op) in ops.iter().enumerate() {
            let reached = run(&mut harness, op);
            harness.reached.merge(reached);
            check_always(&harness, step + 1, op);
            if matches!(op, Op::Collect) {
                check_after_collect(&harness, step + 1);
            }
        }

        // A final collection is where the two directions meet: everything
        // still referenced survives it, everything unreferenced goes.
        let before = reachable(harness.path());
        harness.sources[0].collect_garbage().unwrap();
        check_always(&harness, ops.len() + 1, &Op::Collect);
        check_after_collect(&harness, ops.len() + 1);
        for hash in before {
            prop_assert!(
                blob_path(harness.path(), &hash).is_file(),
                "the final collection took {hash}, which was still referenced"
            );
        }
    }
}

/// The generator is not vacuous.
///
/// Every property above is a statement about states a sequence reaches,
/// so a generator that never conflicts, never collects anything and never
/// purges anything makes all of them pass over nothing. This runs the
/// same strategy, counts the states that matter, and fails when one of
/// them is never reached. It is the guard on the guards.
#[test]
fn interesting_states_are_reached() {
    const RUNS: usize = 120;

    let mut runner = TestRunner::deterministic();
    let strategy = sequence();
    let mut total = Reached::default();

    for _ in 0..RUNS {
        let ops = strategy.new_tree(&mut runner).unwrap().current();
        let mut harness = Harness::open();
        for op in &ops {
            let reached = run(&mut harness, op);
            harness.reached.merge(reached);
        }
        total.merge(harness.reached);
    }

    eprintln!("interesting states over {RUNS} sequences: {total:?}");

    assert!(
        total.conflict_set >= RUNS / 10,
        "conflicts are barely ever set: {total:?}"
    );
    assert!(
        total.conflict_resolved >= RUNS / 20,
        "a conflict is never resolved after being set: {total:?}"
    );
    assert!(
        total.item_conflict_set >= RUNS / 10,
        "the cross-source conflict axis is never reached: {total:?}"
    );
    assert!(
        total.item_conflict_only_pin >= RUNS / 20,
        "no body is ever held by an item's conflict_object alone, so that \
         limb of the refcount is never the one keeping anything: {total:?}"
    );
    assert!(
        total.collected_something >= RUNS / 10,
        "the collector never finds anything to take: {total:?}"
    );
    assert!(
        total.purged_something >= RUNS / 20,
        "a purge never reclaims anything: {total:?}"
    );
    assert!(
        total.drain_applied >= RUNS / 20,
        "the drain never applies a queued action: {total:?}"
    );
}
