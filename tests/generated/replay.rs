//! # The engine replaying generated histories
//!
//! Each history of the generator (`History::mail`, `cards`, `moves`,
//! `refusals`, `appends`, `policies`, `sources`, `scopes`) replayed through
//! io-pimdir from the store of its first step: a run answered by a scripted
//! connector from the run's own pages, fetches and outcomes, a producer's
//! rows enqueued through io-pimdir's producer and drained by its owner, a
//! remote's change left to the next run's pages. After each step the store
//! is read against the oracle of the step's model; a run's pushes, in
//! order, its events and the listing it asked for are held to the run's.
//! Beside the model, every step holds what a run must whatever the model
//! says: no row names a body the store lacks (no loss), a run the model
//! finds nothing to do in leaves every row as it stood (convergence), and
//! receipts follow the queue's append order.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::Path,
};

use io_pimdir::{
    change::{PimdirChange, PimdirChangeKind},
    client::{PimdirStore, producer::PimdirProducer, reader::PimdirReader},
    codec,
    collection::{PimdirCheckpoint, PimdirCollectionId, PimdirCursor, PimdirScope},
    hash,
    object::PimdirObject,
    placement::{PimdirFlags, PimdirHandle},
    remote::{
        PimdirEnumerate, PimdirEnumerated, PimdirFetchedBody, PimdirFetchedItem, PimdirPushOutcome,
        PimdirPushResult, PimdirRemote, PimdirRemoteItem, PimdirRemoteMeta, PimdirRemoteSnapshot,
        PimdirTier,
    },
    summary::{self, mail},
    sync::{PimdirConflictPolicy, PimdirSyncEvent, PimdirSyncOptions},
};
use pimdir_generator::{
    engine::{Answer, Event, Outcome, Push, Run},
    history::{History, Step},
    model::{Collection, Model, Scope, SourceState},
    oracle::Expectation,
    queue::RowState,
    remote::Page,
    spec::Spec,
};
use rusqlite::{Connection, types::Value};

use crate::{
    common::{Generated, SEEDS},
    crash, reader,
};

/// Finding G11 (the generator's, tooling): a `Run` states neither the
/// options its verb ran under (the source's conflict policy, the scope,
/// whether absences are accepted) nor an upgrade's handles, which a sync
/// vector's `run` states. The replay reads the scope from the model's
/// coverage, acceptance from a parked round closing, the handles from the
/// fetches, and the policies from `History::policies`, restated here.
const POLICIES: [(&str, PimdirConflictPolicy); 5] = [
    ("work/remote-wins", PimdirConflictPolicy::PreferRemote),
    ("work/local-wins", PimdirConflictPolicy::PreferLocal),
    ("work/local-unread", PimdirConflictPolicy::PreferLocal),
    ("work/keep-local", PimdirConflictPolicy::KeepLocal),
    ("work/keep-unread", PimdirConflictPolicy::KeepLocal),
];

/// Finding G12 (implementation): the body a source's `PreferLocal` or
/// `KeepLocal` pushes over, kept as a retained item (SYNC §9, "A policy's
/// loser is kept"), keeps the sort key of the item it lost to, not the one
/// its own body derives (Annex A).
const G12: &str = "G12: a policy's kept body keeps the winner's sort key";

/// Finding G13 (implementation): io-pimdir offers another source an item
/// keyed `hash:`, which SYNC §9 says is not offered, asserting no identity.
const G13: &str = "G13: a hash: key offered to another source";

/// Finding G15 (spec minor, reads two ways): one write inserting several
/// rows, a kept body and a member added by one page, draws their `seq` in
/// another order in io-pimdir than in the generator, which draws in the
/// order of the page's members; STORAGE §9.1 states no order.
const G15: &str = "G15: the order one write draws its rows' seq in";

/// Finding G16 (the generator's): the body a collection's policy keeps over
/// a joining member's leaves the binding owing it, which SYNC §5 (“a
/// candidate's change is derived before its write”) pushes by the source's
/// next run, never this one; the generator pushes it as an `Update` in the
/// same run.
const G16: &str = "G16: a fold's debt pushed in the run that folds it";

/// Finding G17 (spec minor, reads two ways): a member whose body differs from
/// its base and equals the item's, another source having folded it: the
/// generator reports `ContentChanged`, io-pimdir nothing, the item taking no
/// body it did not hold (SYNC §5, `ContentChanged` naming a body the item
/// took).
const G17: &str = "G17: ContentChanged for a body the item already holds";

/// Finding G14 (spec minor, reads two ways): a source whose binding owes a
/// change another source folded (`Dirty`, SYNC §9 propagation) and whose
/// server changed the body too: the generator reads SYNC §9's three facts
/// and records the item's divergence under the collection's policy,
/// io-pimdir reads SYNC §5's both-changed case and records the binding's
/// conflict under the source's.
const G14: &str = "G14: a propagated change meeting a server edit";

#[test]
fn every_generated_history_replays_as_its_model_states() {
    for seed in SEEDS {
        let differences = differences(seed);
        let unknown: Vec<&String> = differences
            .iter()
            .filter(|(known, _)| known.is_none())
            .map(|(_, difference)| difference)
            .collect();
        assert!(unknown.is_empty(), "seed {seed}: {unknown:#?}");
    }
}

#[test]
#[ignore = "G12 to G17: the replay findings of the generated suite"]
fn every_generated_history_replays_as_its_model_states_known_included() {
    let differences: Vec<(u64, Option<&str>, String)> = SEEDS
        .into_iter()
        .flat_map(|seed| {
            differences(seed)
                .into_iter()
                .map(move |(known, difference)| (seed, known, difference))
        })
        .collect();
    assert!(differences.is_empty(), "{differences:#?}");
}

/// Every difference the histories of `seed` replay with, each with the
/// known finding it stands under.
fn differences(seed: u64) -> Vec<(Option<&'static str>, String)> {
    histories(seed)
        .into_iter()
        .flat_map(|(name, history)| {
            replay(&history)
                .into_iter()
                .map(move |(known, text)| (known, format!("{name}: {text}")))
        })
        .collect()
}

/// Every history of the generator at `seed`, by name.
pub fn histories(seed: u64) -> [(&'static str, History); 8] {
    [
        ("mail", History::mail(seed)),
        ("cards", History::cards(seed)),
        ("moves", History::moves(seed)),
        ("refusals", History::refusals(seed)),
        ("appends", History::appends(seed)),
        ("policies", History::policies(seed)),
        ("sources", History::sources(seed)),
        ("scopes", History::scopes(seed)),
    ]
}

/// Replays `history` from the store of its first step, naming each
/// disagreement with the step it is found after. A step disagreeing leaves
/// the store rewritten from its model, so the next step starts where the
/// model's does and a finding is named once, at the step it is made.
fn replay(history: &History) -> Vec<(Option<&'static str>, String)> {
    let history = declared(history);
    let spec = Spec::load_default().unwrap();
    let first = &history.steps[0];
    let mut generated = Generated::write(Spec::load_default().unwrap(), first.model.clone());

    let mut revisions = BTreeMap::new();
    let mut differences = Vec::new();
    for (index, pair) in history.steps.windows(2).enumerate() {
        let (before, step) = (&pair[0], &pair[1]);
        let root = generated.root();
        let at = format!("step {} ({})", index + 1, step.label);
        let mut diverging = diverges(before, step).then_some(G14);
        let mut found: Vec<(Option<&'static str>, String)> = produce(&root, before, step)
            .into_iter()
            .map(|text| (None, text))
            .collect();

        let rows = crash::store_rows(&root);
        if let Some(run) = &step.run {
            let (ran, known) = replay_run(&root, before, step, run, &mut revisions);
            found.extend(ran);
            diverging = diverging.or(known);
        }
        let (_, mut read) = reader::compare(&spec, &step.model, &root, retained_sort_key);
        if !read.is_empty() {
            let renumbered = renumbered(before, step, &root);
            let (_, again) = reader::compare(&spec, &renumbered, &root, retained_sort_key);
            if again.iter().all(|(known, _)| known.is_some()) {
                for (known, _) in &mut read {
                    *known = known.or(Some(G15));
                }
            }
        }
        found.extend(read);
        for hash in unheld(&root) {
            found.push((None, format!("a row names {hash}, a body the store lacks")));
        }
        if let Some(run) = &step.run
            && run.verb == "sync"
            && run.pushes.is_empty()
            && run.events.is_empty()
            && run
                .pages
                .iter()
                .all(|page| page.items.is_empty() && page.vanished.is_empty())
            && format!("{:?}", before.model.collections) == format!("{:?}", step.model.collections)
            && crash::store_rows(&root) != rows
        {
            found.push((None, String::from("a run with nothing to do moved a row")));
        }
        if let Some(order) = out_of_order(&root) {
            found.push((None, order));
        }

        if found.is_empty() {
            continue;
        }
        differences.extend(found.into_iter().map(|(known, text)| {
            let known = known.or(diverging);
            (known, format!("{at}: {text}"))
        }));
        generated = Generated::write(Spec::load_default().unwrap(), step.model.clone());
    }
    differences
}

/// The step's model with each item the step created numbered as io-pimdir
/// numbered it, found by its key: G15's reading, the order a write draws
/// the `seq` of the rows it inserts.
fn renumbered(before: &Step, step: &Step, root: &Path) -> Model {
    let reader = PimdirReader::open(root).unwrap();
    let mut model = step.model.clone();
    for item in &mut model.items {
        if item.seq < before.model.next_seq {
            continue;
        }
        if let Some(seq) = reader
            .seq_for_link(&item.collection, &item.link_id)
            .unwrap()
        {
            item.seq = seq;
        }
    }
    model
}

/// Whether the step records an item conflict across sources the store did
/// not hold before it, G14's case.
fn diverges(before: &Step, step: &Step) -> bool {
    step.model.items.iter().any(|item| {
        item.conflicted
            && item.bindings.len() > 1
            && !before.model.items.iter().any(|held| {
                held.collection == item.collection
                    && held.link_id == item.link_id
                    && held.conflicted
            })
    })
}

/// Whether a disagreement is G12's: a retained row read alike but for its
/// sort key.
fn retained_sort_key(
    expectation: &Expectation,
    columns: &[&str],
    got: &[Vec<Value>],
    want: &[Vec<Value>],
) -> Option<&'static str> {
    let at = columns.iter().position(|column| *column == "sort_key")?;
    let alike = got.len() == want.len()
        && got.iter().zip(want).all(|(got, want)| {
            (0..got.len()).all(|column| column == at || got[column] == want[column])
        });
    (expectation.statement == "storage/read/list_retained_page" && alike).then_some(G12)
}

/// The history with every collection it ever declares declared from its
/// first step, never synced there: a declaration writes nothing but its
/// row and its sources' (STORAGE §14), which io-pimdir's owner would
/// write as the configuration it is, so a step's run finds them as the
/// model's did.
pub fn declared(history: &History) -> History {
    let last = &history.steps[history.steps.len() - 1].model;
    let mut history = history.clone();
    for step in &mut history.steps {
        for (id, collection) in &last.collections {
            step.model
                .collections
                .entry(id.clone())
                .or_insert_with(|| Collection {
                    sources: collection
                        .sources
                        .keys()
                        .map(|source| (source.clone(), SourceState::default()))
                        .collect(),
                    ..collection.clone()
                });
        }
    }
    history
}

/// Runs a step's verb as its source against the step's script.
fn replay_run(
    root: &Path,
    before: &Step,
    step: &Step,
    run: &Run,
    revisions: &mut BTreeMap<(String, String), String>,
) -> (Vec<(Option<&'static str>, String)>, Option<&'static str>) {
    for page in &run.pages {
        for item in &page.items {
            if let Some(revision) = &item.revision {
                revisions.insert(
                    (run.collection.clone(), item.handle.clone()),
                    revision.clone(),
                );
            }
        }
    }
    let mut remote = Scripted::of(step, run, revisions, false);
    let events = run_verb(root, before, step, run, &mut remote);

    let mut found = Vec::new();
    let events = match events {
        Ok(events) => events,
        Err(err) => {
            found.push((None, format!("the {} fails: {err}", run.verb)));
            Vec::new()
        }
    };

    let listing = match run.pages.first() {
        Some(page) if page.complete => vec!["round"; run.pages.len()],
        Some(_) => vec!["delta"],
        None => Vec::new(),
    };
    if run.verb == "sync" && remote.requests != listing {
        found.push((
            None,
            format!(
                "io-pimdir asks {:?}, the model lists {listing:?}",
                remote.requests
            ),
        ));
    }

    let mut known = None;
    let want: Vec<String> = run
        .pushes
        .iter()
        .map(|(push, key)| model_push(push, key))
        .collect();
    if remote.pushes != want {
        let offered: Vec<&String> = remote
            .pushes
            .iter()
            .filter(|push| !(push.starts_with("Add ") && push.contains("link Some(\"hash:")))
            .collect();
        let owed: Vec<&String> = want
            .iter()
            .filter(|push| !(push.starts_with("Update ") && !offered.contains(push)))
            .collect();
        known = match (offered.len() < remote.pushes.len(), owed.len() < want.len()) {
            (_, true) if offered == owed => Some(G16),
            (true, false) if offered == owed => Some(G13),
            _ => None,
        };
        found.push((
            known,
            format!("io-pimdir pushes {:?}, the model {want:?}", remote.pushes),
        ));
    }

    let mut got: Vec<String> = events.iter().map(event_text).collect();
    let mut want: Vec<String> = run.events.iter().map(model_event).collect();
    got.sort();
    want.sort();
    if got != want {
        let shared = step.model.collections[&run.collection].sources.len() > 1;
        let conflicted = got
            .iter()
            .any(|event| event.starts_with("Conflicted ") && !want.contains(event));
        let unchanged: Vec<&String> = want
            .iter()
            .filter(|event| !event.starts_with("ContentChanged "))
            .collect();
        let events = match () {
            _ if shared && conflicted => Some(G14),
            _ if unchanged.len() < want.len() && got.iter().collect::<Vec<_>>() == unchanged => {
                Some(G17)
            }
            _ => None,
        };
        known = known.or(events);
        found.push((
            events,
            format!("io-pimdir reports {got:?}, the model {want:?}"),
        ));
    }
    (found, known)
}

/// The options a run ran under, read where the run states none (G11).
pub fn options(before: &Step, step: &Step, run: &Run) -> PimdirSyncOptions {
    let state = step.model.collections[&run.collection]
        .sources
        .get(&run.source);
    let scope = state
        .and_then(|state| {
            state
                .open
                .as_ref()
                .map(|open| open.scope.clone())
                .or_else(|| state.coverage.clone())
        })
        .unwrap_or_default();
    let was_open = before.model.collections[&run.collection]
        .sources
        .get(&run.source)
        .is_some_and(|state| state.open.is_some());
    let conflict = POLICIES
        .iter()
        .find(|(collection, _)| *collection == run.collection)
        .map_or(PimdirConflictPolicy::Manual, |(_, policy)| *policy);

    PimdirSyncOptions {
        conflict,
        scope: scope_of(&scope),
        accept_absences: was_open && run.parked.is_none(),
        ..PimdirSyncOptions::default()
    }
}

fn scope_of(scope: &Scope) -> PimdirScope {
    PimdirScope {
        since: scope.since.map(|since| since.rfc3339()),
        until: scope.until.map(|until| until.rfc3339()),
    }
}

/// Enqueues through io-pimdir's producer the rows the step's model holds
/// and the one before did not, each drained by the owner as the model did
/// unless the model leaves it pending.
pub fn produce(root: &Path, before: &Step, step: &Step) -> Vec<String> {
    let mut found = Vec::new();
    for row in &step.model.queue[before.model.queue.len()..] {
        let action = match codec::action_from_payload(row.action.name(), &row.action.payload()) {
            Ok(action) => action,
            Err(err) => {
                found.push(format!("row {} does not decode: {err}", row.id));
                continue;
            }
        };
        let mut producer = PimdirProducer::open(root, row.producer.as_str()).unwrap();
        let object: Option<PimdirObject> = row
            .action
            .body()
            .map(|body| producer.stage_body(body).unwrap());
        match producer.enqueue(&row.collection, &action, object.as_ref()) {
            Ok(queued) if queued.handle.id != row.id => {
                found.push(format!("row {} is queued as {}", row.id, queued.handle.id));
            }
            Ok(_) => {}
            Err(err) => found.push(format!("row {} is refused: {err}", row.id)),
        }
        drop(producer);

        if row.state != RowState::Pending {
            let source = step.model.collections[&row.collection]
                .source()
                .cloned()
                .unwrap_or_else(|| String::from("local"));
            let mut owner = PimdirStore::open(root).unwrap().for_source(source);
            owner.drain().unwrap();
        }
    }
    found
}

/// Runs a step's verb as its source against `remote`, answering the events
/// it reports.
pub fn run_verb(
    root: &Path,
    before: &Step,
    step: &Step,
    run: &Run,
    remote: &mut Scripted,
) -> Result<Vec<PimdirSyncEvent>, String> {
    let options = options(before, step, run);
    let mut owner = PimdirStore::open(root)
        .unwrap()
        .for_source(run.source.as_str());
    match run.verb {
        "sync" => owner
            .sync(run.collection.as_str(), options, remote)
            .map(|report| report.events)
            .map_err(|err| err.to_string()),
        _ => owner
            .upgrade(
                run.collection.as_str(),
                run.fetch
                    .keys()
                    .map(|handle| PimdirHandle::from(handle.as_str()))
                    .collect(),
                PimdirTier::Full,
                (&options).into(),
                remote,
            )
            .map(|report| report.events)
            .map_err(|err| err.to_string()),
    }
}

/// The connector a run answers from: its pages, its fetched bodies, its
/// outcomes by handle; recording the listing asked for and each push
/// handed. Served in order, or, `resuming` a run a crash cut, by what each
/// request asks: a round from its start or after its cursor, a delta from
/// the checkpoint the run delivers answered with nothing changed, an
/// outcome answered again for a push made again.
pub struct Scripted {
    kind: String,
    collection: String,
    pages: VecDeque<PimdirRemoteSnapshot>,
    all: Vec<PimdirRemoteSnapshot>,
    resuming: bool,
    fetch: BTreeMap<String, Vec<u8>>,
    revisions: BTreeMap<(String, String), String>,
    outcomes: Vec<Outcome>,
    pushes: Vec<String>,
    requests: Vec<&'static str>,
}

impl Scripted {
    /// The connector of `run`, the revisions of earlier listings at hand.
    pub fn of(
        step: &Step,
        run: &Run,
        revisions: &BTreeMap<(String, String), String>,
        resuming: bool,
    ) -> Self {
        let kind = step.model.collections[&run.collection]
            .kind
            .map(|kind| String::from(kind.media_type()))
            .unwrap_or_default();
        let all: Vec<PimdirRemoteSnapshot> = run
            .pages
            .iter()
            .map(snapshot_of(&step.model, &run.collection))
            .collect();
        Self {
            kind,
            collection: run.collection.clone(),
            pages: all.iter().cloned().collect(),
            all,
            resuming,
            fetch: run.fetch.clone(),
            revisions: revisions.clone(),
            outcomes: run.outcomes.clone(),
            pushes: Vec::new(),
            requests: Vec::new(),
        }
    }

    /// The page a resumed run's request asks for.
    fn resumed(&self, request: &PimdirEnumerate) -> Option<PimdirRemoteSnapshot> {
        if let Some(cursor) = request.cursor() {
            let at = self
                .all
                .iter()
                .position(|page| page.cursor.as_ref() == Some(cursor))?;
            return self.all.get(at + 1).cloned();
        }
        let delivered = self.all.iter().find_map(|page| page.checkpoint.as_ref());
        if let Some(checkpoint) = request.checkpoint()
            && Some(checkpoint) == delivered
        {
            return Some(PimdirRemoteSnapshot {
                items: Vec::new(),
                vanished: Vec::new(),
                complete: false,
                last: true,
                cursor: None,
                checkpoint: Some(checkpoint.clone()),
                narrowed: false,
                capped: false,
                enumerated: None,
            });
        }
        self.all.first().cloned()
    }
}

impl PimdirRemote for Scripted {
    type Error = String;

    fn enumerate(
        &mut self,
        _: &PimdirCollectionId,
        request: PimdirEnumerate,
    ) -> Result<PimdirEnumerated, String> {
        self.requests
            .push(match (request.head(), request.checkpoint()) {
                (Some(_), _) => "head",
                (None, Some(_)) => "delta",
                (None, None) => "round",
            });
        if request.head().is_some() {
            return Ok(PimdirEnumerated::NoHead);
        }
        if self.resuming {
            return self
                .resumed(&request)
                .map(PimdirEnumerated::Page)
                .ok_or_else(|| String::from("no page scripted"));
        }
        self.pages
            .pop_front()
            .map(PimdirEnumerated::Page)
            .ok_or_else(|| String::from("no page scripted"))
    }

    fn fetch(
        &mut self,
        _: &PimdirCollectionId,
        handles: Vec<PimdirHandle>,
        tier: PimdirTier,
    ) -> Result<Vec<PimdirFetchedItem>, String> {
        let mut items = Vec::new();
        for handle in handles {
            let Some(body) = self.fetch.get(handle.as_str()).cloned() else {
                continue;
            };
            let derivation = derive(&self.kind, &body, tier == PimdirTier::Meta);
            let revision = self
                .revisions
                .get(&(self.collection.clone(), handle.0.clone()))
                .cloned();
            items.push(PimdirFetchedItem {
                handle,
                link_id: derivation.link_id,
                summary: derivation.summary,
                sort_key: derivation.sort_key,
                body: (tier == PimdirTier::Full).then(|| PimdirFetchedBody::Inline {
                    hash: hash::hash(&body),
                    bytes: body,
                }),
                revision,
            });
        }
        Ok(items)
    }

    fn push(
        &mut self,
        _: &PimdirCollectionId,
        changes: Vec<PimdirChange>,
    ) -> Result<Vec<PimdirPushResult>, String> {
        let mut results = Vec::new();
        for change in changes {
            self.pushes.push(change_text(&change));
            let handle = change.handle().clone();
            let Some(at) = self
                .outcomes
                .iter()
                .position(|outcome| outcome.handle == handle.0)
            else {
                continue;
            };
            let outcome = match self.resuming {
                true => self.outcomes[at].clone(),
                false => self.outcomes.remove(at),
            };
            results.push(PimdirPushResult {
                relocated: outcome.relocated.unwrap_or(false),
                handle,
                outcome: match outcome.answer {
                    Answer::Accepted => PimdirPushOutcome::Accepted,
                    Answer::Rejected => PimdirPushOutcome::Rejected,
                    Answer::Refused(reason) => PimdirPushOutcome::Refused { reason },
                },
                assigned: outcome.assigned.as_deref().map(PimdirHandle::from),
                revision: outcome.revision,
            });
        }
        Ok(results)
    }
}

/// What io-pimdir derives from a body a connector hands it: a message's
/// meta read without the walk of its parts (Annex A.1), else the whole.
fn derive(kind: &str, body: &[u8], meta: bool) -> summary::PimdirDerivation {
    match (meta, kind) {
        (true, "message/rfc822") => mail::derive_meta(body, Some(body.len() as u64), None),
        _ => summary::derive(kind, body).expect("a known kind"),
    }
}

/// A generated page as a connector answers it.
fn snapshot_of<'a>(
    model: &'a Model,
    collection: &'a str,
) -> impl Fn(&Page) -> PimdirRemoteSnapshot + 'a {
    let kind = model.collections[collection]
        .kind
        .map(|kind| String::from(kind.media_type()))
        .unwrap_or_default();
    move |page| PimdirRemoteSnapshot {
        items: page
            .items
            .iter()
            .map(|item| {
                let meta = PimdirRemoteMeta::new(derive(&kind, &item.meta, true));
                PimdirRemoteItem {
                    handle: PimdirHandle::from(item.handle.as_str()),
                    flags: PimdirFlags::Known(item.flags.clone()),
                    revision: item.revision.clone(),
                    meta: match item.carries {
                        true => meta.with_body(PimdirFetchedBody::Inline {
                            hash: hash::hash(&item.meta),
                            bytes: item.meta.clone(),
                        }),
                        false => meta,
                    },
                }
            })
            .collect(),
        vanished: page
            .vanished
            .iter()
            .map(|handle| PimdirHandle::from(handle.as_str()))
            .collect(),
        complete: page.complete,
        last: page.last,
        cursor: page
            .cursor
            .as_ref()
            .map(|cursor| PimdirCursor(cursor.as_bytes().to_vec())),
        checkpoint: page
            .checkpoint
            .as_ref()
            .map(|checkpoint| PimdirCheckpoint(checkpoint.as_bytes().to_vec())),
        narrowed: page.narrowed,
        capped: page.capped,
        enumerated: page.enumerated.as_ref().map(|handles| {
            handles
                .iter()
                .map(|handle| PimdirHandle::from(handle.as_str()))
                .collect()
        }),
    }
}

/// A push io-pimdir hands, as [`model_push`] spells the model's.
fn change_text(change: &PimdirChange) -> String {
    let handle = change.handle().as_str();
    let key = change.key.as_str();
    match &change.kind {
        PimdirChangeKind::Add {
            link_id,
            flags,
            origin,
            object,
            ..
        } => format!(
            "Add {handle} {key} link {:?} flags {:?} origin {:?} object {:?}",
            link_id.as_ref().map(|link| link.as_str()),
            flags.known(),
            origin
                .as_ref()
                .map(|origin| (origin.collection.0.as_str(), origin.handle.as_str())),
            object.as_ref().map(|object| object.as_str()),
        ),
        PimdirChangeKind::Remove {
            to,
            link_id,
            if_match,
            ..
        } => format!(
            "Remove {handle} {key} to {:?} link {:?} if_match {if_match:?}",
            to.as_ref().map(|to| to.0.as_str()),
            link_id.as_ref().map(|link| link.as_str()),
        ),
        PimdirChangeKind::SetFlags { add, remove, .. } => {
            format!("SetFlags {handle} {key} add {add:?} remove {remove:?}")
        }
        PimdirChangeKind::Update {
            object, if_match, ..
        } => format!(
            "Update {handle} {key} object {:?} if_match {if_match:?}",
            object.as_str()
        ),
    }
}

/// A push the model states, as [`change_text`] spells io-pimdir's.
fn model_push(push: &Push, key: &str) -> String {
    match push {
        Push::Add {
            handle,
            link_id,
            flags,
            origin,
            object,
        } => format!(
            "Add {handle} {key} link {:?} flags {:?} origin {:?} object {:?}",
            Some(link_id.as_str()),
            Some(flags),
            origin
                .as_ref()
                .map(|(collection, handle)| (collection.as_str(), handle.as_str())),
            Some(object.0.as_str()),
        ),
        Push::Remove {
            handle,
            to,
            link_id,
            if_match,
        } => format!(
            "Remove {handle} {key} to {:?} link {:?} if_match {if_match:?}",
            to.as_deref(),
            Some(link_id.as_str()),
        ),
        Push::SetFlags {
            handle,
            add,
            remove,
        } => format!("SetFlags {handle} {key} add {add:?} remove {remove:?}"),
        Push::Update {
            handle,
            object,
            if_match,
        } => format!(
            "Update {handle} {key} object {:?} if_match {if_match:?}",
            object.0.as_str()
        ),
    }
}

/// An event io-pimdir reports, as [`model_event`] spells the model's.
fn event_text(event: &PimdirSyncEvent) -> String {
    match event {
        PimdirSyncEvent::Added(handle) => format!("Added {}", handle.as_str()),
        PimdirSyncEvent::FlagsChanged(handle) => format!("FlagsChanged {}", handle.as_str()),
        PimdirSyncEvent::ContentChanged(handle) => format!("ContentChanged {}", handle.as_str()),
        PimdirSyncEvent::Conflicted(handle) => format!("Conflicted {}", handle.as_str()),
        PimdirSyncEvent::Vanished(handle) => format!("Vanished {}", handle.as_str()),
        PimdirSyncEvent::Created(handle) => format!("Created {}", handle.as_str()),
        PimdirSyncEvent::Overruled { handle, link_id } => {
            format!("Overruled {} {}", handle.as_str(), link_id.as_str())
        }
        PimdirSyncEvent::Refused { handle, reason } => {
            format!("Refused {} {reason}", handle.as_str())
        }
        event => format!("{event:?}"),
    }
}

/// An event the model states, as [`event_text`] spells io-pimdir's.
fn model_event(event: &Event) -> String {
    match event {
        Event::Added(handle) => format!("Added {handle}"),
        Event::FlagsChanged(handle) => format!("FlagsChanged {handle}"),
        Event::ContentChanged(handle) => format!("ContentChanged {handle}"),
        Event::Conflicted(handle) => format!("Conflicted {handle}"),
        Event::Vanished(handle) => format!("Vanished {handle}"),
        Event::Created(handle) => format!("Created {handle}"),
        Event::Overruled { handle, link_id } => format!("Overruled {handle} {link_id}"),
        Event::Refused { handle, reason } => format!("Refused {handle} {reason}"),
    }
}

/// The bodies a row of the store at `root` names whose blob is not held:
/// a body lost.
fn unheld(root: &Path) -> BTreeSet<String> {
    let conn = Connection::open(root.join("pimdir.db")).unwrap();
    let mut statement = conn
        .prepare(
            "SELECT object_hash FROM items WHERE object_hash IS NOT NULL \
             UNION SELECT conflict_object FROM items WHERE conflict_object IS NOT NULL \
             UNION SELECT base_object FROM bindings WHERE base_object IS NOT NULL \
             UNION SELECT shared_object FROM bindings WHERE shared_object IS NOT NULL \
             UNION SELECT conflict_object FROM bindings WHERE conflict_object IS NOT NULL",
        )
        .unwrap();
    let named: Vec<String> = statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    named
        .into_iter()
        .filter(|name| !root.join("objects").join(&name[..2]).join(name).is_file())
        .collect()
}

/// The first receipt applied before one of a lower id: a row applied out
/// of the queue's append order (STORAGE §15.2).
fn out_of_order(root: &Path) -> Option<String> {
    let conn = Connection::open(root.join("pimdir.db")).unwrap();
    let mut statement = conn
        .prepare("SELECT id, applied_at FROM receipts ORDER BY id")
        .unwrap();
    let receipts: Vec<(i64, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    receipts.windows(2).find_map(|pair| {
        (pair[1].1 < pair[0].1).then(|| {
            format!(
                "receipt {} applied at {}, before receipt {} at {}",
                pair[1].0, pair[1].1, pair[0].0, pair[0].1
            )
        })
    })
}
