//! The collections a run may skip (`settled_collections`): a collection is
//! listed while its source holds a checkpoint and a coverage there, no
//! round is open, no binding of it owes a push or holds a refusal, no live
//! item lacks it and no supersession names an item there; a run of such a
//! collection against a remote answering an empty delta under that
//! checkpoint, its coverage's scope and no full listing, then yields no
//! push, no write and no event.

#![cfg(feature = "client")]

use std::{collections::BTreeMap, path::Path};

use io_pimdir::{
    change::PimdirChange,
    client::{PimdirError, PimdirSourceStore, PimdirStore, producer::PimdirProducer},
    codec::{PimdirAction, PimdirSupersedes},
    collection::{PimdirCheckpoint, PimdirCollectionId, PimdirCoverage, PimdirCursor},
    coroutine::{PimdirArg, PimdirCoroutine, PimdirCoroutineState, PimdirYield},
    load::PimdirLoadSelection,
    mutate::PimdirMutation,
    object::PimdirObject,
    placement::{PimdirFlags, PimdirHandle, PimdirLinkId},
    remote::{
        PimdirEnumerate, PimdirEnumerated, PimdirFetchedBody, PimdirFetchedItem, PimdirPushOutcome,
        PimdirPushResult, PimdirRemote, PimdirRemoteItem, PimdirRemoteMeta, PimdirRemoteSnapshot,
        PimdirTier,
    },
    summary::{PimdirSummary, mail::PimdirMailSummary},
    sync::{PimdirSyncOptions, PimdirSyncReport},
};

const MAIL: &str = "message/rfc822";

/// How the remote answers a round.
#[derive(Clone, Copy, Default)]
enum Round {
    /// One page listing every member, with the checkpoint.
    #[default]
    Whole,
    /// One page listing every member, with no checkpoint.
    NoCheckpoint,
    /// A first page of two, then a failure.
    Cut,
}

/// A remote holding a few members per collection, each listed with its
/// body: a delta is always empty, under the checkpoint it was asked from.
#[derive(Default)]
struct Remote {
    members: BTreeMap<String, Vec<(String, String)>>,
    round: Round,
    refuse: bool,
    fetches: usize,
    pushes: usize,
}

impl Remote {
    fn holding(collections: &[(&str, &[&str])]) -> Self {
        let members = collections
            .iter()
            .map(|(collection, links)| {
                let members = links
                    .iter()
                    .enumerate()
                    .map(|(n, link)| (format!("{}", n + 1), link.to_string()))
                    .collect();
                (collection.to_string(), members)
            })
            .collect();
        Self {
            members,
            ..Default::default()
        }
    }

    fn listed(&self, collection: &PimdirCollectionId) -> Vec<PimdirRemoteItem> {
        let members = self.members.get(&collection.0).into_iter().flatten();
        members
            .map(|(handle, link)| PimdirRemoteItem {
                handle: PimdirHandle::from(handle.as_str()),
                flags: PimdirFlags::from_iter(["\\Seen"]),
                revision: None,
                meta: PimdirRemoteMeta {
                    link_id: PimdirLinkId::from(link.as_str()),
                    summary: Some(summary(link)),
                    sort_key: Default::default(),
                    body: Some(inline(link)),
                    server_object_id: None,
                },
            })
            .collect()
    }
}

fn body(link: &str) -> Vec<u8> {
    format!("Message-ID: <{link}>\r\n\r\n{link}\r\n").into_bytes()
}

fn inline(link: &str) -> PimdirFetchedBody {
    let bytes = body(link);
    PimdirFetchedBody::Inline {
        hash: io_pimdir::hash::hash(&bytes),
        bytes,
    }
}

fn summary(link: &str) -> PimdirSummary {
    PimdirSummary::Mail(PimdirMailSummary {
        subject: format!("headers:{link}"),
        ..Default::default()
    })
}

fn checkpoint() -> PimdirCheckpoint {
    PimdirCheckpoint(b"c1".to_vec())
}

impl PimdirRemote for Remote {
    type Error = String;

    fn enumerate(
        &mut self,
        collection: &PimdirCollectionId,
        request: PimdirEnumerate,
    ) -> Result<PimdirEnumerated, String> {
        if let Some(since) = request.checkpoint() {
            let mut delta = PimdirRemoteSnapshot::round(Vec::new(), Some(since.clone()));
            delta.complete = false;
            return Ok(PimdirEnumerated::Page(delta));
        }
        let items = self.listed(collection);
        Ok(PimdirEnumerated::Page(match self.round {
            Round::Whole => PimdirRemoteSnapshot::round(items, Some(checkpoint())),
            Round::NoCheckpoint => PimdirRemoteSnapshot::round(items, None),
            Round::Cut if request.cursor().is_some() => return Err("cut".into()),
            Round::Cut => PimdirRemoteSnapshot::page(
                items.into_iter().take(1).collect(),
                Some(PimdirCursor(b"p2".to_vec())),
                Some(checkpoint()),
            ),
        }))
    }

    fn fetch(
        &mut self,
        collection: &PimdirCollectionId,
        handles: Vec<PimdirHandle>,
        tier: PimdirTier,
    ) -> Result<Vec<PimdirFetchedItem>, String> {
        self.fetches += 1;
        let listed = self.listed(collection);
        Ok(listed
            .into_iter()
            .filter(|item| handles.contains(&item.handle))
            .map(|item| PimdirFetchedItem {
                summary: item.meta.summary,
                sort_key: Default::default(),
                handle: item.handle,
                body: match tier {
                    PimdirTier::Meta => None,
                    PimdirTier::Full => Some(inline(item.meta.link_id.as_str())),
                },
                link_id: item.meta.link_id,
                revision: None,
            })
            .collect())
    }

    fn push(
        &mut self,
        _collection: &PimdirCollectionId,
        changes: Vec<PimdirChange>,
    ) -> Result<Vec<PimdirPushResult>, String> {
        self.pushes += 1;
        let outcome = match self.refuse {
            true => PimdirPushOutcome::Refused {
                reason: "a rule the change breaks".into(),
            },
            false => PimdirPushOutcome::Accepted,
        };
        Ok(changes
            .iter()
            .map(|change| PimdirPushResult {
                relocated: false,
                handle: change.handle().clone(),
                outcome: outcome.clone(),
                assigned: None,
                revision: None,
            })
            .collect())
    }
}

/// The store `dir` holds, as `source`, with `inbox` and `archive`.
fn open(dir: &Path, source: &str) -> PimdirSourceStore {
    let store = PimdirStore::open(dir).unwrap().for_source(source).unwrap();
    store.ensure_collection("inbox", MAIL).unwrap();
    store.ensure_collection("archive", MAIL).unwrap();
    store
}

/// A store whose source `left` synced `inbox` and `archive` once, whole,
/// against a remote holding a few mails in each.
fn synced(dir: &Path) -> (PimdirSourceStore, Remote) {
    let mut store = open(dir, "left");
    let mut remote = Remote::holding(&[("inbox", &["a", "b", "c"]), ("archive", &["d", "e"])]);
    for collection in ["inbox", "archive"] {
        store
            .sync(collection, PimdirSyncOptions::default(), &mut remote)
            .unwrap();
    }
    (store, remote)
}

fn settled(store: &PimdirSourceStore) -> Vec<String> {
    let settled = store.settled_collections().unwrap();
    settled.into_iter().map(|(c, _)| c.0).collect()
}

/// How many bindings of `left` in `collection` owe a push, and how many
/// live items there `left` binds not: the guards beside the one a test
/// means to trip.
fn marks(dir: &Path, collection: &str) -> (i64, i64) {
    let conn = rusqlite::Connection::open(dir.join("pimdir.db")).unwrap();
    let count = |sql: &str| -> i64 { conn.query_row(sql, [collection], |r| r.get(0)).unwrap() };
    let owing = count(
        "SELECT count(*) FROM bindings WHERE collection = ?1 AND source = 'left' AND owes = 1",
    );
    let lacking = count(
        "SELECT count(*) FROM items i WHERE i.collection = ?1 AND i.lacks = 1 \
         AND NOT EXISTS (SELECT 1 FROM bindings b WHERE b.item = i.id AND b.source = 'left')",
    );
    (owing, lacking)
}

fn coverage(store: &PimdirSourceStore, collection: &str) -> PimdirCoverage {
    let selection = PimdirLoadSelection::Handles(Vec::new());
    let loaded = store.load(&collection.into(), &selection).unwrap();
    loaded.coverage.expect("a coverage")
}

/// What one run yielded besides its loads.
#[derive(Debug, Default, PartialEq)]
struct Yields {
    writes: usize,
    pushes: usize,
    fetches: usize,
    enumerations: usize,
}

/// Runs a sync of `collection` by hand, counting what it yields.
fn run(
    store: &mut PimdirSourceStore,
    remote: &mut Remote,
    collection: &str,
) -> (PimdirSyncReport, Yields) {
    let mut yields = Yields::default();
    let options = PimdirSyncOptions::default();
    let mut sync = store
        .prepare_sync(collection, options, remote.scope_bound())
        .unwrap()
        .heads(remote.heads());
    let mut arg = None;
    let report = loop {
        let yielded = match sync.resume(arg.take()) {
            PimdirCoroutineState::Complete(report) => break report.unwrap(),
            PimdirCoroutineState::Yielded(yielded) => yielded,
        };
        if let PimdirYield::WantsWrite(_) = yielded {
            yields.writes += 1;
        }
        arg = Some(match store.service(yielded).unwrap() {
            Ok(arg) => arg,
            Err(PimdirYield::WantsEnumerate {
                collection,
                request,
            }) => {
                yields.enumerations += 1;
                PimdirArg::Enumerate(remote.enumerate(&collection, request).unwrap())
            }
            Err(PimdirYield::WantsFetch {
                collection,
                handles,
                tier,
            }) => {
                yields.fetches += 1;
                PimdirArg::Fetch(remote.fetch(&collection, handles, tier).unwrap())
            }
            Err(PimdirYield::WantsPush {
                collection,
                changes,
            }) => {
                yields.pushes += 1;
                PimdirArg::Push(remote.push(&collection, changes).unwrap())
            }
            Err(_) => unreachable!("a storage yield is serviced"),
        });
    };
    (report, yields)
}

fn object_of(bytes: &[u8]) -> PimdirObject {
    PimdirObject {
        hash: io_pimdir::hash::hash(bytes),
        size: bytes.len(),
    }
}

/// A clean sync settles each collection with its coverage, and a run of a
/// settled one against an empty delta pushes, writes and reports nothing.
#[test]
fn a_settled_collection_derives_nothing_from_an_empty_delta() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, mut remote) = synced(dir.path());

    let listed = store.settled_collections().unwrap();
    let expected = vec![
        (
            PimdirCollectionId::from("archive"),
            coverage(&store, "archive"),
        ),
        (PimdirCollectionId::from("inbox"), coverage(&store, "inbox")),
    ];
    assert_eq!(listed, expected);

    for (collection, _) in listed {
        let (report, yields) = run(&mut store, &mut remote, &collection.0);
        assert_eq!(report, PimdirSyncReport::default(), "{collection:?}");
        let expected = Yields {
            enumerations: 1,
            ..Default::default()
        };
        assert_eq!(yields, expected, "{collection:?}");
    }
    assert_eq!((remote.fetches, remote.pushes), (0, 0));
    assert_eq!(marks(dir.path(), "inbox"), (0, 0));
    assert_eq!(settled(&store), ["archive", "inbox"]);
}

/// A flag edit staged and not pushed unsettles its collection alone.
#[test]
fn a_staged_flag_edit_unsettles() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, _) = synced(dir.path());
    let edit = PimdirMutation::SetFlags {
        handle: PimdirHandle::from("2"),
        flags: PimdirFlags::from_iter(["\\Seen", "\\Flagged"]),
        base: None,
    };
    store.mutate("inbox", edit).unwrap();
    assert_eq!(marks(dir.path(), "inbox"), (1, 0));
    assert_eq!(settled(&store), ["archive"]);
}

/// A create staged and not pushed unsettles its collection alone.
#[test]
fn a_staged_create_unsettles() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, _) = synced(dir.path());
    let bytes = body("new");
    let add = PimdirMutation::Add {
        link_id: PimdirLinkId::from("new"),
        flags: PimdirFlags::default(),
        object: object_of(&bytes),
        body: bytes,
        summary: None,
        sort_key: Default::default(),
    };
    store.mutate("inbox", add).unwrap();
    assert_eq!(marks(dir.path(), "inbox"), (1, 0));
    assert_eq!(settled(&store), ["archive"]);
}

/// A push the remote refused, held on its binding, unsettles.
#[test]
fn a_refusal_unsettles() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, mut remote) = synced(dir.path());
    let edit = PimdirMutation::SetFlags {
        handle: PimdirHandle::from("2"),
        flags: PimdirFlags::from_iter(["\\Seen", "\\Flagged"]),
        base: None,
    };
    store.mutate("inbox", edit).unwrap();
    remote.refuse = true;
    let report = store
        .sync("inbox", PimdirSyncOptions::default(), &mut remote)
        .unwrap();
    assert_eq!(remote.pushes, 1);
    assert_eq!(report.events.len(), 1, "{report:?}");
    let refused = store.load(&"inbox".into(), &PimdirLoadSelection::All);
    assert_eq!(refused.unwrap().refused.len(), 1);
    assert_eq!(marks(dir.path(), "inbox"), (1, 0));
    assert_eq!(settled(&store), ["archive"]);
}

/// An item a second source bound there, which this source lacks, unsettles
/// the collection for this source alone.
#[test]
fn an_item_this_source_lacks_unsettles() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = synced(dir.path());
    let mut right = open(dir.path(), "right");
    let mut other = Remote::holding(&[("inbox", &["a", "b", "c", "z"])]);
    right
        .sync("inbox", PimdirSyncOptions::default(), &mut other)
        .unwrap();
    assert_eq!(
        marks(dir.path(), "inbox"),
        (0, 1),
        "the lacking guard alone"
    );
    assert_eq!(settled(&store), ["archive"]);
}

/// A round cut after its first page of two, left open, unsettles even
/// over the checkpoint and coverage an earlier round left.
#[test]
fn an_open_round_unsettles() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, mut remote) = synced(dir.path());
    remote.round = Round::Cut;
    let options = PimdirSyncOptions {
        full: true,
        ..Default::default()
    };
    store.sync("inbox", options, &mut remote).unwrap_err();
    let loaded = store
        .load(&"inbox".into(), &PimdirLoadSelection::Handles(Vec::new()))
        .unwrap();
    assert!(loaded.round.is_some() && loaded.checkpoint.is_some());
    assert!(loaded.coverage.is_some());
    assert_eq!(marks(dir.path(), "inbox"), (0, 0), "the round guard alone");
    assert_eq!(settled(&store), ["archive"]);
}

/// A round whose last page hands no checkpoint settles nothing, nor does a
/// collection the source never synced.
#[test]
fn no_checkpoint_unsettles() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path(), "left");
    assert!(settled(&store).is_empty(), "never synced");

    let mut remote = Remote::holding(&[("inbox", &["a", "b"]), ("archive", &["d"])]);
    store
        .sync("archive", PimdirSyncOptions::default(), &mut remote)
        .unwrap();
    remote.round = Round::NoCheckpoint;
    store
        .sync("inbox", PimdirSyncOptions::default(), &mut remote)
        .unwrap();
    let loaded = store
        .load(&"inbox".into(), &PimdirLoadSelection::Handles(Vec::new()))
        .unwrap();
    assert!(loaded.checkpoint.is_none() && loaded.coverage.is_some());
    assert!(loaded.round.is_none());
    assert_eq!(
        marks(dir.path(), "inbox"),
        (0, 0),
        "the checkpoint guard alone"
    );
    assert_eq!(settled(&store), ["archive"]);
}

/// An add filed into `archive` superseding an item of `inbox` unsettles
/// both while the supersession stands.
#[test]
fn a_supersession_unsettles_both_ends() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, _) = synced(dir.path());
    let seq = store.list_items("inbox", None, 10).unwrap()[0].seq;
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let object = producer.stage_body(&body("filed")).unwrap();
    let add = PimdirAction::Add {
        link_id: Some(PimdirLinkId::from("filed")),
        flags: PimdirFlags::default(),
        object: Some(object.hash.clone()),
        supersedes: Some(PimdirSupersedes {
            collection: PimdirCollectionId::from("inbox"),
            seq,
        }),
    };
    producer.enqueue("archive", &add, Some(&object)).unwrap();
    assert_eq!(store.drain().unwrap().parked, 0);

    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    let pairs: i64 = conn
        .query_row("SELECT count(*) FROM supersessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(pairs, 1);
    assert_eq!(marks(dir.path(), "inbox"), (1, 0));
    assert_eq!(marks(dir.path(), "archive"), (1, 0));
    assert!(settled(&store).is_empty());
}

/// A store whose sync state another engine version wrote is refused by the
/// read as by a load.
#[test]
fn another_sync_version_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    drop(synced(dir.path()));
    let conn = rusqlite::Connection::open(dir.path().join("pimdir.db")).unwrap();
    conn.execute("UPDATE store_meta SET sync_version = 'sync-draft-06'", [])
        .unwrap();
    drop(conn);

    let store = PimdirStore::open(dir.path())
        .unwrap()
        .for_source("left")
        .unwrap();
    let refused = store.settled_collections().err();
    assert!(
        matches!(&refused, Some(PimdirError::SyncVersion { found }) if found == "sync-draft-06"),
        "{refused:?}"
    );
}
