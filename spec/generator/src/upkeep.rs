//! # Owner upkeep
//!
//! The owner's writes outside a sync, on the user's request, read from
//! STORAGE §11 and §14 and applied to the model: a purge, a sort key or a
//! name restated, a rename, a release and an eviction below a date, an
//! erase, a collection deleted, the collector. Each says whether it found
//! what it names, as the owner reports it.

use std::{collections::BTreeSet, mem};

use crate::{
    model::{Kind, Model, State},
    name::ObjectName,
    queue::{Action, RowState},
};

impl Model {
    /// Purges the retained row `seq` of `collection`, the one true delete
    /// (STORAGE §11.2): a live row or a tombstone is never reached.
    pub fn purge(&mut self, collection: &str, seq: i64) -> bool {
        let Some(index) = self.items.iter().position(|item| {
            item.collection == collection
                && item.seq == seq
                && matches!(item.state, State::Retained { .. })
        }) else {
            return false;
        };

        self.items.remove(index);
        self.purges += 1;
        self.drop_references();
        true
    }

    /// Restates the sort key of the item `link_id` holds in `collection`
    /// (STORAGE §9.3).
    pub fn set_sort_key(&mut self, collection: &str, link_id: &str, sort_key: &str) {
        for item in &mut self.items {
            if item.collection == collection && item.link_id == link_id {
                item.sort_key = String::from(sort_key);
            }
        }
    }

    /// Renames a collection's display name.
    pub fn set_collection_name(&mut self, collection: &str, name: &str) {
        if let Some(collection) = self.collections.get_mut(collection) {
            collection.name = String::from(name);
        }
    }

    /// Gives `from` the id `to` (STORAGE §14): its items, the collections
    /// under it and the queue's target column follow, the payload keeping
    /// the id it was enqueued with, and the rename counts in `purges`.
    pub fn rename_collection(&mut self, from: &str, to: &str) {
        let Some(mut collection) = self.collections.remove(from) else {
            return;
        };
        collection.id = String::from(to);
        self.collections.insert(String::from(to), collection);

        for collection in self.collections.values_mut() {
            if collection.parent.as_deref() == Some(from) {
                collection.parent = Some(String::from(to));
            }
        }
        for item in &mut self.items {
            if item.collection == from {
                item.collection = String::from(to);
            }
        }
        for row in &mut self.queue {
            if row.collection == from {
                row.collection = String::from(to);
            }
            if let Action::Move { to: target, .. } | Action::Copy { to: target, .. } =
                &mut row.action
                && target == from
            {
                *target = String::from(to);
            }
        }
        for capability in &mut self.capabilities {
            if capability.collection.as_deref() == Some(from) {
                capability.collection = Some(String::from(to));
            }
        }
        self.purges += 1;
    }

    /// Releases the bodies of `collections`' live mail items sorting below
    /// `before`, every one with no ceiling, that do not need them (STORAGE
    /// §11.4), answering the `seq`s released.
    pub fn release_before(&mut self, collections: &[&str], before: Option<&str>) -> Vec<i64> {
        let mut released = Vec::new();
        for index in 0..self.items.len() {
            let item = &self.items[index];
            let Some(object) = item.object.clone() else {
                continue;
            };
            let collection = &self.collections[&item.collection];
            let below = before.is_none_or(|before| item.sort_key.as_str() < before);
            if !collections.contains(&item.collection.as_str())
                || collection.kind != Some(Kind::Mail)
                || item.state != State::Live
                || !below
            {
                continue;
            }

            let needs = item.conflicted
                || item.bindings.is_empty()
                || item.bindings.iter().any(|binding| {
                    binding.conflicted
                        || !binding.base_present
                        || binding
                            .base_object
                            .as_ref()
                            .is_some_and(|base| *base != object)
                })
                || collection
                    .sources
                    .keys()
                    .any(|source| item.bound(source).is_none());
            if needs {
                continue;
            }

            let item = &mut self.items[index];
            item.object = None;
            item.level = 1;
            for binding in &mut item.bindings {
                if binding.base_object.as_ref() == Some(&object) {
                    binding.base_object = None;
                }
            }
            released.push(item.seq);
        }
        released
    }

    /// Evicts `collection`'s live mail items dated before `before` that
    /// owe nothing (STORAGE §11.3), answering the `seq`s evicted.
    pub fn evict_before(&mut self, collection: &str, before: &str) -> Vec<i64> {
        let named: BTreeSet<(Kind, String)> = self
            .references
            .iter()
            .filter(|reference| reference.origin == "user")
            .flat_map(|reference| {
                [
                    (reference.from_kind, reference.from_link_id.clone()),
                    (reference.to_kind, reference.to_link_id.clone()),
                ]
            })
            .collect();

        let mut evicted = Vec::new();
        let mut kept = Vec::new();
        for item in mem::take(&mut self.items) {
            let mail = self.collections[&item.collection].kind == Some(Kind::Mail);
            let older = item.date().is_some_and(|date| date.as_str() < before);
            let owes = item.bindings.is_empty()
                || item.conflicted
                || item.bindings.iter().any(|binding| {
                    binding.conflicted
                        || !binding.base_present
                        || binding.handle.starts_with('\u{1}')
                        || match (&item.flags, &binding.base_flags) {
                            (Some(flags), Some(base)) => flags != base,
                            _ => false,
                        }
                })
                || named.contains(&(Kind::Mail, item.link_id.clone()));
            if item.collection == collection && mail && item.state == State::Live && older && !owes
            {
                evicted.push(item.seq);
                self.purges += 1;
            } else {
                kept.push(item);
            }
        }
        self.items = kept;
        self.drop_references();
        evicted
    }

    /// Erases the item `seq` of `collection` (STORAGE §11.5): its purge
    /// when retained, its delete when live and bound by no source, then
    /// the collector; a live item a source binds is left as it stands.
    pub fn erase(&mut self, collection: &str, seq: i64) -> bool {
        let erased = match self
            .items
            .iter()
            .position(|item| item.collection == collection && item.seq == seq)
        {
            Some(index) if matches!(self.items[index].state, State::Retained { .. }) => {
                self.purge(collection, seq)
            }
            Some(index)
                if self.items[index].state == State::Live
                    && self.items[index].bindings.is_empty() =>
            {
                self.items.remove(index);
                self.purges += 1;
                self.drop_references();
                true
            }
            _ => false,
        };
        self.collect();
        erased
    }

    /// Deletes a collection with everything under it (STORAGE §14),
    /// answering whether there was one: the collection and each row under
    /// it count in `purges` (§4.5).
    pub fn delete_collection(&mut self, collection: &str) -> bool {
        if self.collections.remove(collection).is_none() {
            return false;
        }
        let held = self.items.len();
        self.items.retain(|item| item.collection != collection);
        self.capabilities
            .retain(|capability| capability.collection.as_deref() != Some(collection));
        self.purges += 1 + (held - self.items.len()) as i64;
        self.drop_references();
        true
    }

    /// Runs the collector (STORAGE §5): every body no row names goes, each
    /// counting in `purges`, a queue row pinning its body until applied.
    pub fn collect(&mut self) {
        let mut named: BTreeSet<ObjectName> = BTreeSet::new();
        for item in &self.items {
            named.extend(item.object.iter().cloned());
            named.extend(item.conflict_object.iter().cloned());
            for binding in &item.bindings {
                named.extend(binding.base_object.iter().cloned());
                named.extend(binding.conflict_object.iter().cloned());
                named.extend(binding.shared_object.iter().cloned());
            }
        }
        for row in &self.queue {
            if matches!(row.state, RowState::Applied(_)) {
                continue;
            }
            if let Some(body) = row.action.body() {
                named.insert(ObjectName::of(body));
            }
        }

        let before = self.objects.len();
        self.objects.retain(|name, _| named.contains(name));
        self.purges += (before - self.objects.len()) as i64;
    }

    /// Drops every reference one of whose ends no row holds any more
    /// (STORAGE §14.2, `items_drop_references`).
    fn drop_references(&mut self) {
        let held: BTreeSet<(Kind, &str)> = self
            .items
            .iter()
            .filter_map(|item| {
                let kind = self.collections.get(&item.collection)?.kind?;
                Some((kind, item.link_id.as_str()))
            })
            .collect();
        let references = mem::take(&mut self.references);
        self.references = references
            .into_iter()
            .filter(|reference| {
                held.contains(&(reference.from_kind, reference.from_link_id.as_str()))
                    && held.contains(&(reference.to_kind, reference.to_link_id.as_str()))
            })
            .collect();
    }
}
