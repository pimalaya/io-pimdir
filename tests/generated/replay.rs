//! # The engine replaying generated histories
//!
//! Each history of the generator (`History::mail`, `cards`, `moves`,
//! `refusals`, `appends`, `policies`, `sources`, `scopes`, `rekeys`,
//! `crashes`, `heads`, `bodies`) replayed through io-pimdir from the store
//! of its first step: a run answered by a scripted connector from the run's
//! own head, pages, fetches and outcomes, a run the model says failed cut
//! where it failed, a producer's rows enqueued through io-pimdir's producer
//! and drained by its owner, a remote's change left to the next run's
//! pages. After each step the store is read against the oracle of the
//! step's model; a run's pushes, in order, its events and the listing it
//! asked for are held to the run's.
//! Beside the model, every step holds what a run must whatever the model
//! says: no row names a body the store lacks (no loss), a run the model
//! finds nothing to do in leaves every row as it stood (convergence), and
//! receipts follow the queue's append order.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    mem,
    path::Path,
};

use io_pimdir::{
    change::{PimdirChange, PimdirChangeKind},
    client::{PimdirStore, producer::PimdirProducer},
    codec,
    collection::{PimdirCheckpoint, PimdirCollectionId, PimdirCursor, PimdirScope},
    hash,
    mutate::PimdirMutation,
    object::PimdirObject,
    placement::{PimdirFlags, PimdirHandle},
    remote::{
        PimdirEnumerate, PimdirEnumerated, PimdirFetchedBody, PimdirFetchedItem, PimdirHeads,
        PimdirPushOutcome, PimdirPushResult, PimdirRemote, PimdirRemoteItem, PimdirRemoteMeta,
        PimdirRemoteSnapshot, PimdirTier,
    },
    summary::{self, mail},
    sync::{PimdirConflictPolicy, PimdirSyncEvent, PimdirSyncOptions},
};
use pimdir_generator::{
    engine::{Answer, Event, Outcome, Push, Run},
    history::{History, Step},
    model::{Collection, Model, Scope, SourceState},
    queue::RowState,
    remote::Page,
    shrink::Case,
    spec::Spec,
    vector::Asked,
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

#[test]
fn every_generated_history_replays_as_its_model_states() {
    for seed in SEEDS {
        let differences = differences(seed);
        assert!(differences.is_empty(), "seed {seed}: {differences:#?}");
    }
}

/// Every difference the histories of `seed` replay with.
fn differences(seed: u64) -> Vec<String> {
    histories(seed)
        .into_iter()
        .flat_map(|(name, history)| {
            replay(&history)
                .into_iter()
                .map(move |(_, text)| format!("{name}: {text}"))
        })
        .collect()
}

/// Every history of the generator at `seed`, by name.
pub fn histories(seed: u64) -> [(&'static str, History); 12] {
    [
        ("mail", History::mail(seed)),
        ("cards", History::cards(seed)),
        ("moves", History::moves(seed)),
        ("refusals", History::refusals(seed)),
        ("appends", History::appends(seed)),
        ("policies", History::policies(seed)),
        ("sources", History::sources(seed)),
        ("scopes", History::scopes(seed)),
        ("rekeys", History::rekeys(seed)),
        ("crashes", History::crashes(seed)),
        ("heads", History::heads(seed)),
        ("bodies", History::bodies(seed)),
    ]
}

/// Replays `history` from the store of its first step, naming each
/// disagreement with the index of the step it is found after and that
/// step. A step disagreeing leaves the store rewritten from its model, so
/// the next step starts where the model's does and a finding is named
/// once, at the step it is made.
pub fn replay(history: &History) -> Vec<(usize, String)> {
    let history = declared(history);
    let spec = Spec::load_default().unwrap();
    let first = &history.steps[0];
    let mut generated = Generated::write(Spec::load_default().unwrap(), first.model.clone());

    let mut revisions = BTreeMap::new();
    let mut differences = Vec::new();
    for (index, pair) in history.steps.windows(2).enumerate() {
        let (before, step) = (&pair[0], &pair[1]);
        let at = format!("step {} ({})", index + 1, step.label);
        let found = replay_step(&spec, &generated.root(), before, step, &mut revisions);
        if found.is_empty() {
            continue;
        }
        differences.extend(
            found
                .into_iter()
                .map(|text| (index + 1, format!("{at}: {text}"))),
        );
        generated = Generated::write(Spec::load_default().unwrap(), step.model.clone());
    }
    differences
}

/// Replays the one run of `case` from a store written from the model
/// before it, as [`replay`] replays that step: what a shrunk case answers.
pub fn replay_case(case: &Case) -> Vec<String> {
    let history = declared(&case.history());
    let spec = Spec::load_default().unwrap();
    let (before, step) = (&history.steps[0], &history.steps[1]);
    let generated = Generated::write(Spec::load_default().unwrap(), before.model.clone());
    let mut revisions = case.revisions.clone();
    replay_step(&spec, &generated.root(), before, step, &mut revisions)
}

/// Replays `step` on the store at `root`, which holds `before`'s model,
/// answering every difference it leaves.
fn replay_step(
    spec: &Spec,
    root: &Path,
    before: &Step,
    step: &Step,
    revisions: &mut BTreeMap<(String, String), String>,
) -> Vec<String> {
    let mut found = produce(root, before, step);

    let rows = unstamped(root);
    if let Some(run) = &step.run {
        found.extend(replay_run(root, before, step, run, revisions));
    }
    let (_, read) = reader::compare(spec, &step.model, root, |_, _, _, _| None);
    found.extend(read.into_iter().map(|(_, text)| text));
    for hash in unheld(root) {
        found.push(format!("a row names {hash}, a body the store lacks"));
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
        && unstamped(root) != rows
    {
        found.push(format!(
            "a run with nothing to do moved a row: {}",
            crash::apart(&unstamped(root), &rows)
        ));
    }
    if let Some(order) = out_of_order(root) {
        found.push(order);
    }
    found
}

/// What `run` of `step` was asked, as [`options`] and the connector read
/// it, in the terms a sync vector states it.
pub fn asked(before: &Step, step: &Step, run: &Run) -> Asked {
    let options = options(before, step, run);
    let conflict = match options.conflict {
        PimdirConflictPolicy::PreferRemote => "prefer-remote",
        PimdirConflictPolicy::PreferLocal => "prefer-local",
        PimdirConflictPolicy::KeepLocal => "keep-local",
        _ => "manual",
    };
    Asked {
        conflict: String::from(conflict),
        since: options.scope.since,
        until: options.scope.until,
        accept_absences: match run.verb {
            "rekey" => run.options.accept_absences,
            _ => options.accept_absences,
        },
        scope_bound: !unbound(before, step, run),
        cursor_rejected: restarted(before, step, run),
    }
}

/// The history with every collection it ever declares declared from its
/// first step as it was first declared, never synced there: a declaration
/// writes nothing but its row and its sources' (STORAGE §14), which
/// io-pimdir's owner would write as the configuration it is, so a step's
/// run finds them as the model's did.
pub fn declared(history: &History) -> History {
    let mut first = BTreeMap::new();
    for step in &history.steps {
        for (id, collection) in &step.model.collections {
            first
                .entry(id.clone())
                .or_insert_with(|| collection.clone());
        }
    }
    let mut history = history.clone();
    for step in &mut history.steps {
        for (id, collection) in &first {
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
) -> Vec<String> {
    remember(run, revisions);
    let mut remote = Scripted::of(before, step, run, revisions, false);
    let events = run_verb(root, before, step, run, &mut remote);

    let mut found = Vec::new();
    let events = match (events, run.failed) {
        (Ok(events), false) => events,
        (Ok(_), true) => {
            found.push(format!("the {} passes, the model's fails", run.verb));
            Vec::new()
        }
        (Err(err), false) => {
            found.push(format!("the {} fails: {err}", run.verb));
            Vec::new()
        }
        (Err(_), true) => Vec::new(),
    };

    let listing = listing(before, step, run);
    if matches!(run.verb, "sync" | "rekey") && remote.requests != listing {
        found.push(format!(
            "io-pimdir asks {:?}, the model lists {listing:?}",
            remote.requests
        ));
    }

    let want: Vec<String> = run
        .pushes
        .iter()
        .map(|(push, key)| model_push(push, key))
        .collect();
    if remote.pushes != want {
        found.push(format!(
            "io-pimdir pushes {:?}, the model {want:?}",
            remote.pushes
        ));
    }

    let mut got: Vec<String> = events.iter().map(event_text).collect();
    let mut want: Vec<String> = run.events.iter().map(model_event).collect();
    got.sort();
    want.sort();
    if got != want && !run.failed {
        found.push(format!("io-pimdir reports {got:?}, the model {want:?}"));
    }
    found
}

/// Keeps the revision of each member the run's head and pages list, which
/// a later fetch answers.
pub fn remember(run: &Run, revisions: &mut BTreeMap<(String, String), String>) {
    for page in run.head.iter().chain(&run.pages) {
        for item in &page.items {
            if let Some(revision) = &item.revision {
                revisions.insert(
                    (run.collection.clone(), item.handle.clone()),
                    revision.clone(),
                );
            }
        }
    }
}

/// The requests a run's listing makes: its head, a cursor the source
/// rejects, each page landed, and the page whose request failed the run
/// between two pages.
fn listing(before: &Step, step: &Step, run: &Run) -> Vec<&'static str> {
    let mut listing = Vec::new();
    if run.head.is_some() {
        listing.push("head");
    }
    if restarted(before, step, run) {
        listing.push("round");
    }
    match run.pages.first() {
        Some(page) if page.complete => listing.extend(vec!["round"; run.pages.len()]),
        Some(_) => listing.push("delta"),
        None => {}
    }
    if cut_between_pages(run) {
        listing.push("round");
    }
    listing
}

/// Whether the run failed between two pages of its round, a connection lost
/// or a page handing no cursor, rather than after a push.
fn cut_between_pages(run: &Run) -> bool {
    run.verb == "sync" && run.failed && run.pushes.is_empty()
}

/// Whether the run restarted the round it resumed, the source rejecting
/// its cursor, which the run does not state (G11): the open round it found
/// over the scope it asks, a cursor to resume from, and a new round id.
fn restarted(before: &Step, step: &Step, run: &Run) -> bool {
    let state = |step: &Step| {
        step.model.collections[&run.collection]
            .sources
            .get(&run.source)
            .cloned()
            .unwrap_or_default()
    };
    let was = state(before);
    run.verb == "sync"
        && was.open.as_ref().is_some_and(|open| {
            open.cursor.is_some() && !open.band && open.scope == run.options.scope
        })
        && run.pages.first().is_some_and(|page| page.complete)
        && state(step).rounds == was.rounds + 1
}

/// Whether the run's connector lists a band its scope-bound checkpoint
/// does not cover, which the run does not state (G11): the round it opens
/// or resumes is a band's.
fn unbound(before: &Step, step: &Step, run: &Run) -> bool {
    [before, step].into_iter().any(|step| {
        step.model.collections[&run.collection]
            .sources
            .get(&run.source)
            .and_then(|state| state.open.as_ref())
            .is_some_and(|open| open.band)
    })
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
            let mut owner = PimdirStore::open(root).unwrap().for_source(source).unwrap();
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
        .for_source(run.source.as_str())
        .unwrap();
    match run.verb {
        "sync" => owner
            .sync(run.collection.as_str(), options, remote)
            .map(|report| report.events)
            .map_err(|err| err.to_string()),
        "rekey" => owner
            .rekey(run.collection.as_str(), run.options.accept_absences, remote)
            .map(|report| report.events)
            .map_err(|err| err.to_string()),
        "mutate" => owner
            .mutate(run.collection.as_str(), overrule(before, step, run))
            .map(|report| report.events)
            .map_err(|err| err.to_string()),
        _ => owner
            .upgrade(
                run.collection.as_str(),
                run.handles
                    .iter()
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

/// The `Edit` an engine overrules a conflict with, the generator's only
/// `mutate` run (SYNC §7): the body the item takes, which the run does not
/// state (G11), read from the item after the run, or before it where the
/// run is refused and the item keeps the body it held.
fn overrule(before: &Step, step: &Step, run: &Run) -> PimdirMutation {
    let handle = &run.handles[0];
    let model = match run.failed {
        true => &before.model,
        false => &step.model,
    };
    let item = model
        .items
        .iter()
        .find(|item| {
            item.collection == run.collection
                && item
                    .bound(&run.source)
                    .is_some_and(|binding| &binding.handle == handle)
        })
        .expect("a bound item");
    let body = model.objects[item.object.as_ref().expect("a held body")].clone();
    let kind = model.collections[&run.collection]
        .kind
        .map(|kind| String::from(kind.media_type()))
        .unwrap_or_default();
    let derivation = summary::derive(&kind, &body).expect("a known kind");
    PimdirMutation::Edit {
        handle: PimdirHandle::from(handle.as_str()),
        object: PimdirObject {
            hash: hash::hash(&body),
            size: body.len(),
        },
        body,
        summary: derivation.summary,
        sort_key: Some(derivation.sort_key),
        if_match: None,
        overrule: true,
        resolves: None,
    }
}

/// The connector a run answers from: its head, its pages, its fetched
/// bodies, its outcomes by handle; recording the listing asked for and each
/// push handed. Served in order, or, `resuming` a run a crash cut, by what
/// each request asks: a round from its start or after its cursor, a delta
/// from the checkpoint the run delivers answered with nothing changed, an
/// outcome answered again for a push made again. A run the model says
/// failed fails where the model's did: a round cut between two pages
/// answers the request after its last page landed with a page before the
/// last handing no cursor, which fails the run as a connection lost there
/// would, nothing of it written; a run cut after a push the remote took
/// fails that push, its outcome never written.
pub struct Scripted {
    kind: String,
    collection: String,
    head: Option<PimdirRemoteSnapshot>,
    reject_cursor: bool,
    scope_bound: bool,
    cut: bool,
    crash: Option<usize>,
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
        before: &Step,
        step: &Step,
        run: &Run,
        revisions: &BTreeMap<(String, String), String>,
        resuming: bool,
    ) -> Self {
        let kind = step.model.collections[&run.collection]
            .kind
            .map(|kind| String::from(kind.media_type()))
            .unwrap_or_default();
        let snapshot = snapshot_of(&step.model, &run.collection);
        let all: Vec<PimdirRemoteSnapshot> = run.pages.iter().map(&snapshot).collect();
        Self {
            kind,
            collection: run.collection.clone(),
            head: run.head.as_ref().map(&snapshot),
            reject_cursor: restarted(before, step, run),
            scope_bound: !unbound(before, step, run),
            cut: cut_between_pages(run),
            crash: (run.failed && !run.pushes.is_empty()).then_some(run.pushes.len()),
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

    /// The connector of a run resumed past the restart its source made,
    /// the cursor it rejected long gone: one cursor is rejected once, and a
    /// store a crash left holding the restarted round resumes it.
    pub fn restarted(mut self, restarted: bool) -> Self {
        if restarted {
            self.reject_cursor = false;
        }
        self
    }

    /// The page a resumed run's request asks for: after the page whose
    /// cursor it names, the first for the cursor the run resumed from.
    fn resumed(&self, request: &PimdirEnumerate) -> Option<PimdirRemoteSnapshot> {
        if let Some(cursor) = request.cursor() {
            return match self
                .all
                .iter()
                .position(|page| page.cursor.as_ref() == Some(cursor))
            {
                Some(at) => self.all.get(at + 1).cloned(),
                None => self.all.first().cloned(),
            };
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

    /// The page a round cut between two pages answers once its last page
    /// landed.
    fn cut_page() -> PimdirRemoteSnapshot {
        PimdirRemoteSnapshot {
            items: Vec::new(),
            vanished: Vec::new(),
            complete: true,
            last: false,
            cursor: None,
            checkpoint: None,
            narrowed: false,
            capped: false,
            enumerated: None,
        }
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
            return Ok(match &self.head {
                Some(head) => PimdirEnumerated::Page(head.clone()),
                None => PimdirEnumerated::NoHead,
            });
        }
        if request.cursor().is_some() && mem::take(&mut self.reject_cursor) {
            return Ok(PimdirEnumerated::CursorRejected);
        }
        let page = match self.resuming {
            true => self.resumed(&request),
            false => self.pages.pop_front(),
        };
        match (page, self.cut) {
            (Some(page), _) => Ok(PimdirEnumerated::Page(page)),
            (None, true) => Ok(PimdirEnumerated::Page(Self::cut_page())),
            (None, false) => Err(String::from("no page scripted")),
        }
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
            if self.crash == Some(self.pushes.len()) {
                return Err(String::from("cut after a push the remote took"));
            }
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

    fn scope_bound(&self) -> bool {
        self.scope_bound
    }

    fn heads(&self) -> PimdirHeads {
        match self.head {
            Some(_) => PimdirHeads::DELTA,
            None => PimdirHeads::NONE,
        }
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
/// a body lost. Each pointer is an object id, read as its row's hash.
fn unheld(root: &Path) -> BTreeSet<String> {
    let conn = Connection::open(root.join("pimdir.db")).unwrap();
    let mut statement = conn
        .prepare(
            "SELECT o.hash FROM objects o WHERE o.id IN ( \
               SELECT object FROM items WHERE object IS NOT NULL \
               UNION SELECT conflict_object FROM items WHERE conflict_object IS NOT NULL \
               UNION SELECT base_object FROM bindings WHERE base_object IS NOT NULL \
               UNION SELECT shared_object FROM bindings WHERE shared_object IS NOT NULL \
               UNION SELECT conflict_object FROM bindings WHERE conflict_object IS NOT NULL)",
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

/// Every row of the store at `root` as [`crash::store_rows`] dumps it, the
/// SYNC version an engine's first run stamps on a store the generator
/// wrote left out: a stamp, no row a run moves.
fn unstamped(root: &Path) -> BTreeMap<String, Vec<Vec<Value>>> {
    let conn = Connection::open(root.join("pimdir.db")).unwrap();
    let at: i64 = conn
        .query_row(
            "SELECT cid FROM pragma_table_info('store_meta') WHERE name = 'sync_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    drop(conn);
    let mut rows = crash::store_rows(root);
    for row in rows.get_mut("store_meta").into_iter().flatten() {
        row[at as usize] = Value::Null;
    }
    rows
}
