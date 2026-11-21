//! # Refresh over SQLite
//!
//! The canonical consumer of [`PimdirIndexRefresh`]: every yield answered
//! and every op written through the canonical statements of SEARCH.md and
//! STORAGE.md (with the crate's own beside them), on index.db with the
//! store attached as `store`, inside the run's one transaction.
//!
//! [`PimdirIndexRefresh`]: crate::search::refresh::PimdirIndexRefresh

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
    vec::Vec,
};

use log::{debug, trace};
use rusqlite::{OptionalExtension, Transaction, named_params};

use crate::{
    client::{PimdirError, PreparedCached, blobs::PimdirBlobs, rows, search::summary},
    codec,
    object::PimdirHash,
    placement::PimdirLinkId,
    search::refresh::{
        PimdirIndexFeedRow, PimdirIndexGone, PimdirIndexGonePlacement, PimdirIndexHorizon,
        PimdirIndexKey, PimdirIndexMessage, PimdirIndexMessageRow, PimdirIndexMeta,
        PimdirIndexObject, PimdirIndexOp, PimdirIndexPlacement, PimdirIndexPresence,
        PimdirIndexRefreshArg, PimdirIndexRefreshYield, PimdirIndexSummary, PimdirIndexText,
        PimdirIndexThreadRow,
    },
    sql::{self, search},
};

/// The canonical statements over one refresh transaction.
pub(crate) struct Statements<'a> {
    pub(crate) tx: &'a Transaction<'a>,
    pub(crate) blobs: &'a PimdirBlobs,
}

impl Statements<'_> {
    /// Answers one yield, writing a [`Writes`] yield's ops in order, their
    /// full-text rows last ([`TextWrite`]). A body that cannot be read is
    /// answered as a gone one (SEARCH §4).
    ///
    /// [`Writes`]: PimdirIndexRefreshYield::Writes
    pub(crate) fn answer(
        &self,
        want: PimdirIndexRefreshYield,
    ) -> Result<PimdirIndexRefreshArg, PimdirError> {
        let tx = self.tx;
        let arg = match want {
            PimdirIndexRefreshYield::WantsIndexState => {
                PimdirIndexRefreshArg::IndexState(self.meta()?)
            }
            PimdirIndexRefreshYield::WantsReexpand {
                old,
                new,
                tzdb_version,
                floating_zone,
            } => PimdirIndexRefreshArg::Reexpand(rows(
                tx,
                search::ITEMS_TO_REEXPAND,
                named_params! {
                    ":old_start": old.start,
                    ":old_end": old.end,
                    ":new_start": new.start,
                    ":new_end": new.end,
                    ":tzdb_version": tzdb_version,
                    ":floating_zone": floating_zone,
                },
                |r| {
                    Ok(PimdirIndexKey {
                        collection: r.get(0)?,
                        seq: r.get(1)?,
                    })
                },
            )?),
            PimdirIndexRefreshYield::WantsFeed { since, limit } => {
                PimdirIndexRefreshArg::Feed(self.feed(
                    search::LOAD_CHANGED_ITEMS,
                    named_params! { ":since": since, ":limit": limit as i64 },
                )?)
            }
            PimdirIndexRefreshYield::WantsMissing => {
                PimdirIndexRefreshArg::Feed(self.feed(search::LOAD_MISSING_ITEMS, &[])?)
            }
            PimdirIndexRefreshYield::WantsHeld {
                placements,
                objects,
            } => {
                let mut held = Vec::new();
                for key in placements {
                    held.extend(self.placement(&key.collection, key.seq)?);
                }
                let mut indexed = BTreeSet::new();
                for hash in objects {
                    if self.object(&hash)?.is_some() {
                        indexed.insert(hash);
                    }
                }
                PimdirIndexRefreshArg::Held {
                    placements: held,
                    objects: indexed,
                }
            }
            PimdirIndexRefreshYield::WantsBodies { hashes } => {
                let mut bodies = BTreeMap::new();
                for hash in hashes {
                    match self.blobs.get(&hash) {
                        Ok(Some(body)) => {
                            bodies.insert(hash, body);
                        }
                        Ok(None) => {}
                        Err(err) => debug!("cannot read body {}: {err}", hash.0),
                    }
                }
                PimdirIndexRefreshArg::Bodies(bodies)
            }
            PimdirIndexRefreshYield::WantsSummaries { placements } => {
                let mut kinds: BTreeMap<String, String> = BTreeMap::new();
                let mut found = BTreeMap::new();
                for key in placements {
                    if !kinds.contains_key(&key.collection) {
                        let kind: Option<String> = tx
                            .query_row_cached(
                                sql::LOAD_KIND,
                                named_params! { ":collection": key.collection },
                                |r| r.get(0),
                            )
                            .optional()?;
                        kinds.insert(key.collection.clone(), kind.unwrap_or_default());
                    }
                    let kind = &kinds[&key.collection];
                    let (summary, addresses) =
                        summary::summary(tx, &key.collection, key.seq, kind)?;
                    let read = PimdirIndexSummary { summary, addresses };
                    found.insert(key, read);
                }
                PimdirIndexRefreshArg::Summaries(found)
            }
            PimdirIndexRefreshYield::WantsMessages { link_ids } => {
                let mut read = BTreeMap::new();
                for link_id in link_ids {
                    let message = self.message(&link_id)?;
                    read.insert(link_id, message);
                }
                PimdirIndexRefreshArg::Messages(read)
            }
            PimdirIndexRefreshYield::WantsThreadRows { threads } => {
                let mut read = Vec::new();
                for thread in threads {
                    read.extend(rows(
                        tx,
                        search::THREAD_ROWS,
                        named_params! { ":thread": thread },
                        |r| {
                            Ok(PimdirIndexThreadRow {
                                link_id: r.get(0)?,
                                parent: r.get(1)?,
                                sort_key: r.get(2)?,
                            })
                        },
                    )?);
                }
                PimdirIndexRefreshArg::ThreadRows(read)
            }
            PimdirIndexRefreshYield::WantsGone => {
                let placements = rows(tx, search::PLACEMENTS_GONE, [], |r| {
                    Ok(PimdirIndexGonePlacement {
                        collection: r.get(0)?,
                        seq: r.get(1)?,
                        link_id: PimdirLinkId(r.get(2)?),
                        hash: r.get::<_, Option<String>>(3)?.map(PimdirHash),
                    })
                })?;
                let objects = rows(tx, search::OBJECTS_GONE, [], |r| Ok(PimdirHash(r.get(0)?)))?;
                PimdirIndexRefreshArg::Gone(PimdirIndexGone {
                    placements,
                    objects,
                })
            }
            PimdirIndexRefreshYield::Writes(ops) => {
                let mut texts = Vec::new();
                for op in ops {
                    self.write(op, &mut texts)?;
                }
                self.write_texts(texts)?;
                PimdirIndexRefreshArg::Written
            }
        };
        Ok(arg)
    }

    /// The index's `index_meta` with the store's change cursor.
    fn meta(&self) -> Result<PimdirIndexMeta, PimdirError> {
        let tx = self.tx;
        let mut meta = tx.query_row_cached(search::LOAD_INDEX_META, [], |r| {
            let start: String = r.get(4)?;
            let end: String = r.get(5)?;
            Ok(PimdirIndexMeta {
                store_change: r.get(2)?,
                store_purges: r.get(3)?,
                horizon: (!start.is_empty() && !end.is_empty())
                    .then_some(PimdirIndexHorizon { start, end }),
                tzdb_version: r.get(6)?,
                floating_zone: r.get(11)?,
                ..Default::default()
            })
        })?;
        (meta.head, meta.purges) =
            tx.query_row_cached(sql::LOAD_CHANGE_CURSOR, [], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(meta)
    }

    /// Feed rows through a statement answering `load_changed_items`'
    /// columns, each row the fold reads the summary of carrying it.
    fn feed(
        &self,
        statement: &str,
        params: &[(&str, &dyn rusqlite::ToSql)],
    ) -> Result<Vec<PimdirIndexFeedRow>, PimdirError> {
        let tx = self.tx;
        let mut feed = rows(tx, statement, params, |r| {
            Ok(PimdirIndexFeedRow {
                collection: r.get(0)?,
                link_id: PimdirLinkId(r.get(1)?),
                seq: r.get(2)?,
                object: r.get::<_, Option<String>>(3)?.map(PimdirHash),
                flags: codec::flags_from_column(r.get::<_, Option<String>>(4)?.as_deref()),
                deleted: r.get::<_, i64>(5)? != 0,
                retained: r.get::<_, Option<String>>(6)?.is_some(),
                kind: r.get(7)?,
                account: r.get(8)?,
                changed: r.get(9)?,
                summary: None,
            })
        })?;
        for row in &mut feed {
            if row.needs_summary() {
                let (summary, addresses) =
                    summary::summary(tx, &row.collection, row.seq, &row.kind)?;
                row.summary = Some(PimdirIndexSummary { summary, addresses });
            }
        }
        Ok(feed)
    }

    /// One placement row as the index holds it.
    fn placement(
        &self,
        collection: &str,
        seq: i64,
    ) -> Result<Option<PimdirIndexPlacement>, PimdirError> {
        Ok(self
            .tx
            .query_row_cached(
                search::LOAD_PLACEMENT,
                named_params! { ":collection": collection, ":seq": seq },
                |r| {
                    Ok(PimdirIndexPlacement {
                        collection: collection.into(),
                        seq,
                        link_id: PimdirLinkId(r.get(1)?),
                        hash: r.get::<_, Option<String>>(2)?.map(PimdirHash),
                    })
                },
            )
            .optional()?)
    }

    /// The rowid of a placement's row, which its summary text is keyed on.
    fn placement_id(&self, collection: &str, seq: i64) -> Result<Option<i64>, PimdirError> {
        Ok(self
            .tx
            .query_row_cached(
                search::LOAD_PLACEMENT,
                named_params! { ":collection": collection, ":seq": seq },
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The rowid of a body's `object` row, which its text is keyed on.
    fn object(&self, hash: &PimdirHash) -> Result<Option<i64>, PimdirError> {
        Ok(self
            .tx
            .query_row_cached(
                search::OBJECT_INDEXED,
                named_params! { ":hash": hash.0 },
                |r| r.get(0),
            )
            .optional()?)
    }

    /// What the index holds of one message id (SEARCH §9).
    fn message(&self, link_id: &str) -> Result<PimdirIndexMessage, PimdirError> {
        let tx = self.tx;
        let key = named_params! { ":link_id": link_id };
        let row = tx
            .query_row_cached(search::LOAD_MESSAGE, key, |r| {
                Ok(PimdirIndexMessageRow {
                    thread: r.get(0)?,
                    parent: r.get(1)?,
                })
            })
            .optional()?;
        let children = rows(tx, search::MESSAGE_CHILDREN, key, |r| r.get(0))?;
        let (count, sort_key, first, last): (i64, Option<String>, Option<String>, Option<String>) =
            tx.query_row_cached(search::MESSAGE_PRESENCE, key, |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?;
        let hash = tx
            .query_row_cached(search::MESSAGE_HASH, key, |r| r.get::<_, String>(0))
            .optional()?
            .map(PimdirHash);
        let in_reply_to = tx
            .query_row_cached(search::MESSAGE_IN_REPLY_TO, key, |r| r.get(0))
            .optional()?;

        Ok(PimdirIndexMessage {
            row,
            children,
            presence: (count > 0).then(|| PimdirIndexPresence {
                sort_key: sort_key.unwrap_or_default(),
                first,
                last,
            }),
            hash,
            in_reply_to,
        })
    }

    /// Writes one op through its statements, its full-text writes held
    /// back in `texts`, in order, for [`write_texts`](Self::write_texts).
    fn write(&self, op: PimdirIndexOp, texts: &mut Vec<TextWrite>) -> Result<(), PimdirError> {
        let tx = self.tx;
        match op {
            PimdirIndexOp::PutObject(object) => self.put_object(object, texts)?,
            PimdirIndexOp::DropObject(hash) => {
                if let Some(id) = self.object(&hash)? {
                    texts.push(TextWrite::DropObject(id));
                }
                tx.execute_cached(search::DELETE_OBJECT_ROW, named_params! { ":hash": hash.0 })?;
            }
            PimdirIndexOp::DropUnplaced(hash) => {
                let placed: bool = tx.query_row_cached(
                    search::OBJECT_PLACED,
                    named_params! { ":hash": hash.0 },
                    |r| r.get(0),
                )?;
                if !placed {
                    return self.write(PimdirIndexOp::DropObject(hash), texts);
                }
            }
            PimdirIndexOp::PutPlacement(placement) => {
                tx.query_row_cached(
                    search::UPSERT_PLACEMENT,
                    named_params! {
                        ":collection": placement.collection,
                        ":seq": placement.seq,
                        ":link_id": placement.link_id.0,
                        ":hash": placement.hash.map(|hash| hash.0),
                    },
                    |r| r.get::<_, i64>(0),
                )?;
            }
            PimdirIndexOp::DropPlacement { collection, seq } => {
                let id: Option<i64> = tx
                    .query_row_cached(
                        search::DELETE_PLACEMENT,
                        named_params! { ":collection": collection, ":seq": seq },
                        |r| r.get(0),
                    )
                    .optional()?;
                texts.extend(id.map(TextWrite::DropSummary));
            }
            PimdirIndexOp::PutSummaryText {
                collection,
                seq,
                text,
            } => {
                if let Some(id) = self.placement_id(&collection, seq)? {
                    texts.push(TextWrite::DropSummary(id));
                    texts.push(TextWrite::PutSummary(id, text));
                }
            }
            PimdirIndexOp::DropSummaryText { collection, seq } => {
                if let Some(id) = self.placement_id(&collection, seq)? {
                    texts.push(TextWrite::DropSummary(id));
                }
            }
            PimdirIndexOp::ReplaceFlags {
                collection,
                seq,
                flags,
            } => {
                let key = named_params! { ":collection": collection, ":seq": seq };
                tx.execute_cached(search::REPLACE_FLAGS, key)?;
                for flag in flags {
                    tx.execute_cached(
                        search::INSERT_FLAG,
                        named_params! { ":flag": flag, ":collection": collection, ":seq": seq },
                    )?;
                }
            }
            PimdirIndexOp::ReplaceOccurrences {
                collection,
                seq,
                occurrences,
                cut,
            } => {
                let key = named_params! { ":collection": collection, ":seq": seq };
                tx.execute_cached(search::REPLACE_OCCURRENCES, key)?;
                tx.execute_cached(
                    search::SET_CUT,
                    named_params! { ":cut": cut, ":collection": collection, ":seq": seq },
                )?;
                for occurrence in occurrences {
                    tx.execute_cached(
                        search::INSERT_OCCURRENCE,
                        named_params! {
                            ":collection": collection,
                            ":seq": seq,
                            ":start": occurrence.start,
                            ":end": occurrence.end,
                        },
                    )?;
                }
            }
            PimdirIndexOp::PutMessage {
                link_id,
                thread,
                parent,
            } => {
                tx.execute_cached(
                    search::UPSERT_MESSAGE,
                    named_params! { ":link_id": link_id, ":thread": thread, ":parent": parent },
                )?;
            }
            PimdirIndexOp::PutMissing(key) => {
                tx.execute_cached(
                    search::INSERT_MISSING,
                    named_params! { ":collection": key.collection, ":seq": key.seq },
                )?;
            }
            PimdirIndexOp::DropMissing(key) => {
                tx.execute_cached(
                    search::DELETE_MISSING,
                    named_params! { ":collection": key.collection, ":seq": key.seq },
                )?;
            }
            PimdirIndexOp::PruneThread(thread) => {
                let key = named_params! { ":thread": thread };
                tx.execute_cached(search::UNPARENT_MESSAGES, key)?;
                let pruned: Vec<String> = rows(tx, search::PRUNE_MESSAGES, key, |r| r.get(0))?;
                trace!("pruned {} message row(s) of thread {thread}", pruned.len());
            }
            PimdirIndexOp::PutThread {
                id,
                first,
                last,
                count,
            } => {
                tx.execute_cached(
                    search::UPSERT_THREAD,
                    named_params! {
                        ":id": id,
                        ":first": first,
                        ":last": last,
                        ":count": count as i64,
                    },
                )?;
            }
            PimdirIndexOp::DropThread(id) => {
                tx.execute_cached(search::DELETE_THREAD, named_params! { ":id": id })?;
            }
            PimdirIndexOp::SetCursor {
                store_change,
                store_purges,
            } => {
                tx.execute_cached(
                    search::SET_INDEX_CURSOR,
                    named_params! {
                        ":store_change": store_change,
                        ":store_purges": store_purges,
                    },
                )?;
            }
            PimdirIndexOp::SetHorizon(horizon) => {
                tx.execute_cached(
                    search::SET_HORIZON,
                    named_params! {
                        ":horizon_start": horizon.start,
                        ":horizon_end": horizon.end,
                    },
                )?;
            }
            PimdirIndexOp::SetTzdbVersion(version) => {
                tx.execute_cached(
                    search::SET_TZDB_VERSION,
                    named_params! { ":tzdb_version": version },
                )?;
            }
            PimdirIndexOp::SetFloatingZone(zone) => {
                tx.execute_cached(
                    search::SET_FLOATING_ZONE,
                    named_params! { ":floating_zone": zone },
                )?;
            }
        }
        Ok(())
    }

    /// Writes a body indexed for the first time: its `object` row, its
    /// categories and its chunks, its text keyed on the row's id held back
    /// in `texts`.
    fn put_object(
        &self,
        object: PimdirIndexObject,
        texts: &mut Vec<TextWrite>,
    ) -> Result<(), PimdirError> {
        let tx = self.tx;
        let id: i64 = tx.query_row_cached(
            search::INSERT_OBJECT,
            named_params! {
                ":hash": object.hash.0,
                ":status": object.status.as_str(),
                ":attachment": i64::from(object.attachment),
                ":tel": i64::from(object.tel),
                ":adr": i64::from(object.adr),
                ":bday": i64::from(object.bday),
                ":photo": i64::from(object.photo),
            },
            |r| r.get(0),
        )?;
        for value in &object.categories {
            tx.execute_cached(
                search::INSERT_OBJECT_CATEGORY,
                named_params! { ":object": id, ":value": value },
            )?;
        }
        for (ord, chunk) in object.chunks.iter().enumerate() {
            tx.execute_cached(
                search::INSERT_CHUNK,
                named_params! {
                    ":object": id,
                    ":ord": ord as i64,
                    ":field": chunk.field,
                    ":start": chunk.range.start as i64,
                    ":end": chunk.range.end as i64,
                },
            )?;
        }
        texts.push(TextWrite::PutObject(id, object.text));
        Ok(())
    }

    /// Writes the full-text rows a yield's ops held back, in their order.
    fn write_texts(&self, texts: Vec<TextWrite>) -> Result<(), PimdirError> {
        let tx = self.tx;
        for write in texts {
            match write {
                TextWrite::PutObject(id, text) => {
                    tx.execute_cached(
                        search::INSERT_OBJECT_TEXT,
                        named_params! {
                            ":rowid": id,
                            ":title": text.title,
                            ":people": text.people,
                            ":body": text.body,
                            ":attachment": text.attachment,
                            ":place": text.place,
                            ":org": text.org,
                            ":note": text.note,
                            ":from": text.from,
                            ":to": text.to,
                            ":cc": text.cc,
                            ":bcc": text.bcc,
                            ":email": text.email,
                            ":organizer": text.organizer,
                            ":attendee": text.attendee,
                        },
                    )?;
                }
                TextWrite::DropObject(id) => {
                    tx.execute_cached(search::DELETE_OBJECT, named_params! { ":rowid": id })?;
                }
                TextWrite::PutSummary(id, text) => {
                    tx.execute_cached(
                        search::INSERT_SUMMARY_TEXT,
                        named_params! {
                            ":rowid": id,
                            ":title": text.title,
                            ":people": text.people,
                            ":from": text.from,
                            ":to": text.to,
                            ":cc": text.cc,
                            ":bcc": text.bcc,
                            ":email": text.email,
                            ":organizer": text.organizer,
                            ":attendee": text.attendee,
                        },
                    )?;
                }
                TextWrite::DropSummary(id) => {
                    tx.execute_cached(search::DELETE_SUMMARY_TEXT, named_params! { ":rowid": id })?;
                }
            }
        }
        Ok(())
    }
}

/// One write to a full-text table, held back until the other ops of its
/// yield are written.
///
/// A statement that may abort after changing a row, which the foreign
/// keys make of most writes here, opens a statement savepoint, and FTS5
/// flushes its pending terms to a new segment at each one: interleaved,
/// every body indexed would be a segment of its own, merged again and
/// again. No other op reads the full-text tables, so writing them last,
/// in their order, ends at the same rows.
enum TextWrite {
    /// A body's text, keyed on its `object` row (`insert_object_text`).
    PutObject(i64, PimdirIndexText),
    /// A body's text dropped (`delete_object`).
    DropObject(i64),
    /// A placement's summary text, keyed on its row (`insert_summary_text`).
    PutSummary(i64, PimdirIndexText),
    /// A placement's summary text dropped (`delete_summary_text`).
    DropSummary(i64),
}
