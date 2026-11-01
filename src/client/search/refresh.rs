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

use log::trace;
use rusqlite::{Connection, OptionalExtension, Transaction, named_params};

use crate::{
    client::{PimdirError, bind_list, blobs::PimdirBlobs, rows, write::tables_of},
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
    summary::{PimdirAddress, PimdirAddressRole, PimdirSummary},
};

/// The canonical statements over one refresh transaction.
pub(crate) struct Statements<'a> {
    pub(crate) tx: &'a Transaction<'a>,
    pub(crate) blobs: &'a PimdirBlobs,
}

impl Statements<'_> {
    /// Answers one yield, writing a [`Writes`] yield's ops in order.
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
                    if let Some(body) = self.blobs.get(&hash)? {
                        bodies.insert(hash, body);
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
                            .query_row(
                                sql::LOAD_KIND,
                                named_params! { ":collection": key.collection },
                                |r| r.get(0),
                            )
                            .optional()?;
                        kinds.insert(key.collection.clone(), kind.unwrap_or_default());
                    }
                    let kind = &kinds[&key.collection];
                    let read = summary(tx, &key.collection, key.seq, kind)?;
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
                    })
                })?;
                let objects = rows(tx, search::OBJECTS_GONE, [], |r| Ok(PimdirHash(r.get(0)?)))?;
                PimdirIndexRefreshArg::Gone(PimdirIndexGone {
                    placements,
                    objects,
                })
            }
            PimdirIndexRefreshYield::Writes(ops) => {
                for op in ops {
                    self.write(op)?;
                }
                PimdirIndexRefreshArg::Written
            }
        };
        Ok(arg)
    }

    /// The index's `index_meta` with the store's change cursor.
    fn meta(&self) -> Result<PimdirIndexMeta, PimdirError> {
        let tx = self.tx;
        let mut meta = tx.query_row(search::LOAD_INDEX_META, [], |r| {
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
            tx.query_row(sql::LOAD_CHANGE_CURSOR, [], |r| Ok((r.get(0)?, r.get(1)?)))?;
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
                row.summary = Some(summary(tx, &row.collection, row.seq, &row.kind)?);
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
            .query_row(
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
            .query_row(
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
            .query_row(
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
            .query_row(search::LOAD_MESSAGE, key, |r| {
                Ok(PimdirIndexMessageRow {
                    thread: r.get(0)?,
                    parent: r.get(1)?,
                })
            })
            .optional()?;
        let children = rows(tx, search::MESSAGE_CHILDREN, key, |r| r.get(0))?;
        let (count, sort_key, first, last): (i64, Option<String>, Option<String>, Option<String>) =
            tx.query_row(search::MESSAGE_PRESENCE, key, |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?;
        let hash = tx
            .query_row(search::MESSAGE_HASH, key, |r| r.get::<_, String>(0))
            .optional()?
            .map(PimdirHash);
        let in_reply_to = tx
            .query_row(search::MESSAGE_IN_REPLY_TO, key, |r| r.get(0))
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

    /// Writes one op through its statements.
    fn write(&self, op: PimdirIndexOp) -> Result<(), PimdirError> {
        let tx = self.tx;
        match op {
            PimdirIndexOp::PutObject(object) => self.put_object(object)?,
            PimdirIndexOp::DropObject(hash) => {
                if let Some(id) = self.object(&hash)? {
                    tx.execute(search::DELETE_OBJECT, named_params! { ":rowid": id })?;
                }
                tx.execute(search::DELETE_OBJECT_ROW, named_params! { ":hash": hash.0 })?;
            }
            PimdirIndexOp::PutPlacement(placement) => {
                tx.query_row(
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
                    .query_row(
                        search::DELETE_PLACEMENT,
                        named_params! { ":collection": collection, ":seq": seq },
                        |r| r.get(0),
                    )
                    .optional()?;
                if let Some(id) = id {
                    tx.execute(search::DELETE_SUMMARY_TEXT, named_params! { ":rowid": id })?;
                }
            }
            PimdirIndexOp::PutSummaryText {
                collection,
                seq,
                text,
            } => {
                if let Some(id) = self.placement_id(&collection, seq)? {
                    tx.execute(search::DELETE_SUMMARY_TEXT, named_params! { ":rowid": id })?;
                    self.put_summary_text(id, &text)?;
                }
            }
            PimdirIndexOp::DropSummaryText { collection, seq } => {
                if let Some(id) = self.placement_id(&collection, seq)? {
                    tx.execute(search::DELETE_SUMMARY_TEXT, named_params! { ":rowid": id })?;
                }
            }
            PimdirIndexOp::ReplaceFlags {
                collection,
                seq,
                flags,
            } => {
                let key = named_params! { ":collection": collection, ":seq": seq };
                tx.execute(search::REPLACE_FLAGS, key)?;
                for flag in flags {
                    tx.execute(
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
                tx.execute(search::REPLACE_OCCURRENCES, key)?;
                tx.execute(
                    search::SET_CUT,
                    named_params! { ":cut": cut, ":collection": collection, ":seq": seq },
                )?;
                for occurrence in occurrences {
                    tx.execute(
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
                tx.execute(
                    search::UPSERT_MESSAGE,
                    named_params! { ":link_id": link_id, ":thread": thread, ":parent": parent },
                )?;
            }
            PimdirIndexOp::PutMissing(key) => {
                tx.execute(
                    search::INSERT_MISSING,
                    named_params! { ":collection": key.collection, ":seq": key.seq },
                )?;
            }
            PimdirIndexOp::DropMissing(key) => {
                tx.execute(
                    search::DELETE_MISSING,
                    named_params! { ":collection": key.collection, ":seq": key.seq },
                )?;
            }
            PimdirIndexOp::PruneThread(thread) => {
                let key = named_params! { ":thread": thread };
                tx.execute(search::UNPARENT_MESSAGES, key)?;
                let pruned: Vec<String> = rows(tx, search::PRUNE_MESSAGES, key, |r| r.get(0))?;
                trace!("pruned {} message row(s) of thread {thread}", pruned.len());
            }
            PimdirIndexOp::PutThread {
                id,
                first,
                last,
                count,
            } => {
                tx.execute(
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
                tx.execute(search::DELETE_THREAD, named_params! { ":id": id })?;
            }
            PimdirIndexOp::SetCursor {
                store_change,
                store_purges,
            } => {
                tx.execute(
                    search::SET_INDEX_CURSOR,
                    named_params! {
                        ":store_change": store_change,
                        ":store_purges": store_purges,
                    },
                )?;
            }
            PimdirIndexOp::SetHorizon(horizon) => {
                tx.execute(
                    search::SET_HORIZON,
                    named_params! {
                        ":horizon_start": horizon.start,
                        ":horizon_end": horizon.end,
                    },
                )?;
            }
            PimdirIndexOp::SetTzdbVersion(version) => {
                tx.execute(
                    search::SET_TZDB_VERSION,
                    named_params! { ":tzdb_version": version },
                )?;
            }
            PimdirIndexOp::SetFloatingZone(zone) => {
                tx.execute(
                    search::SET_FLOATING_ZONE,
                    named_params! { ":floating_zone": zone },
                )?;
            }
        }
        Ok(())
    }

    /// Writes a body indexed for the first time: its `object` row, its
    /// categories, its text keyed on the row's id, and its chunks.
    fn put_object(&self, object: PimdirIndexObject) -> Result<(), PimdirError> {
        let tx = self.tx;
        let id: i64 = tx.query_row(
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
            tx.execute(
                search::INSERT_OBJECT_CATEGORY,
                named_params! { ":object": id, ":value": value },
            )?;
        }

        let text = &object.text;
        tx.execute(
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
        for (ord, chunk) in object.chunks.iter().enumerate() {
            tx.execute(
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
        Ok(())
    }

    /// Writes a placement's summary text keyed on its row's id.
    fn put_summary_text(&self, id: i64, text: &PimdirIndexText) -> Result<(), PimdirError> {
        self.tx.execute(
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
        Ok(())
    }
}

/// One live item's summary as the fold reads it (SEARCH §6): its summary
/// row by key, a seek on each statement, its lists, and its address rows.
pub(crate) fn summary(
    conn: &Connection,
    collection: &str,
    seq: i64,
    kind: &str,
) -> Result<PimdirIndexSummary, PimdirError> {
    let key = named_params! { ":collection": collection, ":seq": seq };
    let mut link_id = None;
    let mut found = None;
    for table in tables_of(kind) {
        let item = conn
            .prepare(table.get_sql())?
            .query_row(key, |row| {
                Ok((row.get::<_, String>(1)?, table.read_row(row, 6)?))
            })
            .optional()?;
        let Some((link, summary)) = item else {
            break;
        };
        link_id = Some(link);
        if summary.is_some() {
            found = summary;
            break;
        }
    }
    let Some(link_id) = link_id else {
        return Ok(PimdirIndexSummary::default());
    };

    let item = named_params! { ":collection": collection, ":link_id": link_id };
    let collection = named_params! { ":collection": collection };
    match &mut found {
        Some(PimdirSummary::Mail(mail)) => {
            bind_list(conn, "links", [&link_id])?;
            mail.in_reply_to = rows(conn, sql::LOAD_IN_REPLY_TO_BY_LINK, collection, |r| {
                r.get(2)
            })?;
        }
        Some(PimdirSummary::Task(task)) => {
            bind_list(conn, "links", [&link_id])?;
            task.categories = rows(conn, sql::LOAD_CATEGORIES_BY_LINK, collection, |r| r.get(2))?;
        }
        _ => (),
    }

    let mut addresses = Vec::new();
    let read = rows(conn, sql::LOAD_ADDRESSES, item, |r| {
        Ok((
            r.get::<_, String>(0)?,
            PimdirAddress {
                address: r.get(2)?,
                name: r.get(3)?,
            },
        ))
    })?;
    for (role, address) in read {
        if let Some(role) = PimdirAddressRole::parse(&role) {
            addresses.push((role, address));
        }
    }

    Ok(PimdirIndexSummary {
        summary: found,
        addresses,
    })
}
