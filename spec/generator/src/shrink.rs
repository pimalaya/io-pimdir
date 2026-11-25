//! # Shrinking
//!
//! A failing case made smaller while it still fails the same way (README.md,
//! "Shrinking"). An engine failure is one step of a history: [`Case::of`]
//! isolates it, every event before it folded into the store it starts
//! from, and [`Case::candidates`] proposes it with fewer items,
//! collections, sources and bookkeeping rows, and with shorter bodies. A
//! failure of another suite is a model: [`candidates`] proposes it with
//! fewer collections, items, queue rows and bookkeeping rows. [`shrink`]
//! keeps a candidate when the caller's predicate says it still fails, and
//! starts again from it, until none does.
//!
//! A candidate drops the same rows from the store before and after a run,
//! never what the run names (a page's member it pushes to, reports or
//! fetches), so the model's expectation stays what SYNC states of the
//! smaller store; the predicate, which runs the implementation, holds the
//! rest: a candidate disagreeing anew is not the same failure and is not
//! kept.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::types::Value;

use crate::{
    engine::{Push, Run},
    history::History,
    model::{Item, Model, Table},
    name::ObjectName,
    queue::Action,
};

/// One engine step and the store it starts from.
#[derive(Clone, Debug)]
pub struct Case {
    /// The store before the run, every earlier event folded into it.
    pub before: Model,
    /// The run.
    pub run: Run,
    /// The store after the run, as the model states it.
    pub after: Model,
    /// The revision each member an earlier run listed holds, by collection
    /// and handle, which a fetch of this run answers.
    pub revisions: BTreeMap<(String, String), String>,
}

impl Case {
    /// The run of step `step` of `history`, its revisions remembered from
    /// the runs before it.
    pub fn of(history: &History, step: usize) -> Option<Self> {
        let run = history.steps.get(step)?.run.clone()?;
        let mut revisions = BTreeMap::new();
        for earlier in history.steps[..=step].iter().filter_map(|s| s.run.as_ref()) {
            for page in earlier.head.iter().chain(&earlier.pages) {
                for item in &page.items {
                    if let Some(revision) = &item.revision {
                        revisions.insert(
                            (earlier.collection.clone(), item.handle.clone()),
                            revision.clone(),
                        );
                    }
                }
            }
        }
        Some(Self {
            before: history.steps[step - 1].model.clone(),
            run,
            after: history.steps[step].model.clone(),
            revisions,
        })
    }

    /// The case as a history of two steps, the store before the run and
    /// the run.
    pub fn history(&self) -> History {
        History {
            steps: vec![
                crate::history::Step {
                    label: String::from("the store before the run"),
                    run: None,
                    model: self.before.clone(),
                },
                crate::history::Step {
                    label: String::from("the run"),
                    run: Some(self.run.clone()),
                    model: self.after.clone(),
                },
            ],
        }
    }

    /// The smaller cases to try, each named by what it drops, the largest
    /// cuts first.
    pub fn candidates(&self) -> Vec<(String, Case)> {
        let mut candidates = Vec::new();

        let mut bare = self.clone();
        for model in [&mut bare.before, &mut bare.after] {
            model.queue.clear();
            model.capabilities.clear();
            model.performers.clear();
            model.references.clear();
        }
        if bare.before.queue.len() != self.before.queue.len()
            || bare.before.capabilities.len() != self.before.capabilities.len()
            || bare.before.references.len() != self.before.references.len()
            || bare.before.performers.len() != self.before.performers.len()
        {
            candidates.push((String::from("the queue and the bookkeeping rows"), bare));
        }

        let named = self.named_collections();
        for id in self.before.collections.keys() {
            if named.contains(id) {
                continue;
            }
            let mut smaller = self.clone();
            for model in [&mut smaller.before, &mut smaller.after] {
                drop_collection(model, id);
                renumber(model);
            }
            candidates.push((format!("collection {id}"), smaller));
        }

        for source in self.other_sources() {
            let mut smaller = self.clone();
            for model in [&mut smaller.before, &mut smaller.after] {
                for collection in model.collections.values_mut() {
                    collection.sources.remove(&source);
                }
                for item in &mut model.items {
                    item.bindings.retain(|binding| binding.source != source);
                }
            }
            candidates.push((format!("source {source}"), smaller));
        }

        let droppable = self.droppable();
        if droppable.len() > 1 {
            let (first, second) = droppable.split_at(droppable.len() / 2);
            for half in [first, second] {
                let mut smaller = self.clone();
                for key in half {
                    smaller.drop_item(key);
                }
                candidates.push((format!("{} items", half.len()), smaller));
            }
        }
        for key in &droppable {
            let mut smaller = self.clone();
            smaller.drop_item(key);
            candidates.push((format!("item {} in {}", key.1, key.0), smaller));
        }

        if let Some(shorter) = self.shortened() {
            candidates.push((String::from("the mail bodies' text"), shorter));
        }
        candidates
    }

    /// The collections the run names: its own, and those its pushes move
    /// into or copy from.
    fn named_collections(&self) -> BTreeSet<String> {
        let mut named = BTreeSet::from([self.run.collection.clone()]);
        for (push, _) in &self.run.pushes {
            match push {
                Push::Remove { to: Some(to), .. } => {
                    named.insert(to.clone());
                }
                Push::Add {
                    origin: Some((origin, _)),
                    ..
                } => {
                    named.insert(origin.clone());
                }
                _ => {}
            }
        }
        named
    }

    fn other_sources(&self) -> BTreeSet<String> {
        self.before
            .collections
            .values()
            .flat_map(|collection| collection.sources.keys().cloned())
            .filter(|source| source != &self.run.source)
            .collect()
    }

    /// The items held before and after the run that the run names nothing
    /// of, by collection and key.
    fn droppable(&self) -> Vec<(String, String)> {
        let mut named: BTreeSet<String> = BTreeSet::new();
        for (push, _) in &self.run.pushes {
            named.insert(String::from(push.handle()));
        }
        for outcome in &self.run.outcomes {
            named.insert(outcome.handle.clone());
            named.extend(outcome.assigned.clone());
        }
        for event in &self.run.events {
            named.insert(event_handle(event));
        }
        named.extend(self.run.handles.iter().cloned());
        named.extend(self.run.fetch.keys().cloned());
        let counted = self.run.parked.is_some() || self.run.unlisted.is_some();

        self.before
            .items
            .iter()
            .filter(|item| {
                let after =
                    self.after.items.iter().any(|held| {
                        held.collection == item.collection && held.link_id == item.link_id
                    });
                let own = item.collection == self.run.collection;
                let touched = own
                    && item.bindings.iter().any(|binding| {
                        binding.source == self.run.source && named.contains(&binding.handle)
                    });
                after && !touched && !(own && counted)
            })
            .map(|item| (item.collection.clone(), item.link_id.clone()))
            .collect()
    }

    /// Drops the item `key` from both stores and its member from the run's
    /// pages.
    fn drop_item(&mut self, key: &(String, String)) {
        let handles: BTreeSet<String> = self
            .before
            .items
            .iter()
            .filter(|item| item.collection == key.0 && item.link_id == key.1)
            .flat_map(|item| item.bindings.iter())
            .filter(|binding| binding.source == self.run.source)
            .map(|binding| binding.handle.clone())
            .collect();
        for model in [&mut self.before, &mut self.after] {
            model
                .items
                .retain(|item| !(item.collection == key.0 && item.link_id == key.1));
        }
        if key.0 != self.run.collection {
            return;
        }
        for page in self.run.head.iter_mut().chain(self.run.pages.iter_mut()) {
            page.items.retain(|item| !handles.contains(&item.handle));
            page.vanished.retain(|handle| !handles.contains(handle));
            if let Some(enumerated) = &mut page.enumerated {
                enumerated.retain(|handle| !handles.contains(handle));
            }
        }
    }

    /// The case with every generated message's text cut to one letter, its
    /// size restated, its name and the keys naming it moved.
    fn shortened(&self) -> Option<Self> {
        let mut renamed: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        for model in [&self.before, &self.after] {
            for bytes in model.objects.values() {
                if let Some(short) = short_text(bytes) {
                    renamed.insert(bytes.clone(), short);
                }
            }
        }
        for page in self.run.head.iter().chain(&self.run.pages) {
            for item in &page.items {
                if let Some(short) = short_text(&item.meta) {
                    renamed.insert(item.meta.clone(), short);
                }
            }
        }
        for bytes in self.run.fetch.values() {
            if let Some(short) = short_text(bytes) {
                renamed.insert(bytes.clone(), short);
            }
        }
        if renamed.is_empty() {
            return None;
        }
        let names: BTreeMap<ObjectName, (ObjectName, i64, i64)> = renamed
            .iter()
            .map(|(old, new)| {
                (
                    ObjectName::of(old),
                    (ObjectName::of(new), old.len() as i64, new.len() as i64),
                )
            })
            .collect();

        let mut case = self.clone();
        for model in [&mut case.before, &mut case.after] {
            rename(model, &renamed, &names);
        }
        for page in case.run.head.iter_mut().chain(case.run.pages.iter_mut()) {
            for item in &mut page.items {
                if let Some(new) = renamed.get(&item.meta) {
                    item.meta = new.clone();
                }
            }
        }
        for bytes in case.run.fetch.values_mut() {
            if let Some(new) = renamed.get(bytes) {
                *bytes = new.clone();
            }
        }
        let collection = case.run.collection.clone();
        for (push, key) in &mut case.run.pushes {
            if let Push::Add { object, .. } | Push::Update { object, .. } = push
                && let Some((new, _, _)) = names.get(object)
            {
                *object = new.clone();
            }
            *key = push.key(&collection);
        }
        Some(case)
    }
}

/// The handle an event names.
fn event_handle(event: &crate::engine::Event) -> String {
    use crate::engine::Event;
    match event {
        Event::Added(handle)
        | Event::FlagsChanged(handle)
        | Event::ContentChanged(handle)
        | Event::Conflicted(handle)
        | Event::Vanished(handle)
        | Event::Created(handle)
        | Event::Overruled { handle, .. }
        | Event::Refused { handle, .. } => handle.clone(),
    }
}

/// A generated message whose single text part is longer than one letter,
/// with that text cut to `x`.
fn short_text(bytes: &[u8]) -> Option<Vec<u8>> {
    const PART: &[u8] = b"Content-Type: text/plain; charset=utf-8\r\n\r\n";
    let start = bytes.windows(PART.len()).position(|w| w == PART)? + PART.len();
    let rest = &bytes[start..];
    let end = rest.windows(2).position(|w| w == b"\r\n")?;
    if end <= 1 || start + end + 2 != bytes.len() {
        return None;
    }
    let mut short = bytes[..start].to_vec();
    short.extend_from_slice(b"x\r\n");
    Some(short)
}

/// Moves every name of `names` to its shorter body in `model`, the sizes
/// of the rows the old body derived restated.
fn rename(
    model: &mut Model,
    bodies: &BTreeMap<Vec<u8>, Vec<u8>>,
    names: &BTreeMap<ObjectName, (ObjectName, i64, i64)>,
) {
    let moved = |name: &mut Option<ObjectName>| {
        if let Some((new, _, _)) = name.as_ref().and_then(|name| names.get(name)) {
            *name = Some(new.clone());
        }
    };
    let objects = std::mem::take(&mut model.objects);
    model.objects = objects
        .into_iter()
        .map(|(name, bytes)| match bodies.get(&bytes) {
            Some(new) => (ObjectName::of(new), new.clone()),
            None => (name, bytes),
        })
        .collect();
    let words = std::mem::take(&mut model.words);
    model.words = words
        .into_iter()
        .map(|(name, words)| match names.get(&name) {
            Some((new, _, _)) => (new.clone(), words),
            None => (name, words),
        })
        .collect();
    let rows = std::mem::take(&mut model.rows);
    model.rows = rows
        .into_iter()
        .map(|(name, mut row)| match names.get(&name) {
            Some((new, old_size, new_size)) => {
                resize(&mut row.summary, *old_size, *new_size);
                (new.clone(), row)
            }
            None => (name, row),
        })
        .collect();
    let sizes: BTreeMap<(Option<String>, i64), i64> = names
        .iter()
        .filter_map(|(old, (_, old_size, new_size))| {
            let bytes = bodies.keys().find(|bytes| &ObjectName::of(bytes) == old)?;
            Some(((message_id(bytes), *old_size), *new_size))
        })
        .collect();
    for item in &mut model.items {
        moved(&mut item.object);
        moved(&mut item.conflict_object);
        for binding in &mut item.bindings {
            moved(&mut binding.base_object);
            moved(&mut binding.shared_object);
            moved(&mut binding.conflict_object);
        }
        if item.summary.table == Table::Mail
            && let Value::Integer(size) = item.summary.get("size")
        {
            let id = match item.summary.get("message_id") {
                Value::Text(id) => Some(id),
                _ => None,
            };
            if let Some(new) = sizes.get(&(id, size)) {
                item.summary.set("size", Value::Integer(*new));
            }
        }
    }
    for row in &mut model.queue {
        match &mut row.action {
            Action::Add { filed, .. } | Action::Update { filed, .. } => {
                if let Some(new) = bodies.get(&filed.bytes) {
                    let (old_size, new_size) = (filed.bytes.len() as i64, new.len() as i64);
                    filed.bytes = new.clone();
                    resize(&mut filed.summary, old_size, new_size);
                }
            }
            _ => {}
        }
    }
}

fn resize(summary: &mut crate::model::Summary, old: i64, new: i64) {
    if summary.table == Table::Mail && summary.get("size") == Value::Integer(old) {
        summary.set("size", Value::Integer(new));
    }
}

/// The bare `Message-ID` a generated message states, as its header writes
/// it.
fn message_id(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    let line = text
        .split("\r\n")
        .take_while(|line| !line.is_empty())
        .find(|line| line.starts_with("Message-ID: <"))?;
    Some(String::from(
        line.trim_start_matches("Message-ID: <")
            .trim_end_matches('>'),
    ))
}

/// Drops a collection, its items, its queue rows and the rows naming it.
fn drop_collection(model: &mut Model, id: &str) {
    model.collections.remove(id);
    model.items.retain(|item| item.collection != id);
    model.queue.retain(|row| {
        row.collection != id
            && !matches!(&row.action, Action::Move { to, .. } | Action::Copy { to, .. } if to == id)
    });
    model
        .capabilities
        .retain(|capability| capability.collection.as_deref() != Some(id));
    let held: BTreeSet<&str> = model
        .items
        .iter()
        .map(|item| item.link_id.as_str())
        .collect();
    model.references.retain(|reference| {
        held.contains(reference.from_link_id.as_str())
            && held.contains(reference.to_link_id.as_str())
    });
}

/// The queue's ids made dense again, in append order.
fn renumber(model: &mut Model) {
    for (at, row) in model.queue.iter_mut().enumerate() {
        row.id = at as i64 + 1;
    }
}

/// `model` with its queue's ids made dense again, as a writer appends
/// them.
pub fn dense(model: &Model) -> Model {
    let mut model = model.clone();
    renumber(&mut model);
    model
}

/// What a shrunk model keeps of the one it was shrunk from: replayable
/// from the seed, so a shrunk failure is written as the seed and this.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Kept {
    /// The collections kept, by id.
    pub collections: Vec<String>,
    /// The items kept, by collection and key.
    pub items: Vec<(String, String)>,
    /// The queue rows kept, by the id they were drawn with.
    pub queue: Vec<i64>,
    /// Whether the capabilities and performers are kept.
    pub capabilities: bool,
    /// Whether the references are kept.
    pub references: bool,
}

impl Kept {
    /// What `smaller` kept of the model it was shrunk from.
    pub fn of(smaller: &Model) -> Self {
        Self {
            collections: smaller.collections.keys().cloned().collect(),
            items: smaller
                .items
                .iter()
                .map(|item| (item.collection.clone(), item.link_id.clone()))
                .collect(),
            queue: smaller.queue.iter().map(|row| row.id).collect(),
            capabilities: !smaller.capabilities.is_empty() || !smaller.performers.is_empty(),
            references: !smaller.references.is_empty(),
        }
    }

    /// `model` cut to what this keeps, its queue numbered again.
    pub fn apply(&self, model: &Model) -> Model {
        let mut kept = model.clone();
        let dropped: Vec<String> = kept
            .collections
            .keys()
            .filter(|id| !self.collections.contains(id))
            .cloned()
            .collect();
        for id in &dropped {
            drop_collection(&mut kept, id);
        }
        kept.items.retain(|item| {
            self.items.iter().any(|(collection, link_id)| {
                &item.collection == collection && &item.link_id == link_id
            })
        });
        kept.queue.retain(|row| self.queue.contains(&row.id));
        if !self.capabilities {
            kept.capabilities.clear();
            kept.performers.clear();
        }
        if !self.references {
            kept.references.clear();
        }
        renumber(&mut kept);
        kept
    }
}

/// The smaller models to try in place of `model`, each named by what it
/// drops, the largest cuts first: for a failure no single engine step
/// holds (a read, the overlay, the drain, the index). The queue keeps the
/// ids its rows were drawn with, so [`Kept::of`] names them; [`dense`]
/// numbers them again before the model is written.
pub fn candidates(model: &Model) -> Vec<(String, Model)> {
    let mut candidates = Vec::new();
    if !model.capabilities.is_empty() || !model.performers.is_empty() {
        let mut smaller = model.clone();
        smaller.capabilities.clear();
        smaller.performers.clear();
        candidates.push((String::from("the capabilities and performers"), smaller));
    }
    if !model.references.is_empty() {
        let mut smaller = model.clone();
        smaller.references.clear();
        candidates.push((String::from("the references"), smaller));
    }
    for id in model.collections.keys() {
        let mut smaller = model.clone();
        drop_collection(&mut smaller, id);
        candidates.push((format!("collection {id}"), smaller));
    }
    if model.queue.len() > 1 {
        let half = model.queue.len() / 2;
        for range in [0..half, half..model.queue.len()] {
            let mut smaller = model.clone();
            let mut at = 0;
            smaller.queue.retain(|_| {
                at += 1;
                !range.contains(&(at - 1))
            });
            candidates.push((format!("{} queue rows", range.len()), smaller));
        }
    }
    for at in 0..model.queue.len() {
        let mut smaller = model.clone();
        let row = smaller.queue.remove(at);
        candidates.push((format!("queue row {}", row.id), smaller));
    }
    let items: Vec<&Item> = model.items.iter().collect();
    if items.len() > 1 {
        let half = items.len() / 2;
        for range in [0..half, half..items.len()] {
            let mut smaller = model.clone();
            let mut at = 0;
            smaller.items.retain(|_| {
                at += 1;
                !range.contains(&(at - 1))
            });
            candidates.push((format!("{} items", range.len()), smaller));
        }
    }
    for (at, item) in items.iter().enumerate() {
        let mut smaller = model.clone();
        smaller.items.remove(at);
        candidates.push((
            format!("item {} in {}", item.link_id, item.collection),
            smaller,
        ));
    }
    candidates
}

/// Makes `start` smaller while `fails` holds of it: each candidate tried
/// in turn, the first still failing kept and its own candidates tried
/// next, until none fails. Answers the smallest found and what each kept
/// step dropped, in order.
pub fn shrink<T: Clone>(
    start: T,
    candidates: impl Fn(&T) -> Vec<(String, T)>,
    mut fails: impl FnMut(&T) -> bool,
) -> (T, Vec<String>) {
    let mut current = start;
    let mut dropped = Vec::new();
    'smaller: loop {
        for (what, candidate) in candidates(&current) {
            if fails(&candidate) {
                current = candidate;
                dropped.push(what);
                continue 'smaller;
            }
        }
        return (current, dropped);
    }
}

/// What a difference's text says once every value that moves with the
/// store is left out: its words up to the first value, digits and long
/// names masked. Two differences of one signature are the same failure.
pub fn signature(text: &str) -> String {
    let cut = text
        .find(['[', '{', '(', '"'])
        .map_or(text, |at| &text[..at]);
    let mut out = String::new();
    for word in cut.split_whitespace() {
        let core = word.trim_end_matches([',', '.', ':', ';']);
        let masked: String = match core.len() > 15 && core.chars().all(char::is_alphanumeric) {
            true => String::from("*"),
            false => {
                let mut masked = String::new();
                for c in word.chars() {
                    match c.is_ascii_digit() {
                        true if masked.ends_with('#') => {}
                        true => masked.push('#'),
                        false => masked.push(c),
                    }
                }
                masked
            }
        };
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&masked);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::History;

    #[test]
    fn a_signature_masks_what_moves_with_the_store() {
        assert_eq!(
            signature("io-pimdir pushes [\"SetFlags 12\"], the model []"),
            "io-pimdir pushes",
        );
        assert_eq!(
            signature("storage/read/list_items at {\"collection\": 1}"),
            "storage/read/list_items at",
        );
        assert_eq!(
            signature("a row names abcdefghijklmnopqrstuvwxyz234567, a body the store lacks"),
            "a row names * a body the store lacks",
        );
        assert_eq!(signature("row 12 add on x"), "row # add on x");
    }

    #[test]
    fn shrinking_keeps_the_smallest_still_failing() {
        let (smallest, dropped) = shrink(
            (0..20).collect::<Vec<u32>>(),
            |list| {
                (0..list.len())
                    .map(|at| {
                        let mut smaller = list.clone();
                        smaller.remove(at);
                        (format!("{at}"), smaller)
                    })
                    .collect()
            },
            |list| list.contains(&7) && list.contains(&13),
        );
        assert_eq!(smallest, vec![7, 13]);
        assert_eq!(dropped.len(), 18);
    }

    #[test]
    fn a_shortened_case_keeps_its_keys_and_sizes_in_step() {
        let history = History::mail(7);
        let step = history
            .steps
            .iter()
            .position(|step| step.run.as_ref().is_some_and(|run| !run.pushes.is_empty()))
            .expect("a run pushing");
        let case = Case::of(&history, step).expect("a run");
        let short = case.shortened().expect("generated messages");
        for (push, key) in &short.run.pushes {
            assert_eq!(key, &push.key(&short.run.collection));
        }
        for model in [&short.before, &short.after] {
            for (name, bytes) in &model.objects {
                assert_eq!(name, &ObjectName::of(bytes));
            }
            for item in &model.items {
                if let (Some(name), Value::Integer(size)) = (&item.object, item.summary.get("size"))
                {
                    assert_eq!(model.objects[name].len() as i64, size);
                }
            }
        }
    }

    #[test]
    fn what_a_shrunk_model_kept_replays_from_its_seed() {
        let spec = crate::spec::Spec::load_default().expect("the vendored spec");
        let model = crate::profile::Profile::Structure
            .generate(&spec, 7)
            .expect("a structure model");
        let mut smaller = model.clone();
        for step in 0..6 {
            smaller = candidates(&smaller).swap_remove(step * 3).1;
            let again = Kept::of(&smaller).apply(&model);
            let want = dense(&smaller);
            assert_eq!(Kept::of(&again), Kept::of(&want), "step {step}");
            assert_eq!(format!("{:?}", again.queue), format!("{:?}", want.queue));
        }
    }

    #[test]
    fn a_case_drops_no_item_its_run_names() {
        let history = History::mail(42);
        for step in 1..history.steps.len() {
            let Some(case) = Case::of(&history, step) else {
                continue;
            };
            let named: BTreeSet<String> = case
                .run
                .pushes
                .iter()
                .map(|(push, _)| String::from(push.handle()))
                .collect();
            for (collection, link_id) in case.droppable() {
                let item = case
                    .before
                    .items
                    .iter()
                    .find(|item| item.collection == collection && item.link_id == link_id)
                    .expect("a held item");
                assert!(
                    item.bindings
                        .iter()
                        .all(|binding| !named.contains(&binding.handle)),
                    "step {step}: {link_id}"
                );
            }
        }
    }
}
