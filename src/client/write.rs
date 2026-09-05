//! # The seam
//!
//! The load and the write of one source (STORAGE §14): a load projects
//! the hub for the source with its probes and checkpoint, and a write
//! folds a batch into the hub narrowed to the link ids it names, then
//! persists only the rows that moved. Probes, retention, refcounts and
//! the generation bump ride the same transaction.

use alloc::{string::String, vec, vec::Vec};

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};

use rusqlite::{
    Connection, OptionalExtension, Row, TransactionBehavior, named_params, types::ToSql,
};

use crate::{
    change::{PimdirDropReason, PimdirWriteOp},
    client::{PimdirError, PimdirSourceStore, blobs::PimdirBlobs, busy_or_sql, release_pins, rows},
    codec,
    collection::{PimdirCheckpoint, PimdirCollectionId},
    hub::{PimdirBinding, PimdirHub, PimdirHubItem, PimdirSourceId},
    load::{PimdirLoadScope, PimdirLoaded},
    object::PimdirHash,
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirOrigin,
        PimdirPlacement, PimdirSortKey, PimdirStatus,
    },
    sql,
    summary::{
        PimdirAddress, PimdirAddressRole, PimdirSummary,
        calendar::{PimdirEventSummary, PimdirJournalSummary, PimdirTaskSummary, PimdirTime},
        contact::PimdirContactSummary,
        mail::PimdirMailSummary,
    },
};

/// The sync seam of one source.
impl PimdirSourceStore {
    /// Loads a collection as this source sees it (SYNC §3, §10).
    ///
    /// `scope` is a floor: the projection holds at least the placements
    /// it names. A handle no binding holds is a probe, and a probe has
    /// no link id, so a `Links` scope yields none of them; nor does it
    /// yield the copy the hub offers for an item this source lacks, which
    /// is no row the source holds under the key. A `Created` placement
    /// carries its origin and a `Tombstone` its destination, both read
    /// from this source's bindings elsewhere (SYNC §3).
    pub fn load(
        &self,
        collection: &PimdirCollectionId,
        scope: &PimdirLoadScope,
    ) -> Result<PimdirLoaded, PimdirError> {
        let conn = &self.store.reader.conn;
        let hub = match scope {
            PimdirLoadScope::All => read_hub(conn, &collection.0, None)?,
            PimdirLoadScope::Links(links) => {
                let links: Vec<String> = links.iter().map(|l| l.0.clone()).collect();
                read_hub(conn, &collection.0, Some(&links))?
            }
            PimdirLoadScope::Handles(handles) => {
                let mut links = Vec::new();
                for handle in handles {
                    links.extend(link_for_handle(conn, &collection.0, &self.source, handle)?);
                }
                read_hub(conn, &collection.0, Some(&links))?
            }
        };

        let mut failed = None;
        let mut placements = hub.project_with(collection, &self.source, |placement| {
            origin_for(conn, &self.source, placement).unwrap_or_else(|err| {
                failed = Some(err);
                None
            })
        });
        if let Some(err) = failed {
            return Err(err.into());
        }
        for placement in &mut placements {
            if placement.status == PimdirStatus::Tombstone {
                placement.origin = destination_for(conn, &self.source, placement)?;
            }
        }
        if matches!(scope, PimdirLoadScope::Links(_)) {
            placements.retain(|placement| {
                placement.link_id.as_ref().is_some_and(|link| {
                    hub.items
                        .get(link)
                        .is_some_and(|item| item.sources.contains_key(&self.source))
                })
            });
        } else {
            let probes = match scope {
                PimdirLoadScope::Handles(handles) => {
                    probes_by_handle(conn, &collection.0, &self.source, handles)?
                }
                _ => probes(conn, &collection.0, &self.source)?,
            };
            for (handle, flags) in probes {
                placements.push(PimdirPlacement {
                    collection: collection.clone(),
                    handle,
                    link_id: None,
                    object: None,
                    level: PimdirLevel::Probed,
                    summary: None,
                    sort_key: PimdirSortKey::default(),
                    flags,
                    status: PimdirStatus::Clean,
                    conflict_revision: None,
                    conflict_object: None,
                    base: None,
                    origin: None,
                });
            }
        }

        let checkpoint = conn
            .query_row(
                sql::LOAD_CHECKPOINT,
                named_params! { ":collection": collection.0, ":source": self.source.0 },
                |r| r.get::<_, Option<Vec<u8>>>(0),
            )
            .optional()?
            .flatten()
            .map(PimdirCheckpoint);

        Ok(PimdirLoaded {
            placements,
            checkpoint,
        })
    }

    /// Resolves which link ids already hold a body, within this handle's
    /// account (§14); a writer-derived key never matches (§9).
    pub fn lookup_objects(
        &self,
        links: &[PimdirLinkId],
    ) -> Result<BTreeMap<PimdirLinkId, PimdirHash>, PimdirError> {
        let ids: Vec<&str> = links.iter().map(|l| l.0.as_str()).collect();
        let found = rows(
            &self.store.reader.conn,
            sql::LOOKUP_OBJECTS,
            named_params! {
                ":links": serde_json::to_string(&ids)?,
                ":account": self.store.account.as_deref(),
            },
            |r| {
                Ok((
                    PimdirLinkId(r.get::<_, String>(0)?),
                    PimdirHash(r.get::<_, String>(1)?),
                ))
            },
        )?;

        Ok(found.into_iter().collect())
    }

    /// Applies a write batch atomically and in order (§14).
    ///
    /// The bodies land before the transaction opens, so the writer lock
    /// is never held across a file write; `BEGIN IMMEDIATE` takes it up
    /// front so contention fails fast as [`PimdirError::Busy`].
    pub fn write(&mut self, ops: Vec<PimdirWriteOp>) -> Result<(), PimdirError> {
        let blobs = self.store.reader.blobs();
        let lock = Arc::clone(&self.store.lock);
        let _writing = lock.writing();
        blobs.stage(&ops)?;

        let tx = self
            .store
            .reader
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(busy_or_sql)?;
        apply(
            &tx,
            &blobs,
            &self.source,
            self.store.account.as_deref(),
            ops,
        )?;
        tx.commit().map_err(busy_or_sql)?;
        Ok(())
    }
}

/// Applies a batch inside the caller's transaction.
///
/// Objects and checkpoints are written as they come. A placement with
/// no link id lands on the binding its handle holds (§10) or, when none
/// does, as a probe; a placement whose handle is bound to another link
/// id retires that binding first, as a `Deleted` drop of the handle
/// would (§10); every other placement op folds into the hub per
/// collection, and the diff between the hub before and after is what
/// reaches the rows.
pub(crate) fn apply(
    tx: &Connection,
    blobs: &PimdirBlobs,
    source: &PimdirSourceId,
    account: Option<&str>,
    ops: Vec<PimdirWriteOp>,
) -> Result<(), PimdirError> {
    let mut hub_ops: BTreeMap<String, Vec<PimdirWriteOp>> = BTreeMap::new();
    let mut licensed: BTreeMap<String, BTreeSet<PimdirHandle>> = BTreeMap::new();
    let mut dropped: BTreeMap<String, BTreeSet<PimdirHandle>> = BTreeMap::new();
    let mut rekeyed: BTreeSet<String> = BTreeSet::new();

    for op in ops {
        match op {
            PimdirWriteOp::StoreObject { object, body } => {
                if let Some(body) = body {
                    blobs.write(&object.hash, &body)?;
                }
                tx.execute(
                    sql::STORE_OBJECT,
                    named_params! { ":hash": object.hash.0, ":size": object.size as i64 },
                )?;
            }
            PimdirWriteOp::SetCheckpoint {
                collection,
                checkpoint,
            } => {
                ensure_collection(tx, &collection.0, account)?;
                tx.execute(
                    sql::UPSERT_CHECKPOINT,
                    named_params! {
                        ":collection": collection.0,
                        ":source": source.0,
                        ":checkpoint": checkpoint.0,
                    },
                )?;
            }
            PimdirWriteOp::UpsertPlacement(mut placement) => {
                ensure_collection(tx, &placement.collection.0, account)?;
                let bound =
                    link_for_handle(tx, &placement.collection.0, source, &placement.handle)?
                        .map(PimdirLinkId);
                if placement.link_id.is_none() {
                    placement.link_id = bound.clone();
                }
                let Some(link) = &placement.link_id else {
                    tx.execute(
                        sql::UPSERT_PROBE,
                        named_params! {
                            ":collection": placement.collection.0,
                            ":source": source.0,
                            ":handle": placement.handle.0,
                            ":flags": codec::flags_to_json(&placement.flags),
                        },
                    )?;
                    continue;
                };
                delete_probe(tx, &placement.collection.0, source, &placement.handle)?;
                let ops = hub_ops.entry(placement.collection.0.clone()).or_default();
                let already_dropped = dropped
                    .get(&placement.collection.0)
                    .is_some_and(|handles| handles.contains(&placement.handle));
                if bound.is_some_and(|bound| bound != *link) && !already_dropped {
                    ops.push(PimdirWriteOp::DropPlacement {
                        collection: placement.collection.clone(),
                        handle: placement.handle.clone(),
                        reason: PimdirDropReason::Deleted,
                    });
                }
                ops.push(PimdirWriteOp::UpsertPlacement(placement));
            }
            PimdirWriteOp::DropPlacement {
                collection,
                handle,
                reason,
            } => {
                delete_probe(tx, &collection.0, source, &handle)?;
                dropped
                    .entry(collection.0.clone())
                    .or_default()
                    .insert(handle.clone());
                if reason != PimdirDropReason::Deleted {
                    licensed
                        .entry(collection.0.clone())
                        .or_default()
                        .insert(handle.clone());
                }
                if reason == PimdirDropReason::Rekeyed {
                    rekeyed.insert(collection.0.clone());
                }
                hub_ops.entry(collection.0.clone()).or_default().push(
                    PimdirWriteOp::DropPlacement {
                        collection,
                        handle,
                        reason,
                    },
                );
            }
        }
    }

    for (collection, ops) in hub_ops {
        refuse_colliding_upserts(&collection, source, &ops)?;
        let links = batch_links(tx, &collection, source, &ops)?;
        let old = read_hub(tx, &collection, Some(&links))?;
        let mut new = old.clone();
        new.absorb(source, &ops);
        let licensed = licensed.remove(&collection).unwrap_or_default();
        save_hub_diff(tx, &collection, source, &old, &new, &licensed)?;
        adjust_refcounts(tx, &object_refs(&old), &object_refs(&new))?;
    }

    for collection in rekeyed {
        tx.query_row(
            sql::BUMP_GENERATION,
            named_params! { ":collection": collection },
            |row| row.get::<_, i64>(0),
        )?;
    }

    Ok(())
}

fn ensure_collection(
    conn: &Connection,
    collection: &str,
    account: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        sql::ENSURE_COLLECTION,
        named_params! { ":collection": collection, ":account": account },
    )?;
    Ok(())
}

fn delete_probe(
    conn: &Connection,
    collection: &str,
    source: &PimdirSourceId,
    handle: &PimdirHandle,
) -> rusqlite::Result<()> {
    conn.execute(
        sql::DELETE_PROBE,
        named_params! { ":collection": collection, ":source": source.0, ":handle": handle.0 },
    )?;
    Ok(())
}

/// One source's probes of a collection: the unnamed handles with the
/// flags the enumeration reported.
fn probes(
    conn: &Connection,
    collection: &str,
    source: &PimdirSourceId,
) -> rusqlite::Result<Vec<(PimdirHandle, PimdirFlags)>> {
    rows(
        conn,
        sql::LOAD_PROBES,
        named_params! { ":collection": collection, ":source": source.0 },
        probe_from_row,
    )
}

/// The probes of a `Handles` load: the unnamed handles among those asked
/// for (§14), bound as a JSON array.
fn probes_by_handle(
    conn: &Connection,
    collection: &str,
    source: &PimdirSourceId,
    handles: &[PimdirHandle],
) -> Result<Vec<(PimdirHandle, PimdirFlags)>, PimdirError> {
    let handles: Vec<&str> = handles.iter().map(|handle| handle.0.as_str()).collect();
    Ok(rows(
        conn,
        sql::LOAD_PROBES_BY_HANDLE,
        named_params! {
            ":collection": collection,
            ":source": source.0,
            ":handles": serde_json::to_string(&handles)?,
        },
        probe_from_row,
    )?)
}

fn probe_from_row(row: &Row) -> rusqlite::Result<(PimdirHandle, PimdirFlags)> {
    Ok((
        PimdirHandle(row.get(0)?),
        codec::flags_from_json(row.get::<_, Option<String>>(1)?.as_deref()),
    ))
}

/// Refuses a batch binding one link id to two handles, unless a
/// `Superseded` or `Rekeyed` drop between them licenses it (SYNC §10).
fn refuse_colliding_upserts(
    collection: &str,
    source: &PimdirSourceId,
    ops: &[PimdirWriteOp],
) -> Result<(), PimdirError> {
    let mut claimed: BTreeMap<&PimdirLinkId, &PimdirHandle> = BTreeMap::new();

    for op in ops {
        let placement = match op {
            PimdirWriteOp::UpsertPlacement(placement) => placement,
            PimdirWriteOp::DropPlacement { handle, reason, .. }
                if *reason != PimdirDropReason::Deleted =>
            {
                claimed.retain(|_, bound| *bound != handle);
                continue;
            }
            _ => continue,
        };
        let Some(link) = placement.link_id.as_ref() else {
            continue;
        };
        if let Some(bound) = claimed.insert(link, &placement.handle)
            && *bound != placement.handle
        {
            return Err(PimdirError::Rebind {
                collection: collection.into(),
                link_id: link.0.clone(),
                source: source.0.clone(),
                bound: bound.0.clone(),
                incoming: placement.handle.0.clone(),
            });
        }
    }

    Ok(())
}

/// The link id one source's handle is bound to, a seek on bindings_by_handle.
pub(crate) fn link_for_handle(
    conn: &Connection,
    collection: &str,
    source: &PimdirSourceId,
    handle: &PimdirHandle,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        sql::LINK_FOR_HANDLE,
        named_params! {
            ":collection": collection,
            ":source": source.0,
            ":handle": handle.0,
        },
        |r| r.get::<_, String>(0),
    )
    .optional()
}

/// Where this source already binds the placement's identity and body in
/// another collection (SYNC §3), so its push is a server-side copy.
fn origin_for(
    conn: &Connection,
    source: &PimdirSourceId,
    placement: &PimdirPlacement,
) -> rusqlite::Result<Option<PimdirOrigin>> {
    let Some(link) = &placement.link_id else {
        return Ok(None);
    };
    conn.query_row(
        sql::ORIGIN_FOR_LINK,
        named_params! {
            ":collection": placement.collection.0,
            ":source": source.0,
            ":link_id": link.0,
            ":object": placement.object.as_ref().map(|o| &o.0),
        },
        |r| {
            Ok(PimdirOrigin {
                collection: PimdirCollectionId(r.get(0)?),
                handle: PimdirHandle(r.get(1)?),
            })
        },
    )
    .optional()
}

/// Where this source holds a pending create of the placement's identity
/// in another collection (SYNC §3): the destination a `Tombstone`
/// carries, under its own handle, so its remove is a relocation.
fn destination_for(
    conn: &Connection,
    source: &PimdirSourceId,
    placement: &PimdirPlacement,
) -> rusqlite::Result<Option<PimdirOrigin>> {
    let Some(link) = &placement.link_id else {
        return Ok(None);
    };
    conn.query_row(
        sql::DESTINATION_FOR_LINK,
        named_params! {
            ":collection": placement.collection.0,
            ":source": source.0,
            ":link_id": link.0,
        },
        |r| {
            Ok(PimdirOrigin {
                collection: PimdirCollectionId(r.get(0)?),
                handle: placement.handle.clone(),
            })
        },
    )
    .optional()
}

/// The link ids one batch touches: its upserts' and the ones its drops
/// and upserted handles resolve to; a handle nothing binds is left out.
fn batch_links(
    conn: &Connection,
    collection: &str,
    source: &PimdirSourceId,
    ops: &[PimdirWriteOp],
) -> rusqlite::Result<Vec<String>> {
    let mut links: BTreeSet<String> = BTreeSet::new();

    for op in ops {
        let handle = match op {
            PimdirWriteOp::UpsertPlacement(placement) => {
                if let Some(link) = &placement.link_id {
                    links.insert(link.0.clone());
                }
                &placement.handle
            }
            PimdirWriteOp::DropPlacement { handle, .. } => handle,
            _ => continue,
        };
        links.extend(link_for_handle(conn, collection, source, handle)?);
    }

    Ok(links.into_iter().collect())
}

/// Reads a collection's hub: its policy, items, bindings, summaries and
/// addresses, the whole collection with `None` or the named link ids.
pub(crate) fn read_hub(
    conn: &Connection,
    collection: &str,
    links: Option<&[String]>,
) -> Result<PimdirHub, PimdirError> {
    let mut hub = PimdirHub::default();

    if let Some(policy) = conn
        .query_row(
            sql::LOAD_CONFLICT,
            named_params! { ":collection": collection },
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        hub.conflict = codec::conflict_from_str(&policy);
    }

    let scope = links.map(serde_json::to_string).transpose()?;
    let (items_sql, bindings_sql) = match scope {
        Some(_) => (sql::LOAD_ITEMS_BY_LINK, sql::LOAD_BINDINGS_BY_LINK),
        None => (sql::LOAD_ITEMS, sql::LOAD_BINDINGS),
    };
    let mut params: Vec<(&str, &dyn ToSql)> = vec![(":collection", &collection)];
    if let Some(scope) = &scope {
        params.push((":links", scope));
    }

    for (link, item) in rows(conn, items_sql, params.as_slice(), item_from_row)? {
        hub.items.insert(link, item);
    }
    for (link, source, binding) in rows(conn, bindings_sql, params.as_slice(), binding_from_row)? {
        if let Some(item) = hub.items.get_mut(&link) {
            item.sources.insert(source, binding);
        }
    }

    let kind = kind_of(conn, collection)?;
    for table in tables_of(&kind) {
        for (link, summary) in load_summaries(conn, table, collection, scope.as_deref())? {
            if let Some(item) = hub.items.get_mut(&link) {
                item.summary = Some(summary);
            }
        }
    }
    for (link, role, address) in load_addresses(conn, collection, scope.as_deref())? {
        if let Some(summary) = hub
            .items
            .get_mut(&link)
            .and_then(|item| item.summary.as_mut())
        {
            attach_address(summary, role, address);
        }
    }

    Ok(hub)
}

/// The declared kind of a collection, empty when undeclared.
pub(crate) fn kind_of(conn: &Connection, collection: &str) -> rusqlite::Result<String> {
    conn.query_row(
        sql::LOAD_KIND,
        named_params! { ":collection": collection },
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map(|kind| kind.unwrap_or_default())
}

/// The summary tables a collection's kind may hold rows in; every table
/// for a collection whose kind was never declared.
pub(crate) fn tables_of(kind: &str) -> Vec<PimdirSummaryTable> {
    match kind.split(';').next().unwrap_or_default().trim() {
        "message/rfc822" => vec![PimdirSummaryTable::Mail],
        "text/vcard" => vec![PimdirSummaryTable::Contact],
        "text/calendar" => vec![
            PimdirSummaryTable::Event,
            PimdirSummaryTable::Task,
            PimdirSummaryTable::Journal,
        ],
        _ => vec![
            PimdirSummaryTable::Mail,
            PimdirSummaryTable::Contact,
            PimdirSummaryTable::Event,
            PimdirSummaryTable::Task,
            PimdirSummaryTable::Journal,
        ],
    }
}

/// One of the five summary tables (§4.3).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PimdirSummaryTable {
    Mail,
    Contact,
    Event,
    Task,
    Journal,
}

impl PimdirSummaryTable {
    /// The table a summary variant is written to.
    fn of(summary: &PimdirSummary) -> Self {
        match summary {
            PimdirSummary::Mail(_) => Self::Mail,
            PimdirSummary::Contact(_) => Self::Contact,
            PimdirSummary::Event(_) => Self::Event,
            PimdirSummary::Task(_) => Self::Task,
            PimdirSummary::Journal(_) => Self::Journal,
        }
    }

    fn load_sql(self) -> &'static str {
        match self {
            Self::Mail => sql::LOAD_MAIL_SUMMARIES,
            Self::Contact => sql::LOAD_CONTACT_SUMMARIES,
            Self::Event => sql::LOAD_EVENT_SUMMARIES,
            Self::Task => sql::LOAD_TASK_SUMMARIES,
            Self::Journal => sql::LOAD_JOURNAL_SUMMARIES,
        }
    }

    /// The delete a component rewritten as another runs; a mail or contact
    /// row never leaves its item, the collection's kind being fixed.
    fn delete_sql(self) -> Option<&'static str> {
        match self {
            Self::Mail | Self::Contact => None,
            Self::Event => Some(sql::DELETE_EVENT_SUMMARY),
            Self::Task => Some(sql::DELETE_TASK_SUMMARY),
            Self::Journal => Some(sql::DELETE_JOURNAL_SUMMARY),
        }
    }

    /// Maps a row whose summary columns start at `at`, addresses empty.
    pub(crate) fn read_row(self, row: &Row, at: usize) -> rusqlite::Result<Option<PimdirSummary>> {
        let text = |offset: usize| row.get::<_, Option<String>>(at + offset);
        let flag = |offset: usize| {
            Ok::<_, rusqlite::Error>(row.get::<_, Option<i64>>(at + offset)?.map(|v| v != 0))
        };
        let time = |value: usize, tzid: usize, kind: usize| {
            Ok::<_, rusqlite::Error>(text(value)?.map(|value| PimdirTime {
                value,
                tzid: text(tzid).ok().flatten(),
                date: text(kind).ok().flatten().as_deref() == Some("date"),
            }))
        };

        // NOTE: a LEFT JOIN yields the row with every summary column NULL
        // for an item that has none; the NOT NULL text column says which.
        let present = |offset: usize| row.get::<_, Option<String>>(at + offset);

        let summary = match self {
            Self::Mail => {
                let Some(subject) = present(2)? else {
                    return Ok(None);
                };
                PimdirSummary::Mail(PimdirMailSummary {
                    message_id: text(0)?,
                    in_reply_to: text(1)?
                        .map(|ids| codec::ids_from_json(&ids))
                        .unwrap_or_default(),
                    subject,
                    sender: text(3)?,
                    sender_name: text(4)?,
                    date: text(5)?,
                    size: row
                        .get::<_, Option<i64>>(at + 6)?
                        .map(|size| size.max(0) as u64),
                    attachment: flag(7)?,
                    ..Default::default()
                })
            }
            Self::Contact => {
                let Some(full_name) = present(1)? else {
                    return Ok(None);
                };
                PimdirSummary::Contact(PimdirContactSummary {
                    uid: text(0)?,
                    full_name,
                    kind: text(2)?,
                    org: text(3)?,
                    emails: Vec::new(),
                })
            }
            Self::Event => {
                let Some(summary) = present(1)? else {
                    return Ok(None);
                };
                PimdirSummary::Event(PimdirEventSummary {
                    uid: text(0)?,
                    summary,
                    location: text(2)?,
                    dtstart: time(3, 4, 5)?,
                    dtend: text(6)?,
                    recurring: flag(7)?,
                    until: text(8)?,
                    organizer: None,
                    attendees: Vec::new(),
                })
            }
            Self::Task => {
                let Some(summary) = present(1)? else {
                    return Ok(None);
                };
                PimdirSummary::Task(PimdirTaskSummary {
                    uid: text(0)?,
                    summary,
                    dtstart: time(2, 3, 4)?,
                    due: time(5, 6, 7)?,
                    status: text(8)?,
                    completed: text(9)?,
                    percent: row.get(at + 10)?,
                    recurring: flag(11)?,
                    until: text(12)?,
                    organizer: None,
                    attendees: Vec::new(),
                })
            }
            Self::Journal => {
                let Some(summary) = present(1)? else {
                    return Ok(None);
                };
                PimdirSummary::Journal(PimdirJournalSummary {
                    uid: text(0)?,
                    summary,
                    dtstart: time(2, 3, 4)?,
                    organizer: None,
                    attendees: Vec::new(),
                })
            }
        };

        Ok(Some(summary))
    }
}

/// The summaries a table holds for a collection, or for `:links`.
pub(crate) fn load_summaries(
    conn: &Connection,
    table: PimdirSummaryTable,
    collection: &str,
    links: Option<&str>,
) -> rusqlite::Result<Vec<(PimdirLinkId, PimdirSummary)>> {
    let loaded = rows(
        conn,
        table.load_sql(),
        named_params! { ":collection": collection, ":links": links },
        |row| Ok((PimdirLinkId(row.get(0)?), table.read_row(row, 1)?)),
    )?;

    Ok(loaded
        .into_iter()
        .filter_map(|(link, summary)| Some((link, summary?)))
        .collect())
}

/// The addresses of a collection's items, or of `:links`, in document order.
pub(crate) fn load_addresses(
    conn: &Connection,
    collection: &str,
    links: Option<&str>,
) -> rusqlite::Result<Vec<(PimdirLinkId, PimdirAddressRole, PimdirAddress)>> {
    let loaded = rows(
        conn,
        sql::LOAD_ADDRESSES_BY_LINK,
        named_params! { ":collection": collection, ":links": links },
        |row| {
            Ok((
                PimdirLinkId(row.get(0)?),
                row.get::<_, String>(1)?,
                PimdirAddress {
                    address: row.get(3)?,
                    name: row.get(4)?,
                },
            ))
        },
    )?;

    Ok(loaded
        .into_iter()
        .filter_map(|(link, role, address)| Some((link, PimdirAddressRole::parse(&role)?, address)))
        .collect())
}

/// Files an address row under the field its role names.
pub(crate) fn attach_address(
    summary: &mut PimdirSummary,
    role: PimdirAddressRole,
    address: PimdirAddress,
) {
    match (summary, role) {
        (PimdirSummary::Mail(mail), PimdirAddressRole::From) => mail.from.push(address),
        (PimdirSummary::Mail(mail), PimdirAddressRole::To) => mail.to.push(address),
        (PimdirSummary::Mail(mail), PimdirAddressRole::Cc) => mail.cc.push(address),
        (PimdirSummary::Mail(mail), PimdirAddressRole::Bcc) => mail.bcc.push(address),
        (PimdirSummary::Contact(contact), PimdirAddressRole::Email) => contact.emails.push(address),
        (PimdirSummary::Event(event), PimdirAddressRole::Organizer) => {
            event.organizer = Some(address)
        }
        (PimdirSummary::Event(event), PimdirAddressRole::Attendee) => event.attendees.push(address),
        (PimdirSummary::Task(task), PimdirAddressRole::Organizer) => task.organizer = Some(address),
        (PimdirSummary::Task(task), PimdirAddressRole::Attendee) => task.attendees.push(address),
        (PimdirSummary::Journal(journal), PimdirAddressRole::Organizer) => {
            journal.organizer = Some(address)
        }
        (PimdirSummary::Journal(journal), PimdirAddressRole::Attendee) => {
            journal.attendees.push(address)
        }
        _ => {}
    }
}

/// Persists the change from `old` to `new` for a collection's hub by
/// diffing the two and issuing only the writes that differ.
///
/// The bindings the diff releases go first, every item's, before any
/// binding is inserted: a handle names one item per source (§10) and the
/// index is unique, so a handle moving between two items in one batch
/// has to leave before it arrives. An item no source holds any more is
/// retained (§11), or purged when the account holds it live in another
/// collection, `source` naming the side whose removal retired it.
/// `licensed` carries the handles this batch superseded or rekeyed, the
/// one thing the two hubs cannot say: a rebuilt spine and a duplicated
/// identity produce the same diff.
fn save_hub_diff(
    conn: &Connection,
    collection: &str,
    source: &PimdirSourceId,
    old: &PimdirHub,
    new: &PimdirHub,
    licensed: &BTreeSet<PimdirHandle>,
) -> Result<(), PimdirError> {
    for (link, item) in &new.items {
        if let Some(prev) = old.items.get(link) {
            release_bindings(conn, collection, link, prev, item, licensed)?;
        }
    }

    for (link, item) in &new.items {
        let prev = old.items.get(link);
        let unbound = item.sources.is_empty();

        match prev {
            None if unbound => continue,
            None => {
                insert_item(conn, collection, link, item)?;
                write_summary(conn, collection, link, None, item.summary.as_ref())?;
            }
            Some(prev) => {
                let columns_moved = !item_columns_eq(prev, item);
                if columns_moved {
                    update_item(conn, collection, link, item)?;
                }
                let summary_moved = write_summary(
                    conn,
                    collection,
                    link,
                    prev.summary.as_ref(),
                    item.summary.as_ref(),
                )?;
                if summary_moved && !columns_moved {
                    conn.execute(
                        sql::STAMP_ITEM,
                        named_params! { ":collection": collection, ":link_id": link.0 },
                    )?;
                    conn.execute(sql::BUMP_NEXT_CHANGE, [])?;
                }
                save_bindings_diff(conn, collection, link, prev, item, licensed)?;
            }
        }

        if unbound && prev.is_some_and(|prev| !prev.sources.is_empty()) {
            conn.execute(
                sql::RETAIN_ITEM,
                named_params! { ":collection": collection, ":link_id": link.0, ":source": source.0 },
            )?;
            conn.execute(
                sql::DELETE_ITEM_BINDINGS,
                named_params! { ":collection": collection, ":link_id": link.0 },
            )?;
            if held_elsewhere(conn, collection, link)? {
                purge_retained(conn, collection, link)?;
            }
        }
    }

    Ok(())
}

/// Whether the same account holds `link` live in another collection (§11).
fn held_elsewhere(
    conn: &Connection,
    collection: &str,
    link: &PimdirLinkId,
) -> rusqlite::Result<bool> {
    conn.query_row(
        sql::HELD_ELSEWHERE,
        named_params! { ":collection": collection, ":link_id": link.0 },
        |_| Ok(()),
    )
    .optional()
    .map(|held| held.is_some())
}

/// Purges the row just retained under `(collection, link)`: a move loses
/// nothing, the holder pinning the body (§11).
fn purge_retained(
    conn: &Connection,
    collection: &str,
    link: &PimdirLinkId,
) -> Result<(), PimdirError> {
    let seq: i64 = conn.query_row(
        sql::RETAINED_ITEM,
        named_params! { ":collection": collection, ":link_id": link.0 },
        |r| r.get(0),
    )?;
    let (object, conflict_object): (Option<String>, Option<String>) = conn.query_row(
        sql::PURGE_ITEM,
        named_params! { ":collection": collection, ":seq": seq },
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    release_pins(conn, [object, conflict_object].into_iter().flatten())
}

/// Whether two items' own columns match, bindings and summary aside.
///
/// Every column `update_item` writes is here: one left out could never
/// change again.
fn item_columns_eq(a: &PimdirHubItem, b: &PimdirHubItem) -> bool {
    a.flags == b.flags
        && a.object == b.object
        && a.sort_key == b.sort_key
        && a.level == b.level
        && a.deleted == b.deleted
        && a.conflicted == b.conflicted
        && a.conflict_object == b.conflict_object
}

fn insert_item(
    conn: &Connection,
    collection: &str,
    link: &PimdirLinkId,
    item: &PimdirHubItem,
) -> rusqlite::Result<()> {
    if revive_item(conn, collection, link, item)? {
        return Ok(());
    }

    // NOTE: a derived key states nothing the content carries and never
    // shares a public id (§9.1).
    let derived = ["alt:", "hash:", "dup:"]
        .iter()
        .any(|prefix| link.0.starts_with(prefix));
    let existing = match derived {
        true => None,
        false => conn
            .query_row(
                sql::SEQ_FOR_LINK_ANY,
                named_params! { ":link_id": link.0 },
                |row| row.get::<_, i64>(0),
            )
            .optional()?,
    };
    let seq = match existing {
        Some(seq) => seq,
        None => conn.query_row(sql::BUMP_NEXT_SEQ, [], |row| row.get(0))?,
    };
    conn.execute(
        sql::INSERT_ITEM,
        named_params! {
            ":collection": collection,
            ":link_id": link.0,
            ":seq": seq,
            ":flags": codec::flags_to_json(&item.flags),
            ":object_hash": item.object.as_ref().map(|o| o.0.as_str()),
            ":sort_key": item.sort_key.0.as_str(),
            ":level": codec::level_to_int(item.level),
            ":deleted": item.deleted as i64,
            ":conflicted": item.conflicted as i64,
            ":conflict_object": item.conflict_object.as_ref().map(|o| o.0.as_str()),
        },
    )?;
    for (source, binding) in &item.sources {
        insert_binding(conn, collection, link, source, binding)?;
    }
    Ok(())
}

/// Revives the retained row holding `(collection, link)`, if any (§11):
/// it stops being retained, adopts the incoming content and binds the
/// sources, and the pins the retained row held are released as the
/// caller's refcount diff takes the live ones.
fn revive_item(
    conn: &Connection,
    collection: &str,
    link: &PimdirLinkId,
    item: &PimdirHubItem,
) -> rusqlite::Result<bool> {
    let pinned: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            sql::RETAINED_ITEM,
            named_params! { ":collection": collection, ":link_id": link.0 },
            |row| Ok((row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((object, conflict_object)) = pinned else {
        return Ok(false);
    };

    conn.execute(
        sql::REVIVE_ITEM,
        named_params! { ":collection": collection, ":link_id": link.0 },
    )?;
    update_item(conn, collection, link, item)?;
    for hash in [object, conflict_object].into_iter().flatten() {
        conn.execute(
            sql::ADJUST_REFCOUNT,
            named_params! { ":delta": -1, ":hash": hash },
        )?;
    }
    for (source, binding) in &item.sources {
        insert_binding(conn, collection, link, source, binding)?;
    }
    Ok(true)
}

fn update_item(
    conn: &Connection,
    collection: &str,
    link: &PimdirLinkId,
    item: &PimdirHubItem,
) -> rusqlite::Result<()> {
    conn.execute(
        sql::UPDATE_ITEM,
        named_params! {
            ":collection": collection,
            ":link_id": link.0,
            ":flags": codec::flags_to_json(&item.flags),
            ":object_hash": item.object.as_ref().map(|o| o.0.as_str()),
            ":sort_key": item.sort_key.0.as_str(),
            ":level": codec::level_to_int(item.level),
            ":deleted": item.deleted as i64,
            ":conflicted": item.conflicted as i64,
            ":conflict_object": item.conflict_object.as_ref().map(|o| o.0.as_str()),
        },
    )?;
    Ok(())
}

/// Writes the summary and address rows when they moved, reporting
/// whether they did: the row of the old variant goes when the variant
/// changed, the addresses are replaced as a set (Annex A.6).
fn write_summary(
    conn: &Connection,
    collection: &str,
    link: &PimdirLinkId,
    old: Option<&PimdirSummary>,
    new: Option<&PimdirSummary>,
) -> rusqlite::Result<bool> {
    if old == new {
        return Ok(false);
    }

    let key = named_params! { ":collection": collection, ":link_id": link.0 };
    if let Some(old) = old
        && new.is_none_or(|new| PimdirSummaryTable::of(new) != PimdirSummaryTable::of(old))
        && let Some(delete) = PimdirSummaryTable::of(old).delete_sql()
    {
        conn.execute(delete, key)?;
    }
    conn.execute(sql::REPLACE_ADDRESSES, key)?;

    let Some(new) = new else {
        return Ok(true);
    };

    match new {
        PimdirSummary::Mail(mail) => conn.execute(
            sql::UPSERT_MAIL_SUMMARY,
            named_params! {
                ":collection": collection,
                ":link_id": link.0,
                ":message_id": mail.message_id,
                ":in_reply_to": codec::ids_to_json(&mail.in_reply_to),
                ":subject": mail.subject,
                ":sender": mail.sender,
                ":sender_name": mail.sender_name,
                ":date": mail.date,
                ":size": mail.size.map(|size| size as i64),
                ":attachment": mail.attachment.map(i64::from),
            },
        )?,
        PimdirSummary::Contact(contact) => conn.execute(
            sql::UPSERT_CONTACT_SUMMARY,
            named_params! {
                ":collection": collection,
                ":link_id": link.0,
                ":uid": contact.uid,
                ":fn": contact.full_name,
                ":kind": contact.kind,
                ":org": contact.org,
            },
        )?,
        PimdirSummary::Event(event) => conn.execute(
            sql::UPSERT_EVENT_SUMMARY,
            named_params! {
                ":collection": collection,
                ":link_id": link.0,
                ":uid": event.uid,
                ":summary": event.summary,
                ":location": event.location,
                ":dtstart": event.dtstart.as_ref().map(|t| t.value.as_str()),
                ":dtstart_tzid": event.dtstart.as_ref().and_then(|t| t.tzid.as_deref()),
                ":dtstart_value": event.dtstart.as_ref().map(PimdirTime::value_kind),
                ":dtend": event.dtend,
                ":recurring": event.recurring.map(i64::from),
                ":until": event.until,
            },
        )?,
        PimdirSummary::Task(task) => conn.execute(
            sql::UPSERT_TASK_SUMMARY,
            named_params! {
                ":collection": collection,
                ":link_id": link.0,
                ":uid": task.uid,
                ":summary": task.summary,
                ":dtstart": task.dtstart.as_ref().map(|t| t.value.as_str()),
                ":dtstart_tzid": task.dtstart.as_ref().and_then(|t| t.tzid.as_deref()),
                ":dtstart_value": task.dtstart.as_ref().map(PimdirTime::value_kind),
                ":due": task.due.as_ref().map(|t| t.value.as_str()),
                ":due_tzid": task.due.as_ref().and_then(|t| t.tzid.as_deref()),
                ":due_value": task.due.as_ref().map(PimdirTime::value_kind),
                ":status": task.status,
                ":completed": task.completed,
                ":percent": task.percent,
                ":recurring": task.recurring.map(i64::from),
                ":until": task.until,
            },
        )?,
        PimdirSummary::Journal(journal) => conn.execute(
            sql::UPSERT_JOURNAL_SUMMARY,
            named_params! {
                ":collection": collection,
                ":link_id": link.0,
                ":uid": journal.uid,
                ":summary": journal.summary,
                ":dtstart": journal.dtstart.as_ref().map(|t| t.value.as_str()),
                ":dtstart_tzid": journal.dtstart.as_ref().and_then(|t| t.tzid.as_deref()),
                ":dtstart_value": journal.dtstart.as_ref().map(PimdirTime::value_kind),
            },
        )?,
    };

    let mut position: HashMap<PimdirAddressRole, i64> = HashMap::new();
    for (role, address) in new.addresses() {
        let at = position.entry(role).or_insert(0);
        conn.execute(
            sql::INSERT_ADDRESS,
            named_params! {
                ":collection": collection,
                ":link_id": link.0,
                ":role": role.as_str(),
                ":position": *at,
                ":address": address.address,
                ":name": address.name,
            },
        )?;
        *at += 1;
    }

    Ok(true)
}

/// Deletes the bindings one item's diff releases: a source that no
/// longer binds it, and a binding moving to another handle under a
/// `Superseded` or `Rekeyed` drop. A binding resolved to another handle
/// without the licence is the one write no diff may express (§10) and
/// is refused.
fn release_bindings(
    conn: &Connection,
    collection: &str,
    link: &PimdirLinkId,
    old: &PimdirHubItem,
    new: &PimdirHubItem,
    licensed: &BTreeSet<PimdirHandle>,
) -> Result<(), PimdirError> {
    for (source, prev) in &old.sources {
        let released = match new.sources.get(source) {
            None => true,
            Some(binding) if binding.handle == prev.handle => false,
            Some(_) if licensed.contains(&prev.handle) => true,
            Some(binding) => {
                return Err(PimdirError::Rebind {
                    collection: collection.into(),
                    link_id: link.0.clone(),
                    source: source.0.clone(),
                    bound: prev.handle.0.clone(),
                    incoming: binding.handle.0.clone(),
                });
            }
        };
        if released {
            conn.execute(
                sql::DELETE_BINDING,
                named_params! { ":collection": collection, ":link_id": link.0, ":source": source.0 },
            )?;
        }
    }
    Ok(())
}

/// Inserts and updates one item's bindings, the releases having gone
/// first ([`release_bindings`]).
fn save_bindings_diff(
    conn: &Connection,
    collection: &str,
    link: &PimdirLinkId,
    old: &PimdirHubItem,
    new: &PimdirHubItem,
    licensed: &BTreeSet<PimdirHandle>,
) -> Result<(), PimdirError> {
    for (source, binding) in &new.sources {
        match old.sources.get(source) {
            None => insert_binding(conn, collection, link, source, binding)?,
            Some(prev) if binding.handle != prev.handle && licensed.contains(&prev.handle) => {
                insert_binding(conn, collection, link, source, binding)?
            }
            Some(prev) if prev != binding => {
                update_binding(conn, collection, link, source, binding)?
            }
            Some(_) => {}
        }
    }
    Ok(())
}

fn insert_binding(
    conn: &Connection,
    collection: &str,
    link: &PimdirLinkId,
    source: &PimdirSourceId,
    binding: &PimdirBinding,
) -> rusqlite::Result<()> {
    conn.execute(
        sql::INSERT_BINDING,
        named_params! {
            ":collection": collection,
            ":link_id": link.0,
            ":source": source.0,
            ":handle": binding.handle.0,
            ":base_flags": binding.base.as_ref().map(|b| codec::flags_to_json(&b.flags)),
            ":base_object": binding.base.as_ref().and_then(|b| b.object.as_ref()).map(|o| o.0.as_str()),
            ":base_revision": binding.base.as_ref().and_then(|b| b.revision.as_deref()),
            ":base_present": binding.base.is_some() as i64,
            ":conflicted": binding.conflicted as i64,
            ":conflict_revision": binding.conflicted.then_some(binding.conflict_revision.as_deref()).flatten(),
            ":conflict_object": conflict_object(binding).map(|hash| hash.0.as_str()),
            ":shared_object": binding.shared_object.as_ref().map(|hash| hash.0.as_str()),
        },
    )?;
    Ok(())
}

fn update_binding(
    conn: &Connection,
    collection: &str,
    link: &PimdirLinkId,
    source: &PimdirSourceId,
    binding: &PimdirBinding,
) -> rusqlite::Result<()> {
    conn.execute(
        sql::UPDATE_BINDING,
        named_params! {
            ":collection": collection,
            ":link_id": link.0,
            ":source": source.0,
            ":base_flags": binding.base.as_ref().map(|b| codec::flags_to_json(&b.flags)),
            ":base_object": binding.base.as_ref().and_then(|b| b.object.as_ref()).map(|o| o.0.as_str()),
            ":base_revision": binding.base.as_ref().and_then(|b| b.revision.as_deref()),
            ":base_present": binding.base.is_some() as i64,
            ":conflicted": binding.conflicted as i64,
            ":conflict_revision": binding.conflicted.then_some(binding.conflict_revision.as_deref()).flatten(),
            ":conflict_object": conflict_object(binding).map(|hash| hash.0.as_str()),
            ":shared_object": binding.shared_object.as_ref().map(|hash| hash.0.as_str()),
        },
    )?;
    Ok(())
}

/// The diverging body a binding is stuck on, as the column holds it:
/// the hash while conflicted, `NULL` otherwise (§13).
fn conflict_object(binding: &PimdirBinding) -> Option<&PimdirHash> {
    binding
        .conflicted
        .then_some(binding.conflict_object.as_ref())
        .flatten()
}

/// The multiset of object references a hub holds (§5): every item's body
/// and conflict body, every binding's base body and, while conflicted,
/// its diverging body. A binding's `shared_object` is never counted.
fn object_refs(hub: &PimdirHub) -> HashMap<String, i64> {
    let mut refs: HashMap<String, i64> = HashMap::new();
    let mut bump = |hash: &PimdirHash| *refs.entry(hash.0.clone()).or_insert(0) += 1;
    for item in hub.items.values() {
        if let Some(object) = &item.object {
            bump(object);
        }
        if let Some(conflict) = &item.conflict_object {
            bump(conflict);
        }
        for binding in item.sources.values() {
            if let Some(object) = binding.base.as_ref().and_then(|b| b.object.as_ref()) {
                bump(object);
            }
            if let Some(conflict) = conflict_object(binding) {
                bump(conflict);
            }
        }
    }
    refs
}

/// Applies the change between two reference multisets as per-hash deltas.
fn adjust_refcounts(
    conn: &Connection,
    old: &HashMap<String, i64>,
    new: &HashMap<String, i64>,
) -> rusqlite::Result<()> {
    for (hash, new_count) in new {
        let delta = new_count - old.get(hash).copied().unwrap_or(0);
        if delta != 0 {
            conn.execute(
                sql::ADJUST_REFCOUNT,
                named_params! { ":delta": delta, ":hash": hash },
            )?;
        }
    }
    for (hash, old_count) in old {
        if !new.contains_key(hash) {
            conn.execute(
                sql::ADJUST_REFCOUNT,
                named_params! { ":delta": -old_count, ":hash": hash },
            )?;
        }
    }
    Ok(())
}

fn item_from_row(row: &Row) -> rusqlite::Result<(PimdirLinkId, PimdirHubItem)> {
    let link: String = row.get(0)?;
    let flags: Option<String> = row.get(1)?;
    let object: Option<String> = row.get(2)?;
    let sort_key: String = row.get(3)?;
    let level: i64 = row.get(4)?;
    let deleted: i64 = row.get(5)?;
    let conflicted: i64 = row.get(6)?;
    let conflict_object: Option<String> = row.get(7)?;

    Ok((
        PimdirLinkId(link),
        PimdirHubItem {
            flags: codec::flags_from_json(flags.as_deref()),
            object: object.map(PimdirHash),
            summary: None,
            sort_key: PimdirSortKey(sort_key),
            level: codec::level_from_int(level),
            deleted: deleted != 0,
            conflicted: conflicted != 0,
            conflict_object: conflict_object.map(PimdirHash),
            sources: BTreeMap::new(),
        },
    ))
}

/// Maps a `load_bindings`-shaped row.
pub(crate) fn binding_from_row(
    row: &Row,
) -> rusqlite::Result<(PimdirLinkId, PimdirSourceId, PimdirBinding)> {
    let link: String = row.get(0)?;
    let source: String = row.get(1)?;
    let handle: String = row.get(2)?;
    let base_flags: Option<String> = row.get(3)?;
    let base_object: Option<String> = row.get(4)?;
    let base_revision: Option<String> = row.get(5)?;
    let base_present: i64 = row.get(6)?;
    let conflicted: i64 = row.get(7)?;
    let conflict_revision: Option<String> = row.get(8)?;
    let conflict_object: Option<String> = row.get(9)?;
    let shared_object: Option<String> = row.get(10)?;

    // NOTE: either witness (§13): the column is the fact, and the value
    // columns stay one for a row written before the column existed.
    let base = if base_present != 0
        || base_flags.is_some()
        || base_object.is_some()
        || base_revision.is_some()
    {
        Some(PimdirBase {
            flags: codec::flags_from_json(base_flags.as_deref()),
            revision: base_revision,
            object: base_object.map(PimdirHash),
        })
    } else {
        None
    };

    let conflicted = conflicted != 0;
    Ok((
        PimdirLinkId(link),
        PimdirSourceId(source),
        PimdirBinding {
            handle: PimdirHandle(handle),
            base,
            conflicted,
            conflict_revision: conflicted.then_some(conflict_revision).flatten(),
            conflict_object: conflicted
                .then_some(conflict_object)
                .flatten()
                .map(PimdirHash),
            shared_object: shared_object.map(PimdirHash),
        },
    ))
}
