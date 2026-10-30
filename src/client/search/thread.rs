//! # Threads
//!
//! Mail threads in the index (SEARCH §9): one `message` row per message
//! id a held message bears or names as its parent, present in the store
//! or not, its `parent` the first `In-Reply-To` id, else the last
//! `References` id, read from the body the index holds, else from the
//! summary's `in_reply_to`. A thread is a connected set over those
//! rows, named after its present member with the lowest
//! `(sort_key, link_id)`.
//!
//! A refresh collects the message ids its batch touched (a mail placement
//! new, rebodied, rekeyed or dropped) and [`relink`]s them before the
//! batch commits: their parents re-read, the edge closing a cycle
//! ignored, and every thread they reach recomputed, a join writing the
//! new row and repointing the members before the old row goes. No
//! joining by subject.

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::{String, ToString},
    vec,
    vec::Vec,
};

use log::{debug, trace};
use mail_parser::MessageParser;
use rusqlite::{OptionalExtension, Transaction, named_params};

use crate::{
    client::{PimdirError, blobs::PimdirBlobs, rows, schema::StoreShape},
    object::PimdirHash,
    sql::search,
};

/// Rereads the parents of the message ids a batch touched and recomputes
/// every thread they reach (SEARCH §9), in the batch's transaction.
pub(crate) fn relink(
    tx: &Transaction,
    blobs: &PimdirBlobs,
    shape: StoreShape,
    touched: &BTreeSet<String>,
) -> Result<(), PimdirError> {
    if touched.is_empty() {
        return Ok(());
    }
    debug!("relinking {} message(s)", touched.len());

    let mut seeds = BTreeSet::new();
    for link_id in touched {
        let held = message(tx, link_id)?;
        let present = presence(tx, shape, link_id)?.is_some();
        if held.is_none() && !present {
            continue;
        }

        let parent = match present {
            true => parent(tx, blobs, shape, link_id)?,
            false => None,
        };
        // NOTE: the edge that would close a cycle is ignored, a message
        // naming itself included.
        let parent = match parent {
            Some(parent) if reaches(tx, &parent, link_id)? => {
                trace!("ignoring {link_id} -> {parent}, which closes a cycle");
                None
            }
            parent => parent,
        };
        trace!("message {link_id} replies to {parent:?}");

        match held {
            None => {
                provisional(tx, link_id)?;
                write(tx, link_id, link_id, parent.as_deref())?;
            }
            Some((thread, old)) if old != parent => {
                write(tx, link_id, &thread, parent.as_deref())?;
                seeds.extend(old);
            }
            Some(_) => (),
        }
        if let Some(parent) = &parent
            && message(tx, parent)?.is_none()
        {
            provisional(tx, parent)?;
            write(tx, parent, parent, None)?;
        }
        seeds.insert(link_id.clone());
    }

    rethread(tx, shape, &seeds)
}

/// Recomputes the thread of every component a seed is in: a component
/// no placement holds a member of dropped whole (a message row nobody
/// holds has no parent, so one nobody names is alone), else every member
/// pointed to one thread row named after the present member with the
/// lowest `(sort_key, link_id)`; last the thread rows no member points
/// to any more.
fn rethread(
    tx: &Transaction,
    shape: StoreShape,
    seeds: &BTreeSet<String>,
) -> Result<(), PimdirError> {
    let mut done = BTreeSet::new();
    let mut stale = BTreeSet::new();

    for seed in seeds {
        if done.contains(seed) {
            continue;
        }

        let mut members: BTreeMap<String, (String, Option<String>)> = BTreeMap::new();
        let mut queue = vec![seed.clone()];
        while let Some(link_id) = queue.pop() {
            if members.contains_key(&link_id) {
                continue;
            }
            let Some((thread, parent)) = message(tx, &link_id)? else {
                continue;
            };
            queue.extend(parent.clone());
            queue.extend(rows(
                tx,
                search::MESSAGE_CHILDREN,
                named_params! { ":link_id": link_id },
                |r| r.get::<_, String>(0),
            )?);
            members.insert(link_id, (thread, parent));
        }
        done.extend(members.keys().cloned());
        stale.extend(members.values().map(|(thread, _)| thread.clone()));

        let mut present = Vec::new();
        for link_id in members.keys() {
            if let Some(presence) = presence(tx, shape, link_id)? {
                present.push((link_id.clone(), presence));
            }
        }

        let Some((id, _)) = present
            .iter()
            .min_by(|(a, x), (b, y)| (&x.sort_key, a).cmp(&(&y.sort_key, b)))
        else {
            trace!("thread of {seed} holds no message, dropping it");
            for link_id in members.keys() {
                tx.execute(
                    search::DELETE_MESSAGE,
                    named_params! { ":link_id": link_id },
                )?;
            }
            continue;
        };
        let first = present
            .iter()
            .filter_map(|(_, presence)| presence.first.clone())
            .min();
        let last = present
            .iter()
            .filter_map(|(_, presence)| presence.last.clone())
            .max();
        trace!("thread {id} holds {} message(s)", present.len());

        tx.execute(
            search::UPSERT_THREAD,
            named_params! {
                ":id": id,
                ":first": first.unwrap_or_default(),
                ":last": last.unwrap_or_default(),
                ":count": present.len() as i64,
            },
        )?;
        for (link_id, (thread, parent)) in &members {
            if thread != id {
                write(tx, link_id, id, parent.as_deref())?;
            }
        }
    }

    for id in stale {
        tx.execute(search::DELETE_THREAD_UNUSED, named_params! { ":id": id })?;
    }
    Ok(())
}

/// What the index holds of one message id: its mail placements' lowest
/// sort key, and their dated span.
struct Presence {
    sort_key: String,
    first: Option<String>,
    last: Option<String>,
}

/// The message row of an id, `(thread, parent)`, `None` without one.
fn message(
    tx: &Transaction,
    link_id: &str,
) -> Result<Option<(String, Option<String>)>, PimdirError> {
    Ok(tx
        .query_row(
            search::LOAD_MESSAGE,
            named_params! { ":link_id": link_id },
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

/// The mail placements the index holds for an id, `None` when it holds
/// none: a message the store does not hold.
fn presence(
    tx: &Transaction,
    shape: StoreShape,
    link_id: &str,
) -> Result<Option<Presence>, PimdirError> {
    let (count, sort_key, first, last): (i64, Option<String>, Option<String>, Option<String>) = tx
        .query_row(
            &shape.sql(tx, search::MESSAGE_PRESENCE)?,
            named_params! { ":link_id": link_id },
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;

    Ok((count > 0).then(|| Presence {
        sort_key: sort_key.unwrap_or_default(),
        first,
        last,
    }))
}

/// The parent of a held message (SEARCH §9): from a body the index holds,
/// its first `In-Reply-To` id, else its last `References` id; from its
/// summary's `in_reply_to` when it holds none, or none it can read.
fn parent(
    tx: &Transaction,
    blobs: &PimdirBlobs,
    shape: StoreShape,
    link_id: &str,
) -> Result<Option<String>, PimdirError> {
    let hash: Option<String> = tx
        .query_row(
            search::MESSAGE_HASH,
            named_params! { ":link_id": link_id },
            |r| r.get(0),
        )
        .optional()?;
    let body = match hash {
        Some(hash) => blobs.get(&PimdirHash(hash))?,
        None => None,
    };
    if let Some(message) = body
        .as_deref()
        .and_then(|body| MessageParser::default().parse_headers(body))
    {
        let first = message
            .in_reply_to()
            .as_text_list()
            .and_then(|ids| ids.first())
            .or_else(|| message.references().as_text_list()?.last());
        return Ok(first.map(|id| id.to_string()).filter(|id| !id.is_empty()));
    }

    let in_reply_to: Option<String> = tx
        .query_row(
            &shape.sql(tx, search::MESSAGE_IN_REPLY_TO)?,
            named_params! { ":link_id": link_id },
            |r| r.get(0),
        )
        .optional()?;
    let ids: Vec<String> = in_reply_to
        .and_then(|ids| serde_json::from_str(&ids).ok())
        .unwrap_or_default();
    Ok(ids.into_iter().next())
}

/// Whether following parents up from `from` reaches `to`, `from` itself
/// included: the edge `to -> from` would close a cycle.
fn reaches(tx: &Transaction, from: &str, to: &str) -> Result<bool, PimdirError> {
    let mut seen = BTreeSet::new();
    let mut at = Some(from.to_string());
    while let Some(link_id) = at {
        if link_id == to {
            return Ok(true);
        }
        if !seen.insert(link_id.clone()) {
            return Ok(false);
        }
        at = message(tx, &link_id)?.and_then(|(_, parent)| parent);
    }
    Ok(false)
}

/// A thread row named after a message entering the index, so its message
/// row has one to point to until [`rethread`] names the real one.
fn provisional(tx: &Transaction, link_id: &str) -> Result<(), PimdirError> {
    tx.execute(
        search::UPSERT_THREAD,
        named_params! { ":id": link_id, ":first": "", ":last": "", ":count": 0 },
    )?;
    Ok(())
}

/// Writes one message row.
fn write(
    tx: &Transaction,
    link_id: &str,
    thread: &str,
    parent: Option<&str>,
) -> Result<(), PimdirError> {
    tx.execute(
        search::UPSERT_MESSAGE,
        named_params! { ":link_id": link_id, ":thread": thread, ":parent": parent },
    )?;
    Ok(())
}
