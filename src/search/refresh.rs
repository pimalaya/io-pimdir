//! # Refresh
//!
//! I/O-free coroutine folding a store's change feed into its index
//! (SEARCH §4): one batch of the feed per run, read and written through
//! typed yields, never SQL, so `PimdirIndex` over rusqlite
//! (`client::search`) and an owner holding its own SQLite in another
//! language run one fold.
//!
//! A run asks the index's state ([`WantsIndexState`]), rolls the horizon
//! when it moved ([`WantsReexpand`]), reads one page of the feed
//! ([`WantsFeed`]) with what the index holds of it ([`WantsHeld`]), the
//! bodies it extracts and expands ([`WantsBodies`]) and the summaries a
//! bodiless or unreadable item is indexed from ([`WantsSummaries`]), then
//! relinks the threads the page touched ([`WantsMessages`]). The run that
//! exhausts the feed also reconciles what the store no longer holds
//! ([`WantsGone`]) and meets again the bodies an earlier pass could not
//! read ([`WantsUnread`]). Every write is a [`PimdirIndexOp`] naming the
//! index's rows, its text already folded (SEARCH §5), which a consumer
//! maps to its own shape's statements.
//!
//! One run is one transaction: a consumer answers every yield of a run
//! inside one write transaction on the index, the store read in it as one
//! snapshot, and commits when the run completes. [`Writes`] comes before
//! any read the ops it carries change the answer of, so a consumer
//! applies them in order as they come and reads its own writes. A run
//! completes with the cursor it recorded and whether the feed was
//! exhausted, so a caller runs again or stops within its deadline.
//!
//! [`WantsIndexState`]: PimdirIndexRefreshYield::WantsIndexState
//! [`WantsReexpand`]: PimdirIndexRefreshYield::WantsReexpand
//! [`WantsFeed`]: PimdirIndexRefreshYield::WantsFeed
//! [`WantsHeld`]: PimdirIndexRefreshYield::WantsHeld
//! [`WantsBodies`]: PimdirIndexRefreshYield::WantsBodies
//! [`WantsSummaries`]: PimdirIndexRefreshYield::WantsSummaries
//! [`WantsMessages`]: PimdirIndexRefreshYield::WantsMessages
//! [`WantsGone`]: PimdirIndexRefreshYield::WantsGone
//! [`WantsUnread`]: PimdirIndexRefreshYield::WantsUnread
//! [`Writes`]: PimdirIndexRefreshYield::Writes

use core::mem;

use alloc::{
    boxed::Box,
    collections::{BTreeMap, BTreeSet},
    string::String,
    vec::Vec,
};

use log::{debug, trace};

use crate::{
    coroutine::{PimdirArgError, PimdirCoroutineState},
    object::PimdirHash,
    placement::{PimdirFlags, PimdirLinkId},
    search::{
        extract::{PimdirChunk, PimdirExtract, PimdirExtractStatus},
        fold,
        occurrence::{self, Horizon},
        thread::{self, Heads},
    },
    summary::{PimdirAddress, PimdirAddressRole, PimdirSummary, file, time},
};

/// The most bodies one [`WantsBodies`](PimdirIndexRefreshYield::WantsBodies)
/// asks for, so a consumer holds a bounded number of them at once.
pub const BODIES_MAX: usize = 16;

/// I/O-free coroutine folding one batch of a store's change feed into its
/// index (SEARCH §4).
pub struct PimdirIndexRefresh {
    now: i64,
    batch: usize,
    state: State,
    /// The ops not yielded yet.
    ops: Vec<PimdirIndexOp>,
    output: PimdirIndexRefreshOutput,
    meta: PimdirIndexMeta,
    horizon: Horizon,
    /// The stamp of the page's last row.
    end: Option<i64>,
    /// Whether the page was full, the feed then not exhausted.
    full: bool,
    /// The bodies this run put an object for.
    put: BTreeSet<PimdirHash>,
    /// The message ids to relink (SEARCH §9).
    touched: BTreeSet<String>,
}

impl PimdirIndexRefresh {
    /// Creates a run folding at most `batch` feed rows, its horizon drawn
    /// around `now` (Unix seconds, SEARCH §7).
    pub fn new(now: i64, batch: usize) -> Self {
        Self {
            now,
            batch: batch.max(1),
            state: State::Start,
            ops: Vec::new(),
            output: PimdirIndexRefreshOutput::default(),
            meta: PimdirIndexMeta::default(),
            horizon: Horizon::around(now),
            end: None,
            full: false,
            put: BTreeSet::new(),
            touched: BTreeSet::new(),
        }
    }

    /// Advances one step, with the arg matching the previous yield;
    /// [`None`] on the initial call only.
    pub fn resume(
        &mut self,
        arg: Option<PimdirIndexRefreshArg>,
    ) -> PimdirCoroutineState<
        PimdirIndexRefreshYield,
        Result<PimdirIndexRefreshOutput, PimdirArgError>,
    > {
        let state = mem::replace(&mut self.state, State::Done);
        match (state, arg) {
            (State::Start, None) => {
                debug!("read the index state");
                self.state = State::ReadingMeta;
                PimdirCoroutineState::Yielded(PimdirIndexRefreshYield::WantsIndexState)
            }
            (State::ReadingMeta, Some(PimdirIndexRefreshArg::IndexState(meta))) => {
                trace!("index state: {meta:?}");
                self.roll(meta)
            }
            (State::ListingReexpand { new }, Some(PimdirIndexRefreshArg::Reexpand(keys))) => {
                debug!("re-expand {} calendar item(s)", keys.len());
                if keys.is_empty() {
                    self.record(new);
                    return self.feed();
                }
                let want = PimdirIndexRefreshYield::WantsHeld {
                    placements: keys.clone(),
                    objects: Vec::new(),
                };
                self.ask(want, State::ReadingReexpandHeld { new, keys })
            }
            (
                State::ReadingReexpandHeld { new, keys },
                Some(PimdirIndexRefreshArg::Held { placements, .. }),
            ) => {
                let held: BTreeMap<PimdirIndexKey, Option<PimdirHash>> = placements
                    .into_iter()
                    .map(|placement| (placement.key(), placement.hash))
                    .collect();
                let items: Vec<(PimdirIndexKey, Option<PimdirHash>)> = keys
                    .into_iter()
                    .map(|key| {
                        let hash = held.get(&key).cloned().flatten();
                        (key, hash)
                    })
                    .collect();
                let queue: BTreeSet<PimdirHash> =
                    items.iter().filter_map(|(_, hash)| hash.clone()).collect();
                let reexpand = Reexpand {
                    new,
                    items,
                    queue: queue.into_iter().collect(),
                    asked: Vec::new(),
                    derived: BTreeMap::new(),
                };
                self.reexpand(Box::new(reexpand))
            }
            (State::Expanding(mut reexpand), Some(PimdirIndexRefreshArg::Bodies(bodies))) => {
                for hash in mem::take(&mut reexpand.asked) {
                    if let Some(body) = bodies.get(&hash) {
                        let derived = reexpand.new.occurrences(body);
                        reexpand.derived.insert(hash, derived);
                    }
                }
                self.reexpand(reexpand)
            }
            (State::ReadingFeed, Some(PimdirIndexRefreshArg::Feed(rows))) => {
                debug!("fold {} feed row(s)", rows.len());
                self.full = rows.len() >= self.batch;
                self.end = rows.last().map(|row| row.changed);
                self.output.folded += rows.len() as u64;
                self.fold(rows, After::Feed)
            }
            (State::ReadingUnread, Some(PimdirIndexRefreshArg::Feed(rows))) => {
                debug!("fold {} row(s) whose body was not read", rows.len());
                self.fold(rows, After::Unread)
            }
            (
                State::ReadingHeld(mut fold),
                Some(PimdirIndexRefreshArg::Held {
                    placements,
                    objects,
                }),
            ) => {
                fold.held = placements
                    .into_iter()
                    .map(|placement| (placement.key(), placement))
                    .collect();
                fold.indexed = objects;
                fold.plan(&self.put);
                self.bodies(fold)
            }
            (State::ReadingBodies(mut fold), Some(PimdirIndexRefreshArg::Bodies(bodies))) => {
                fold.read(&bodies, &self.horizon);
                self.bodies(fold)
            }
            (State::ReadingSummaries(mut fold), Some(PimdirIndexRefreshArg::Summaries(found))) => {
                fold.summaries = found;
                self.emit(*fold)
            }
            (State::ReadingGone, Some(PimdirIndexRefreshArg::Gone(gone))) => {
                debug!(
                    "drop {} placement(s) and {} object(s) the store no longer holds",
                    gone.placements.len(),
                    gone.objects.len()
                );
                self.output.reconciled = true;
                for placement in gone.placements {
                    self.touched.insert(placement.link_id.0);
                    self.output.removed += 1;
                    self.ops.push(PimdirIndexOp::DropPlacement {
                        collection: placement.collection,
                        seq: placement.seq,
                    });
                }
                self.ops
                    .extend(gone.objects.into_iter().map(PimdirIndexOp::DropObject));
                self.unread()
            }
            (State::Relinking(mut relink), Some(PimdirIndexRefreshArg::Messages(mut read))) => {
                for link_id in mem::take(&mut relink.messages) {
                    let message = read.remove(&link_id).unwrap_or_default();
                    relink.nodes.insert(link_id, message);
                }
                self.relink(relink)
            }
            (State::Relinking(mut relink), Some(PimdirIndexRefreshArg::Bodies(bodies))) => {
                for hash in mem::take(&mut relink.bodies) {
                    let head = bodies.get(&hash).and_then(|body| thread::parent(body));
                    relink.heads.insert(hash, head);
                }
                self.relink(relink)
            }
            (State::Writing(then), Some(PimdirIndexRefreshArg::Written)) => {
                let (want, next) = *then;
                self.state = next;
                PimdirCoroutineState::Yielded(want)
            }
            (State::Finishing, Some(PimdirIndexRefreshArg::Written)) => {
                debug!(
                    "refreshed one batch: {} row(s) folded, cursor {}",
                    self.output.folded, self.output.cursor
                );
                PimdirCoroutineState::Complete(Ok(self.output))
            }
            (State::Done, _) | (_, Some(_)) => {
                PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg))
            }
            (_, None) => PimdirCoroutineState::Complete(Err(PimdirArgError::MissingArg)),
        }
    }

    /// Asks `want` once the ops pending are written, so its answer sees
    /// them, the run going on in `next`.
    fn ask(&mut self, want: PimdirIndexRefreshYield, next: State) -> Step {
        if self.ops.is_empty() {
            self.state = next;
            return PimdirCoroutineState::Yielded(want);
        }

        let ops = mem::take(&mut self.ops);
        trace!("write {} index op(s)", ops.len());
        self.state = State::Writing(Box::new((want, next)));
        PimdirCoroutineState::Yielded(PimdirIndexRefreshYield::Writes(ops))
    }

    /// Rolls the horizon to the one around `now` when it moved by a day or
    /// more, re-expanding what `items_to_reexpand` names when it rolled or
    /// the time zone database names another version than the one recorded
    /// (SEARCH §7), then reads the feed.
    fn roll(&mut self, meta: PimdirIndexMeta) -> Step {
        let around = Horizon::around(self.now);
        let version = occurrence::tzdb_version();
        let old = meta.horizon.as_ref().and_then(|horizon| {
            Some(Horizon {
                start: occurrence::instant(&horizon.start)?,
                end: occurrence::instant(&horizon.end)?,
            })
        });
        let recorded = meta.tzdb_version.clone();
        self.meta = meta;

        let Some(old) = old else {
            self.record(around);
            return self.feed();
        };
        let rolled = (around.start - old.start).abs() >= occurrence::ROLL;
        if !rolled && recorded == version {
            self.horizon = old;
            return self.feed();
        }

        let new = if rolled { around } else { old };
        debug!(
            "re-expand calendar items for the horizon {}..{} under tzdb {version:?}",
            time::rfc3339(new.start),
            time::rfc3339(new.end)
        );
        self.horizon = new;
        let want = PimdirIndexRefreshYield::WantsReexpand {
            old: PimdirIndexHorizon::from(old),
            new: PimdirIndexHorizon::from(new),
            tzdb_version: version.into(),
        };
        self.ask(want, State::ListingReexpand { new })
    }

    /// Records the horizon and the time zone database version in force.
    fn record(&mut self, horizon: Horizon) {
        self.horizon = horizon;
        self.ops
            .push(PimdirIndexOp::SetHorizon(PimdirIndexHorizon::from(horizon)));
        self.ops.push(PimdirIndexOp::SetTzdbVersion(
            occurrence::tzdb_version().into(),
        ));
    }

    /// Asks the next bodies a re-expansion needs, else replaces the
    /// occurrences of every item it names and records the new horizon.
    fn reexpand(&mut self, mut reexpand: Box<Reexpand>) -> Step {
        let take = reexpand.queue.len().min(BODIES_MAX);
        if take > 0 {
            reexpand.asked = reexpand.queue.drain(..take).collect();
            let want = PimdirIndexRefreshYield::WantsBodies {
                hashes: reexpand.asked.clone(),
            };
            return self.ask(want, State::Expanding(reexpand));
        }

        let Reexpand {
            new,
            items,
            derived,
            ..
        } = *reexpand;
        for (key, hash) in items {
            let derived = hash.and_then(|hash| derived.get(&hash));
            let op = self.occurrences(&key, derived);
            self.ops.push(op);
        }
        self.record(new);
        self.feed()
    }

    /// The op replacing one placement's occurrences with those `derived`
    /// holds, none without; counted when expanded.
    fn occurrences(
        &mut self,
        key: &PimdirIndexKey,
        derived: Option<&Occurrences>,
    ) -> PimdirIndexOp {
        let mut occurrences = Vec::new();
        if let Some((instants, cut)) = derived {
            trace!(
                "{} occurrence(s) of {}/{}",
                instants.len(),
                key.collection,
                key.seq
            );
            occurrences = instants
                .iter()
                .map(|(start, end)| PimdirIndexOccurrence {
                    start: time::rfc3339(*start),
                    end: time::rfc3339(*end),
                })
                .collect();
            self.output.expanded += 1;
            if *cut {
                self.output.bounded += 1;
            }
        }
        PimdirIndexOp::ReplaceOccurrences {
            collection: key.collection.clone(),
            seq: key.seq,
            occurrences,
        }
    }

    /// Reads one page of the feed above the cursor.
    fn feed(&mut self) -> Step {
        let want = PimdirIndexRefreshYield::WantsFeed {
            since: self.meta.store_change,
            limit: self.batch,
        };
        self.ask(want, State::ReadingFeed)
    }

    /// Reads what the index holds of `rows`, so they are folded.
    fn fold(&mut self, rows: Vec<PimdirIndexFeedRow>, after: After) -> Step {
        if rows.is_empty() {
            return self.after(after);
        }

        let placements: BTreeSet<PimdirIndexKey> =
            rows.iter().map(PimdirIndexFeedRow::key).collect();
        let objects: BTreeSet<PimdirHash> = rows
            .iter()
            .filter(|row| row.is_live())
            .filter_map(PimdirIndexFeedRow::indexable)
            .filter(|hash| !self.put.contains(*hash))
            .cloned()
            .collect();
        let want = PimdirIndexRefreshYield::WantsHeld {
            placements: placements.into_iter().collect(),
            objects: objects.into_iter().collect(),
        };
        let fold = Fold {
            rows,
            after,
            held: BTreeMap::new(),
            indexed: BTreeSet::new(),
            extract: BTreeMap::new(),
            expand: BTreeSet::new(),
            queue: Vec::new(),
            asked: Vec::new(),
            extracted: BTreeMap::new(),
            derived: BTreeMap::new(),
            summaries: BTreeMap::new(),
        };
        self.ask(want, State::ReadingHeld(Box::new(fold)))
    }

    /// Asks the next bodies a fold needs, else the summaries it lacks,
    /// else folds.
    fn bodies(&mut self, mut fold: Box<Fold>) -> Step {
        let take = fold.queue.len().min(BODIES_MAX);
        if take > 0 {
            fold.asked = fold.queue.drain(..take).collect();
            let want = PimdirIndexRefreshYield::WantsBodies {
                hashes: fold.asked.clone(),
            };
            return self.ask(want, State::ReadingBodies(fold));
        }

        let lacking = fold.lacking(&self.put);
        if lacking.is_empty() {
            return self.emit(*fold);
        }
        let want = PimdirIndexRefreshYield::WantsSummaries {
            placements: lacking,
        };
        self.ask(want, State::ReadingSummaries(fold))
    }

    /// Folds every row of a page (SEARCH §4): a gone row loses its
    /// placement, a live one has its body indexed when new, its placement
    /// upserted, its summary text written while it has no body and dropped
    /// once it has one, its occurrences re-expanded when its body changed
    /// and its flags replaced. A message placed, rebodied, rekeyed or
    /// dropped is touched, to be rethreaded before the run completes.
    fn emit(&mut self, mut fold: Fold) -> Step {
        let mut rows = mem::take(&mut fold.rows);
        for row in &mut rows {
            if let Some(summary) = row.summary.take() {
                fold.summaries.insert(row.key(), summary);
            }
        }

        for row in rows {
            let key = row.key();
            let held = fold.held.remove(&key);

            if !row.is_live() {
                if let Some(held) = held {
                    self.touched.insert(held.link_id.0.clone());
                    self.output.removed += 1;
                    self.ops.push(PimdirIndexOp::DropPlacement {
                        collection: row.collection,
                        seq: row.seq,
                    });
                }
                continue;
            }

            let hash = fold.hash(&row, &self.put);
            if let Some(hash) = &hash
                && !fold.indexed.contains(hash)
                && !self.put.contains(hash)
            {
                let extract = fold.extract_of(hash);
                trace!(
                    "index object {} ({})",
                    hash.as_str(),
                    extract.status.as_str()
                );
                self.ops
                    .push(PimdirIndexOp::PutObject(PimdirIndexObject::new(
                        hash, &extract,
                    )));
                self.put.insert(hash.clone());
                self.output.indexed += 1;
            }

            let held_hash = held.as_ref().map(|held| held.hash.clone());
            if row.kind == MAIL {
                let keyed = held.map(|held| held.link_id);
                if held_hash.as_ref() != Some(&hash) || keyed.as_ref() != Some(&row.link_id) {
                    self.touched.extend(keyed.map(|keyed| keyed.0));
                    self.touched.insert(row.link_id.0.clone());
                }
            }

            self.ops
                .push(PimdirIndexOp::PutPlacement(PimdirIndexPlacement {
                    collection: row.collection.clone(),
                    seq: row.seq,
                    link_id: row.link_id.clone(),
                    hash: hash.clone(),
                }));
            match &hash {
                None => {
                    let extract = summary_extract(fold.summaries.get(&key));
                    self.ops.push(PimdirIndexOp::PutSummaryText {
                        collection: row.collection.clone(),
                        seq: row.seq,
                        text: PimdirIndexText::summary(&extract),
                    });
                }
                Some(_) => self.ops.push(PimdirIndexOp::DropSummaryText {
                    collection: row.collection.clone(),
                    seq: row.seq,
                }),
            }

            if row.kind == CALENDAR && held_hash.flatten() != hash {
                let derived = hash.as_ref().and_then(|hash| fold.derived.get(hash));
                let derived = derived.and_then(Option::as_ref);
                let op = self.occurrences(&key, derived);
                self.ops.push(op);
            }

            self.ops.push(PimdirIndexOp::ReplaceFlags {
                collection: row.collection,
                seq: row.seq,
                flags: row.flags.known().cloned().unwrap_or_default(),
            });
        }

        self.after(fold.after)
    }

    /// What follows a folded page: a full one relinks and records its
    /// cursor; the last one first reconciles keys when the store's purge
    /// count or a collection's stamp moved (SEARCH §4) and meets again the
    /// bodies not read; those relink and record.
    fn after(&mut self, after: After) -> Step {
        match after {
            After::Feed if self.full => self.relink_touched(),
            After::Feed if self.meta.purges != self.meta.store_purges || self.meta.moved => {
                debug!("reconcile index keys");
                self.ask(PimdirIndexRefreshYield::WantsGone, State::ReadingGone)
            }
            After::Feed => self.unread(),
            After::Unread => self.relink_touched(),
        }
    }

    /// Reads the live placements the index holds without the body the
    /// store holds (SEARCH §4).
    fn unread(&mut self) -> Step {
        let want = PimdirIndexRefreshYield::WantsUnread { limit: self.batch };
        self.ask(want, State::ReadingUnread)
    }

    /// Reads the messages the run touched, so they are relinked; records
    /// the cursor when none was.
    fn relink_touched(&mut self) -> Step {
        if self.touched.is_empty() {
            return self.finish();
        }
        debug!("relink {} message(s)", self.touched.len());
        let relink = Relink {
            nodes: BTreeMap::new(),
            heads: BTreeMap::new(),
            messages: self.touched.iter().cloned().collect(),
            bodies: Vec::new(),
        };
        let want = PimdirIndexRefreshYield::WantsMessages {
            link_ids: relink.messages.clone(),
        };
        self.ask(want, State::Relinking(Box::new(relink)))
    }

    /// Relinks over the messages read so far, asking what it still needs.
    fn relink(&mut self, mut relink: Box<Relink>) -> Step {
        match thread::relink(&relink.nodes, &relink.heads, &self.touched) {
            Ok(ops) => {
                self.ops.extend(ops);
                self.finish()
            }
            Err(need) if !need.messages.is_empty() => {
                trace!("relink reads {} more message(s)", need.messages.len());
                relink.messages = need.messages.into_iter().collect();
                let want = PimdirIndexRefreshYield::WantsMessages {
                    link_ids: relink.messages.clone(),
                };
                self.ask(want, State::Relinking(relink))
            }
            Err(need) => {
                relink.bodies = need.bodies.into_iter().take(BODIES_MAX).collect();
                let want = PimdirIndexRefreshYield::WantsBodies {
                    hashes: relink.bodies.clone(),
                };
                self.ask(want, State::Relinking(relink))
            }
        }
    }

    /// Records the cursor the run reached, with what the last completed
    /// pass recorded for a full page and what the store held before the
    /// run for the last one (SEARCH §4), and writes what is pending.
    fn finish(&mut self) -> Step {
        let op = match self.full {
            true => {
                self.output.cursor = self.end.unwrap_or(self.meta.store_change);
                PimdirIndexOp::SetCursor {
                    store_change: self.output.cursor,
                    store_purges: self.meta.store_purges,
                    store_reconciled: self.meta.store_reconciled,
                }
            }
            false => {
                self.output.cursor = self
                    .end
                    .unwrap_or(self.meta.store_change)
                    .max(self.meta.head);
                self.output.exhausted = true;
                PimdirIndexOp::SetCursor {
                    store_change: self.output.cursor,
                    store_purges: self.meta.purges,
                    store_reconciled: self.meta.head,
                }
            }
        };
        self.ops.push(op);

        let ops = mem::take(&mut self.ops);
        trace!("write {} index op(s) and the cursor", ops.len());
        self.state = State::Finishing;
        PimdirCoroutineState::Yielded(PimdirIndexRefreshYield::Writes(ops))
    }
}

/// What a run asks of its consumer, each answered by the
/// [`PimdirIndexRefreshArg`] it names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirIndexRefreshYield {
    /// The index's `index_meta` with the store's change cursor and whether
    /// a collection moved, answered by
    /// [`IndexState`](PimdirIndexRefreshArg::IndexState).
    WantsIndexState,
    /// The calendar items a horizon roll or a new time zone database
    /// re-expands (`items_to_reexpand`, SEARCH §7), answered by
    /// [`Reexpand`](PimdirIndexRefreshArg::Reexpand).
    WantsReexpand {
        /// The horizon in force.
        old: PimdirIndexHorizon,
        /// The horizon the run records, the old one when only the database
        /// moved.
        new: PimdirIndexHorizon,
        /// The version of the time zone database in use.
        tzdb_version: String,
    },
    /// The store's feed above a stamp in stamp order (`load_changed_items`,
    /// STORAGE §4.5), answered by [`Feed`](PimdirIndexRefreshArg::Feed).
    WantsFeed {
        /// The stamp the rows lie above.
        since: i64,
        /// The most rows answered.
        limit: usize,
    },
    /// The live placements the index holds without the body the store
    /// holds, files never (SEARCH §4), answered by
    /// [`Feed`](PimdirIndexRefreshArg::Feed).
    WantsUnread {
        /// The most rows answered.
        limit: usize,
    },
    /// What the index holds of these placements and objects, answered by
    /// [`Held`](PimdirIndexRefreshArg::Held).
    WantsHeld {
        /// The placements, by key.
        placements: Vec<PimdirIndexKey>,
        /// The bodies, by hash.
        objects: Vec<PimdirHash>,
    },
    /// The bytes of these bodies, answered by
    /// [`Bodies`](PimdirIndexRefreshArg::Bodies); at most [`BODIES_MAX`].
    WantsBodies {
        /// The bodies, by hash.
        hashes: Vec<PimdirHash>,
    },
    /// The summaries of these live items, answered by
    /// [`Summaries`](PimdirIndexRefreshArg::Summaries): one a body was not
    /// read for, or none of a parser could read.
    WantsSummaries {
        /// The items, by placement key.
        placements: Vec<PimdirIndexKey>,
    },
    /// What the index holds of these message ids (SEARCH §9), answered by
    /// [`Messages`](PimdirIndexRefreshArg::Messages).
    WantsMessages {
        /// The message ids.
        link_ids: Vec<String>,
    },
    /// What the index holds and the store no longer does
    /// (`placements_gone`, `objects_gone`), answered by
    /// [`Gone`](PimdirIndexRefreshArg::Gone).
    ///
    /// A purge or a collection rename is a yield of its own rather than a
    /// row of the feed: the feed pages what the store holds by stamp, and
    /// a purged item, a collected object or a renamed collection's old id
    /// leaves no row to page and draws no stamp (STORAGE §4.5), so what is
    /// gone is a comparison of the index with the store, asked once by the
    /// run exhausting the feed when the purge count or a collection's stamp
    /// moved.
    WantsGone,
    /// Apply these ops in order, inside the run's transaction, answered by
    /// [`Written`](PimdirIndexRefreshArg::Written).
    Writes(Vec<PimdirIndexOp>),
}

/// What a consumer answers a [`PimdirIndexRefreshYield`] with.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirIndexRefreshArg {
    /// Answers [`WantsIndexState`](PimdirIndexRefreshYield::WantsIndexState).
    IndexState(PimdirIndexMeta),
    /// Answers [`WantsReexpand`](PimdirIndexRefreshYield::WantsReexpand):
    /// the items, by placement key.
    Reexpand(Vec<PimdirIndexKey>),
    /// Answers [`WantsFeed`](PimdirIndexRefreshYield::WantsFeed) and
    /// [`WantsUnread`](PimdirIndexRefreshYield::WantsUnread), in stamp
    /// order for the feed.
    Feed(Vec<PimdirIndexFeedRow>),
    /// Answers [`WantsHeld`](PimdirIndexRefreshYield::WantsHeld).
    Held {
        /// The placement rows the index holds among those asked.
        placements: Vec<PimdirIndexPlacement>,
        /// The bodies it holds an `object` row for among those asked.
        objects: BTreeSet<PimdirHash>,
    },
    /// Answers [`WantsBodies`](PimdirIndexRefreshYield::WantsBodies): the
    /// bodies read, one the blob directory no longer holds left out.
    Bodies(BTreeMap<PimdirHash, Vec<u8>>),
    /// Answers [`WantsSummaries`](PimdirIndexRefreshYield::WantsSummaries):
    /// the summaries found, by placement key.
    Summaries(BTreeMap<PimdirIndexKey, PimdirIndexSummary>),
    /// Answers [`WantsMessages`](PimdirIndexRefreshYield::WantsMessages),
    /// by message id; one the index knows nothing of may be left out.
    Messages(BTreeMap<String, PimdirIndexMessage>),
    /// Answers [`WantsGone`](PimdirIndexRefreshYield::WantsGone).
    Gone(PimdirIndexGone),
    /// Answers [`Writes`](PimdirIndexRefreshYield::Writes) once applied.
    Written,
}

/// What one run did and how far it reached.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PimdirIndexRefreshOutput {
    /// The cursor the run recorded: every stamp up to it is folded.
    pub cursor: i64,
    /// Whether the run exhausted the feed, so another one folds nothing
    /// new.
    pub exhausted: bool,
    /// Feed rows folded.
    pub folded: u64,
    /// Bodies extracted and indexed for the first time.
    pub indexed: u64,
    /// Calendar items whose occurrences were expanded (SEARCH §7).
    pub expanded: u64,
    /// Calendar items cut at [`OCCURRENCES_MAX`](occurrence::OCCURRENCES_MAX).
    pub bounded: u64,
    /// Placements dropped: deleted, retained, purged or renamed away.
    pub removed: u64,
    /// Whether the key reconciliation ran (SEARCH §4).
    pub reconciled: bool,
}

/// One write to the index, naming the rows of SEARCH §3's model; a
/// consumer maps each to its shape's statements.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirIndexOp {
    /// Puts a body indexed for the first time: its `object` row, its card
    /// categories, its text and its chunks (`insert_object`,
    /// `insert_object_category`, `insert_object_text`, `insert_chunk`).
    PutObject(PimdirIndexObject),
    /// Drops a body the store no longer holds, its text, chunks and
    /// categories with it (`delete_object`, `delete_object_row`).
    DropObject(PimdirHash),
    /// Puts a placement, or updates its key and body (`upsert_placement`).
    PutPlacement(PimdirIndexPlacement),
    /// Drops a placement, its flags and occurrences with it, and its
    /// summary text (`delete_placement`, then `delete_summary_text` by the
    /// id it returned).
    DropPlacement {
        /// The collection.
        collection: String,
        /// The placement's `seq` there.
        seq: i64,
    },
    /// Replaces a placement's summary text (`delete_summary_text`, then
    /// `insert_summary_text`): a placement holding no body.
    PutSummaryText {
        /// The collection.
        collection: String,
        /// The placement's `seq` there.
        seq: i64,
        /// The text, `title`, `people` and the role fields alone.
        text: PimdirIndexText,
    },
    /// Drops a placement's summary text, if it has any
    /// (`delete_summary_text`): its body is indexed.
    DropSummaryText {
        /// The collection.
        collection: String,
        /// The placement's `seq` there.
        seq: i64,
    },
    /// Replaces a placement's flags (`replace_flags`, then `insert_flag`),
    /// an unknown set as none.
    ReplaceFlags {
        /// The collection.
        collection: String,
        /// The placement's `seq` there.
        seq: i64,
        /// The flags.
        flags: BTreeSet<String>,
    },
    /// Replaces a placement's occurrences (`replace_occurrences`, then
    /// `insert_occurrence`).
    ReplaceOccurrences {
        /// The collection.
        collection: String,
        /// The placement's `seq` there.
        seq: i64,
        /// The occurrences within the horizon, possibly none.
        occurrences: Vec<PimdirIndexOccurrence>,
    },
    /// Puts a message row (`upsert_message`), its thread row written
    /// before it.
    PutMessage {
        /// The message id.
        link_id: String,
        /// Its thread's id.
        thread: String,
        /// The message id it replies to.
        parent: Option<String>,
    },
    /// Drops a message row (`delete_message`).
    DropMessage(String),
    /// Puts a thread row (`upsert_thread`).
    PutThread {
        /// The link id of its present member with the lowest
        /// `(sort_key, link_id)`.
        id: String,
        /// The earliest sort key naming a date among its present members,
        /// empty with none.
        first: String,
        /// The latest, empty with none.
        last: String,
        /// Its present members.
        count: u64,
    },
    /// Drops a thread row when no message row points to it any more,
    /// keeping it otherwise (`delete_thread_unused`).
    DropThread(String),
    /// Records the store's cursor folded in (`set_index_cursor`).
    SetCursor {
        /// The last stamp folded.
        store_change: i64,
        /// The purge count reconciled.
        store_purges: i64,
        /// The cursor of the last completed pass.
        store_reconciled: i64,
    },
    /// Records the horizon occurrences are expanded within
    /// (`set_horizon`).
    SetHorizon(PimdirIndexHorizon),
    /// Records the time zone database version occurrences are expanded
    /// under (`set_tzdb_version`).
    SetTzdbVersion(String),
}

/// What [`WantsIndexState`](PimdirIndexRefreshYield::WantsIndexState)
/// reads: the index's `index_meta`, and the store's change cursor and
/// collection stamps read in the same transaction.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirIndexMeta {
    /// The last stamp the index folded (`index_meta.store_change`).
    pub store_change: i64,
    /// The purge count it reconciled (`index_meta.store_purges`).
    pub store_purges: i64,
    /// The cursor of the last completed pass
    /// (`index_meta.store_reconciled`).
    pub store_reconciled: i64,
    /// The horizon recorded, `None` before one was.
    pub horizon: Option<PimdirIndexHorizon>,
    /// The time zone database version recorded, empty with none.
    pub tzdb_version: String,
    /// The store's last stamp drawn (`load_change_cursor`).
    pub head: i64,
    /// The store's purge count (`load_change_cursor`).
    pub purges: i64,
    /// Whether a collection's stamp lies above `store_reconciled`, which a
    /// rename leaves (SEARCH §4).
    pub moved: bool,
}

/// The window occurrences are expanded within, RFC 3339 instants in UTC
/// (`index_meta.horizon_start`, `horizon_end`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirIndexHorizon {
    /// Its start, included.
    pub start: String,
    /// Its end, excluded.
    pub end: String,
}

impl From<Horizon> for PimdirIndexHorizon {
    fn from(horizon: Horizon) -> Self {
        Self {
            start: time::rfc3339(horizon.start),
            end: time::rfc3339(horizon.end),
        }
    }
}

/// A placement's key in the index and the store: its collection and its
/// `seq` there.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PimdirIndexKey {
    /// The collection.
    pub collection: String,
    /// The placement's `seq` there.
    pub seq: i64,
}

/// One row of the store's feed, in the crate's types: the placement as
/// `load_changed_items` answers it, with its summary where the fold reads
/// it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirIndexFeedRow {
    /// The collection.
    pub collection: String,
    /// The placement's `seq` there.
    pub seq: i64,
    /// The item's key.
    pub link_id: PimdirLinkId,
    /// The body it holds, `None` while bodiless.
    pub object: Option<PimdirHash>,
    /// Its flags.
    pub flags: PimdirFlags,
    /// Whether it is deleted.
    pub deleted: bool,
    /// Whether it is retained (STORAGE §11).
    pub retained: bool,
    /// The collection's kind, a media type.
    pub kind: String,
    /// The collection's account.
    pub account: Option<String>,
    /// The stamp the row took.
    pub changed: i64,
    /// Its summary: carried by a live row holding no body, and by a file
    /// whatever it holds, which is indexed from its summary alone; any
    /// other row may leave it out, the run asking it by
    /// [`WantsSummaries`](PimdirIndexRefreshYield::WantsSummaries) when it
    /// needs it.
    pub summary: Option<PimdirIndexSummary>,
}

impl PimdirIndexFeedRow {
    /// Its key.
    pub fn key(&self) -> PimdirIndexKey {
        PimdirIndexKey {
            collection: self.collection.clone(),
            seq: self.seq,
        }
    }

    /// Whether it is live: neither deleted nor retained.
    pub fn is_live(&self) -> bool {
        !self.deleted && !self.retained
    }

    /// Whether the fold reads its summary whatever the run reads: a live
    /// row holding no body, or a file.
    pub fn needs_summary(&self) -> bool {
        self.is_live() && (self.object.is_none() || self.kind == file::KIND)
    }

    /// The body it is indexed by: its object, a file's never.
    fn indexable(&self) -> Option<&PimdirHash> {
        self.object.as_ref().filter(|_| self.kind != file::KIND)
    }
}

/// An item's summary as the fold reads it: the summary row, its lists
/// (`in_reply_to`, `categories`) filled, and its address rows (STORAGE
/// Annex A.6), a mail summary's own address lists left empty.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirIndexSummary {
    /// The summary row, `None` for an item with none.
    pub summary: Option<PimdirSummary>,
    /// Its address rows, by role, in document order within each.
    pub addresses: Vec<(PimdirAddressRole, PimdirAddress)>,
}

/// A placement row as the index holds it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirIndexPlacement {
    /// The collection.
    pub collection: String,
    /// The placement's `seq` there.
    pub seq: i64,
    /// The item's key.
    pub link_id: PimdirLinkId,
    /// The body indexed for it, `None` while bodiless.
    pub hash: Option<PimdirHash>,
}

impl PimdirIndexPlacement {
    /// Its key.
    pub fn key(&self) -> PimdirIndexKey {
        PimdirIndexKey {
            collection: self.collection.clone(),
            seq: self.seq,
        }
    }
}

/// A body indexed for the first time (SEARCH §6): its `object` row, its
/// folded text, its card categories and the chunks of its unfolded
/// `body` and `note` fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirIndexObject {
    /// The body's hash.
    pub hash: PimdirHash,
    /// What the extraction could read (`object.status`).
    pub status: PimdirExtractStatus,
    /// Whether the extraction wrote any text into the `attachment` field
    /// (`object.attachment`).
    pub attachment: bool,
    /// Whether a card holds a `TEL` (`object.tel`).
    pub tel: bool,
    /// Whether a card holds an `ADR` (`object.adr`).
    pub adr: bool,
    /// Whether a card holds a `BDAY` (`object.bday`).
    pub bday: bool,
    /// Whether a card holds a `PHOTO` (`object.photo`).
    pub photo: bool,
    /// The text by field, folded (`object_text`).
    pub text: PimdirIndexText,
    /// A card's `CATEGORIES` values, folded as STORAGE Annex A.2 folds
    /// `fn`, each once (`object_category`).
    pub categories: Vec<String>,
    /// The chunks, numbered in order from 0 (`chunk`).
    pub chunks: Vec<PimdirChunk>,
}

impl PimdirIndexObject {
    fn new(hash: &PimdirHash, extract: &PimdirExtract) -> Self {
        Self {
            hash: hash.clone(),
            status: extract.status,
            attachment: !extract.attachment.is_empty(),
            tel: extract.tel,
            adr: extract.adr,
            bday: extract.bday,
            photo: extract.photo,
            text: PimdirIndexText::object(extract),
            categories: extract.categories.clone(),
            chunks: extract.chunks(),
        }
    }
}

/// An item's text by field as the index's FTS rows take it, folded under
/// SEARCH §5: the seven fields of SEARCH §6, then one per address role.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirIndexText {
    /// The `title` field.
    pub title: String,
    /// The `people` field.
    pub people: String,
    /// The `body` field, empty in a summary's text.
    pub body: String,
    /// The `attachment` field, empty in a summary's text.
    pub attachment: String,
    /// The `place` field, empty in a summary's text.
    pub place: String,
    /// The `org` field, empty in a summary's text.
    pub org: String,
    /// The `note` field, empty in a summary's text.
    pub note: String,
    /// The `from` role's.
    pub from: String,
    /// The `to` role's.
    pub to: String,
    /// The `cc` role's.
    pub cc: String,
    /// The `bcc` role's.
    pub bcc: String,
    /// The `email` role's.
    pub email: String,
    /// The `organizer` role's.
    pub organizer: String,
    /// The `attendee` role's.
    pub attendee: String,
}

impl PimdirIndexText {
    /// An extraction's every field, folded (`object_text`).
    fn object(extract: &PimdirExtract) -> Self {
        Self {
            body: fold::text(&extract.body),
            attachment: fold::text(&extract.attachment),
            place: fold::text(&extract.place),
            org: fold::text(&extract.org),
            note: fold::text(&extract.note),
            ..Self::summary(extract)
        }
    }

    /// The fields a summary's text holds, folded (`summary_text`).
    fn summary(extract: &PimdirExtract) -> Self {
        Self {
            title: fold::text(&extract.title),
            people: fold::text(&extract.people),
            from: fold::text(&extract.from),
            to: fold::text(&extract.to),
            cc: fold::text(&extract.cc),
            bcc: fold::text(&extract.bcc),
            email: fold::text(&extract.email),
            organizer: fold::text(&extract.organizer),
            attendee: fold::text(&extract.attendee),
            ..Self::default()
        }
    }
}

/// One occurrence of a calendar item, RFC 3339 instants in UTC
/// (`occurrence`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirIndexOccurrence {
    /// When it starts.
    pub start: String,
    /// When it ends, its start for one of zero length.
    pub end: String,
}

/// What the index holds of one message id (SEARCH §9), and where its
/// parent is read from.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirIndexMessage {
    /// Its `message` row, `None` without one.
    pub row: Option<PimdirIndexMessageRow>,
    /// The message ids whose row names it as their parent.
    pub children: Vec<String>,
    /// Its mail placements the index holds, `None` when it holds none: a
    /// message the store does not hold.
    pub presence: Option<PimdirIndexPresence>,
    /// A body the index holds for it, of its mail placement with the
    /// lowest `(collection, seq)` holding one.
    pub hash: Option<PimdirHash>,
    /// The first `In-Reply-To` id of its summary (`mail_in_reply_to`), of
    /// its placement with the lowest `(collection, seq)` naming one.
    pub in_reply_to: Option<String>,
}

/// A `message` row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirIndexMessageRow {
    /// Its thread's id.
    pub thread: String,
    /// The message id it replies to.
    pub parent: Option<String>,
}

/// The mail placements the index holds for one message id, with their
/// sort keys as the store holds them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirIndexPresence {
    /// Their lowest sort key.
    pub sort_key: String,
    /// Their lowest sort key naming a date, `None` without one.
    pub first: Option<String>,
    /// Their highest.
    pub last: Option<String>,
}

/// What the index holds and the store no longer does (SEARCH §4).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirIndexGone {
    /// The placements the store no longer holds (`placements_gone`): purged
    /// rows, a renamed collection's old id; as the index holds them.
    pub placements: Vec<PimdirIndexPlacement>,
    /// The bodies the collector took (`objects_gone`).
    pub objects: Vec<PimdirHash>,
}

/// What a step of the run answers.
type Step =
    PimdirCoroutineState<PimdirIndexRefreshYield, Result<PimdirIndexRefreshOutput, PimdirArgError>>;

/// The kind a message's collection holds.
const MAIL: &str = "message/rfc822";

/// The kind a calendar item's collection holds.
const CALENDAR: &str = "text/calendar";

/// What the coroutine is doing while it waits for the caller.
enum State {
    Start,
    ReadingMeta,
    ListingReexpand {
        new: Horizon,
    },
    ReadingReexpandHeld {
        new: Horizon,
        keys: Vec<PimdirIndexKey>,
    },
    Expanding(Box<Reexpand>),
    ReadingFeed,
    ReadingUnread,
    ReadingHeld(Box<Fold>),
    ReadingBodies(Box<Fold>),
    ReadingSummaries(Box<Fold>),
    ReadingGone,
    Relinking(Box<Relink>),
    Writing(Box<(PimdirIndexRefreshYield, State)>),
    Finishing,
    Done,
}

/// What follows a folded page.
#[derive(Clone, Copy, Debug)]
enum After {
    /// A page of the feed.
    Feed,
    /// The rows whose body was not read.
    Unread,
}

/// A re-expansion under way: the items and the body each holds, and the
/// occurrences derived so far by hash.
struct Reexpand {
    new: Horizon,
    items: Vec<(PimdirIndexKey, Option<PimdirHash>)>,
    queue: Vec<PimdirHash>,
    asked: Vec<PimdirHash>,
    derived: BTreeMap<PimdirHash, Occurrences>,
}

/// A page being folded and what was read for it.
struct Fold {
    rows: Vec<PimdirIndexFeedRow>,
    after: After,
    held: BTreeMap<PimdirIndexKey, PimdirIndexPlacement>,
    indexed: BTreeSet<PimdirHash>,
    /// The bodies to extract, with the kind and key of the first row
    /// holding each.
    extract: BTreeMap<PimdirHash, (String, PimdirIndexKey)>,
    /// The bodies a calendar row's occurrences are expanded from.
    expand: BTreeSet<PimdirHash>,
    queue: Vec<PimdirHash>,
    asked: Vec<PimdirHash>,
    extracted: BTreeMap<PimdirHash, Extracted>,
    /// The occurrences by hash, `None` for a body not read.
    derived: BTreeMap<PimdirHash, Option<Occurrences>>,
    summaries: BTreeMap<PimdirIndexKey, PimdirIndexSummary>,
}

/// A calendar resource's occurrences within the horizon, Unix seconds,
/// and whether it held more than the bound.
type Occurrences = (Vec<(i64, i64)>, bool);

/// A body extracted.
enum Extracted {
    /// Read by its parser.
    Read(Box<PimdirExtract>),
    /// Read from the summary of the row at `key`: no parser accepted it,
    /// `parsed` when one for its kind tried.
    Summary { parsed: bool, key: PimdirIndexKey },
}

impl Fold {
    /// The bodies the page reads: each one the index holds no object for,
    /// to extract, and each a calendar row holds that is not the one its
    /// placement was expanded from.
    fn plan(&mut self, put: &BTreeSet<PimdirHash>) {
        for row in self.rows.iter().filter(|row| row.is_live()) {
            let Some(hash) = row.indexable() else {
                continue;
            };
            let key = row.key();
            if !self.indexed.contains(hash) && !put.contains(hash) {
                self.extract
                    .entry(hash.clone())
                    .or_insert_with(|| (row.kind.clone(), key.clone()));
            }
            let held = self.held.get(&key).and_then(|held| held.hash.as_ref());
            if row.kind == CALENDAR && held != Some(hash) {
                self.expand.insert(hash.clone());
            }
        }

        let queue: BTreeSet<&PimdirHash> = self.extract.keys().chain(&self.expand).collect();
        self.queue = queue.into_iter().cloned().collect();
    }

    /// Extracts and expands the bodies asked, a missing one recorded as
    /// such.
    fn read(&mut self, bodies: &BTreeMap<PimdirHash, Vec<u8>>, horizon: &Horizon) {
        for hash in mem::take(&mut self.asked) {
            let body = bodies.get(&hash);
            if let Some((kind, key)) = self.extract.get(&hash) {
                match body {
                    Some(body) => {
                        let parsed = PimdirExtract::body(kind, body);
                        let extracted = match parsed {
                            Some(extract) if extract.status != PimdirExtractStatus::Unparseable => {
                                Extracted::Read(Box::new(extract))
                            }
                            parsed => Extracted::Summary {
                                parsed: parsed.is_some(),
                                key: key.clone(),
                            },
                        };
                        self.extracted.insert(hash.clone(), extracted);
                    }
                    None => debug!("blob {} is missing, indexing it later", hash.as_str()),
                }
            }
            if self.expand.contains(&hash) {
                let derived = body.map(|body| horizon.occurrences(body));
                self.derived.insert(hash, derived);
            }
        }
    }

    /// The body a live row is indexed by, once its page is read: its own
    /// when the index holds it or it was read, `None` while bodiless, for
    /// a file, or for a body the blob directory no longer holds.
    fn hash(&self, row: &PimdirIndexFeedRow, put: &BTreeSet<PimdirHash>) -> Option<PimdirHash> {
        let hash = row.indexable()?;
        let held = self.indexed.contains(hash) || put.contains(hash);
        (held || self.extracted.contains_key(hash)).then(|| hash.clone())
    }

    /// The summaries the page reads that its rows did not carry: a row
    /// indexed by no body, and the row an unparseable body is read from.
    fn lacking(&self, put: &BTreeSet<PimdirHash>) -> Vec<PimdirIndexKey> {
        let carried: BTreeSet<PimdirIndexKey> = self
            .rows
            .iter()
            .filter(|row| row.summary.is_some())
            .map(PimdirIndexFeedRow::key)
            .collect();

        let mut lacking = BTreeSet::new();
        for row in self.rows.iter().filter(|row| row.is_live()) {
            if self.hash(row, put).is_none() {
                lacking.insert(row.key());
            }
        }
        for extracted in self.extracted.values() {
            if let Extracted::Summary { key, .. } = extracted {
                lacking.insert(key.clone());
            }
        }
        lacking.retain(|key| !carried.contains(key));
        lacking.into_iter().collect()
    }

    /// The extraction of a body read for the first time, from its parser
    /// or from the summary of the row it was first met in.
    fn extract_of(&mut self, hash: &PimdirHash) -> PimdirExtract {
        match self.extracted.remove(hash) {
            Some(Extracted::Read(extract)) => *extract,
            Some(Extracted::Summary { parsed, key }) => {
                let mut extract = summary_extract(self.summaries.get(&key));
                if parsed {
                    extract.status = PimdirExtractStatus::Unparseable;
                }
                extract
            }
            None => PimdirExtract::default(),
        }
    }
}

/// A relink under way: the messages and the body heads read so far, and
/// what was asked last.
struct Relink {
    nodes: BTreeMap<String, PimdirIndexMessage>,
    heads: Heads,
    messages: Vec<String>,
    bodies: Vec<PimdirHash>,
}

/// An item's text read from its summary and address rows (SEARCH §6).
fn summary_extract(summary: Option<&PimdirIndexSummary>) -> PimdirExtract {
    match summary {
        Some(found) => PimdirExtract::summary(found.summary.as_ref(), &found.addresses),
        None => PimdirExtract::summary(None, &[]),
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use crate::{
        coroutine::{PimdirArgError, PimdirCoroutineState},
        search::refresh::*,
    };

    /// A first run over an empty store: the horizon recorded before the
    /// feed is read, the cursor at the store's head once it is exhausted.
    #[test]
    fn an_empty_feed_records_the_horizon_then_the_cursor() {
        crate::testlog::init();
        let mut run = PimdirIndexRefresh::new(1_790_000_000, 8);
        let mut answers = vec![
            PimdirIndexRefreshArg::IndexState(PimdirIndexMeta {
                head: 3,
                ..Default::default()
            }),
            PimdirIndexRefreshArg::Written,
            PimdirIndexRefreshArg::Feed(Vec::new()),
            PimdirIndexRefreshArg::Feed(Vec::new()),
        ]
        .into_iter();

        let mut wants = Vec::new();
        let mut arg = None;
        let ops = loop {
            match run.resume(arg.take()) {
                PimdirCoroutineState::Yielded(PimdirIndexRefreshYield::Writes(ops))
                    if ops
                        .iter()
                        .any(|op| matches!(op, PimdirIndexOp::SetCursor { .. })) =>
                {
                    break ops;
                }
                PimdirCoroutineState::Yielded(want) => wants.push(want),
                state => panic!("expected a yield, got {state:?}"),
            }
            arg = answers.next();
        };

        assert!(matches!(wants[0], PimdirIndexRefreshYield::WantsIndexState));
        let PimdirIndexRefreshYield::Writes(recorded) = &wants[1] else {
            panic!("the horizon is written first: {:?}", wants[1]);
        };
        assert!(matches!(recorded[0], PimdirIndexOp::SetHorizon(_)));
        assert_eq!(
            wants[2],
            PimdirIndexRefreshYield::WantsFeed { since: 0, limit: 8 }
        );
        assert_eq!(wants[3], PimdirIndexRefreshYield::WantsUnread { limit: 8 });
        assert_eq!(
            ops,
            [PimdirIndexOp::SetCursor {
                store_change: 3,
                store_purges: 0,
                store_reconciled: 3,
            }]
        );

        match run.resume(Some(PimdirIndexRefreshArg::Written)) {
            PimdirCoroutineState::Complete(Ok(output)) => {
                assert!(output.exhausted);
                assert_eq!(output.cursor, 3);
            }
            state => panic!("expected Complete(Ok), got {state:?}"),
        }
    }

    /// A caller resuming a finished run, or answering the wrong yield, is
    /// told, never handed a success.
    #[test]
    fn a_broken_contract_is_refused() {
        let mut run = PimdirIndexRefresh::new(0, 1);
        match run.resume(Some(PimdirIndexRefreshArg::Written)) {
            PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => (),
            state => panic!("expected UnexpectedArg, got {state:?}"),
        }
        match run.resume(None) {
            PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => (),
            state => panic!("expected UnexpectedArg, got {state:?}"),
        }

        let mut run = PimdirIndexRefresh::new(0, 1);
        let _ = run.resume(None);
        match run.resume(None) {
            PimdirCoroutineState::Complete(Err(PimdirArgError::MissingArg)) => (),
            state => panic!("expected MissingArg, got {state:?}"),
        }
    }
}
