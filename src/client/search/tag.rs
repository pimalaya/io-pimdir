//! # Tags
//!
//! Tagging from a query over the index (SEARCH §10): [`PimdirIndex::tag`]
//! answers a [`PimdirSearchTagPlan`] without writing, so a caller can
//! show it before it writes, and [`PimdirSearchTagPlan::stage`] appends it
//! through a producer. The plan's types are
//! [`search::tag`](crate::search::tag). Applying it is the owner's: a
//! drain, or the next sync.

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::{String, ToString},
    vec::Vec,
};

use log::{debug, trace};

use crate::{
    capability::{self, PimdirSourceCapabilities, PimdirSupport},
    client::{
        PimdirError,
        producer::{PimdirPendingAction, PimdirProducer},
        search::{Found, PimdirIndex, PimdirSearchClock, PimdirSearchSort},
    },
    codec::PimdirAction,
    search::{
        query::PimdirQuery,
        tag::{
            self, PimdirSearchTagAction, PimdirSearchTagError, PimdirSearchTagHit,
            PimdirSearchTagPlacement, PimdirSearchTagPlan, PimdirSearchTagSource,
        },
    },
};

/// What a tag reads of one collection: the source its actions are staged
/// as, its kind, and that source's declaration there.
struct Collection {
    source: PimdirSearchTagSource,
    kind: String,
    declared: Vec<PimdirSourceCapabilities>,
}

impl PimdirIndex {
    /// The plan of tagging every hit of a query (SEARCH §10): `add` added
    /// to and `remove` removed from each live placement's flags, the
    /// `is:` spellings `read`, `flagged`, `answered` and `draft` read as
    /// `\Seen`, `\Flagged`, `\Answered` and `\Draft` and any other value
    /// as written. Writes nothing; answers from the index as it stands.
    ///
    /// Each placement's action is staged as a source syncing its
    /// collection: `source` when it does, else the only one; several and
    /// no `source`, or a `source` not among them, leave the placement
    /// unstaged ([`PimdirSearchTagSource`]).
    ///
    /// An empty change, a flag both added and removed, and a value naming
    /// no flag are [`PimdirError::Tag`].
    pub fn tag(
        &self,
        query: &PimdirQuery,
        add: &[String],
        remove: &[String],
        source: Option<&str>,
        clock: &PimdirSearchClock,
    ) -> Result<PimdirSearchTagPlan, PimdirError> {
        let add = tag::flags(add)?;
        let remove = tag::flags(remove)?;
        if add.is_empty() && remove.is_empty() {
            return Err(PimdirSearchTagError::Empty.into());
        }
        if let Some(flag) = add.intersection(&remove).next() {
            return Err(PimdirSearchTagError::Both(flag.clone()).into());
        }
        debug!("planning a tag adding {add:?} and removing {remove:?}");

        let now = clock.now();
        let mut compiled = self.compile(query, now)?;
        let found = self.found(&mut compiled, PimdirSearchSort::Date, None)?;

        let mut pending: BTreeMap<String, Vec<PimdirPendingAction>> = BTreeMap::new();
        let mut sources: BTreeMap<String, Collection> = BTreeMap::new();
        let mut hits = Vec::new();
        for Found { account, seq, .. } in found {
            let (kind, title, held) = self.placements(account.as_deref(), seq)?;
            let mut placements = Vec::new();
            for mut placement in held {
                if !pending.contains_key(&placement.collection) {
                    let actions = self.reader.pending_actions(&placement.collection)?;
                    pending.insert(placement.collection.clone(), actions);
                    let syncing = self.reader.collection_sources(&placement.collection)?;
                    let resolved = match (source, syncing.as_slice()) {
                        (Some(asked), syncing) if syncing.iter().any(|s| s == asked) => {
                            PimdirSearchTagSource::Resolved(asked.to_string())
                        }
                        (None, [only]) => PimdirSearchTagSource::Resolved(only.clone()),
                        (None, [_, _, ..]) => PimdirSearchTagSource::Ambiguous(syncing),
                        _ => PimdirSearchTagSource::Unsynced(syncing),
                    };
                    trace!("tag stages {} as {resolved:?}", placement.collection);
                    let kind = self
                        .reader
                        .collection_kind(&placement.collection)?
                        .unwrap_or_default();
                    let declared = match &resolved {
                        PimdirSearchTagSource::Resolved(source) => self
                            .reader
                            .capabilities(&placement.collection)?
                            .into_iter()
                            .filter(|declared| &declared.source == source)
                            .collect(),
                        _ => Vec::new(),
                    };
                    let collection = Collection {
                        source: resolved,
                        kind,
                        declared,
                    };
                    sources.insert(placement.collection.clone(), collection);
                }
                // NOTE: the flags a pending set-flags will set, so a tag
                // staged behind it builds on it rather than undoing it
                // (STORAGE §15.4).
                let staged = pending[&placement.collection]
                    .iter()
                    .rev()
                    .find_map(|pending| match &pending.action {
                        PimdirAction::SetFlags { seq: at, flags } if *at == seq => {
                            Some(flags.clone())
                        }
                        _ => None,
                    });
                if let Some(flags) = staged {
                    placement.flags = flags;
                }

                let next: Option<BTreeSet<String>> = placement.flags.known().map(|flags| {
                    flags
                        .union(&add)
                        .filter(|flag| !remove.contains(*flag))
                        .cloned()
                        .collect()
                });
                let collection = &sources[&placement.collection];
                // NOTE: flags exist on the wire for mail alone (Annex
                // B.1); an undeclared source is not gated, so it is taken
                // at its word as the gate takes it.
                let (syncs, missing) = match (&collection.source, &next) {
                    (PimdirSearchTagSource::Resolved(_), Some(next))
                        if collection.kind == "message/rfc822" =>
                    {
                        let action = PimdirAction::SetFlags {
                            seq,
                            flags: next.iter().collect(),
                        };
                        let needed =
                            capability::required(&collection.kind, &action, &placement.flags);
                        let missing: Vec<String> = needed
                            .into_iter()
                            .filter(|needed| {
                                collection.declared.iter().any(|source| {
                                    source.declared.as_ref().is_some_and(|declared| {
                                        !matches!(
                                            declared.get(needed),
                                            Some((PimdirSupport::Full | PimdirSupport::Partial, _))
                                        )
                                    })
                                })
                            })
                            .collect();
                        (missing.is_empty(), missing)
                    }
                    _ => (false, Vec::new()),
                };
                let source = collection.source.clone();
                placements.push(PimdirSearchTagPlacement {
                    collection: placement.collection,
                    link_id: placement.link_id,
                    flags: placement.flags,
                    next,
                    source,
                    syncs,
                    missing,
                });
            }
            hits.push(PimdirSearchTagHit {
                account,
                seq,
                kind,
                title,
                placements,
            });
        }
        trace!("tag reaches {} hit(s)", hits.len());

        Ok(PimdirSearchTagPlan {
            add,
            remove,
            hits,
            coverage: self.coverage(query, now)?,
        })
    }
}

impl PimdirSearchTagPlan {
    /// Appends one `set-flags` action per placement the plan changes,
    /// resolves a source for and finds no capability `missing` for,
    /// through `producer` (STORAGE §15), answering them in plan order.
    ///
    /// Every action passes the capability gate before any is appended, so
    /// one a source refuses ([`PimdirError::Unsupported`]), a declaration
    /// changed since the plan, leaves the queue as it was. A placement
    /// whose flags were never read is left out, as is one the change leaves
    /// as it is.
    pub fn stage(
        &self,
        producer: &mut PimdirProducer,
    ) -> Result<Vec<PimdirSearchTagAction>, PimdirError> {
        let mut staged = Vec::new();
        for hit in &self.hits {
            for placement in hit.placements.iter().filter(|p| p.changes()) {
                let PimdirSearchTagSource::Resolved(source) = &placement.source else {
                    continue;
                };
                if !placement.missing.is_empty() {
                    continue;
                }
                let flags = placement.next.clone().unwrap_or_default();
                let action = PimdirAction::SetFlags {
                    seq: hit.seq,
                    flags: flags.iter().collect(),
                };
                producer.check(&placement.collection, &action)?;
                staged.push((
                    placement.collection.clone(),
                    hit.seq,
                    source.clone(),
                    flags,
                    action,
                ));
            }
        }
        debug!("staging {} set-flags action(s)", staged.len());

        let mut actions = Vec::new();
        for (collection, seq, source, flags, action) in staged {
            let id = producer.enqueue(&collection, &action, None)?;
            trace!("staged action {id} on {collection} seq {seq}");
            actions.push(PimdirSearchTagAction {
                collection,
                seq,
                source,
                flags,
                action: id,
            });
        }
        Ok(actions)
    }
}
