//! # A body entering the store, or linked from it
//!
//! A message's attachments are files holding no body, one per part
//! carrying `Content-Disposition: attachment`, each referenced by the
//! message (STORAGE §14.3). They are recorded at the instant the
//! message's body enters the store, in the transaction writing it,
//! whatever wrote it: a sync landing a fetched body, an upgrade, a
//! queued add, a direct write. A body the store already held under the
//! same key in the account (a move, a copy, a second label) enters
//! nothing, so neither does a body held before the rule ran: nothing is
//! walked twice, nothing is backfilled, and a stand-in once recorded is
//! never recomputed.
//!
//! The same walk gives the columns of the row that only the body gives
//! (Annex A.1), the attachment mark and the invitation, which the write
//! states over whatever the writer read without the body: a writer
//! streaming the body never holds it whole to derive them. A placement
//! gaining a body the account already holds takes them from a row
//! holding it (`held_mail_summary`), its size the body's, never the
//! guess its listing made. A held body no such row states a mark of
//! (rows written before marks were read off bodies) is walked for those
//! two columns alone: its parts stand for nothing, its rows are left as
//! they are.

use alloc::{format, string::String, vec::Vec};

use std::{collections::BTreeMap, io::Read};

use rusqlite::{Connection, OptionalExtension, named_params};

use crate::{
    client::{PimdirError, blobs::PimdirBlobs, write::kind_of},
    hub::PimdirHub,
    object::PimdirHash,
    placement::PimdirLinkId,
    sql,
    summary::{
        PimdirSummary,
        file::{self, PimdirFileSummary},
        mail::attachments::{PimdirAttachmentWalk, PimdirParts},
    },
};

/// The kind whose bodies stand for attachments.
const MAIL: &str = "message/rfc822";

/// The role of an account's attachments collection (STORAGE §14).
pub(crate) const ROLE: &str = "attachments";

/// The name an account's attachments collection is created under, after
/// the account and a `U+0001`, which no server puts in a mailbox name:
/// `<account>/\u{1}attachments`, or `\u{1}attachments` in a store of one
/// account. The Android app files them alike.
pub(crate) const NAME: &str = "attachments";

/// How much of a body one read takes.
const CHUNK: usize = 64 * 1024;

/// The stand-ins the bodies entering one mail collection stand for,
/// walked before its diff is saved and recorded after it.
pub(crate) struct Entering {
    collection: String,
    account: Option<String>,
    bodies: Vec<(PimdirLinkId, Vec<PimdirFileSummary>)>,
}

/// Reads the bodies the diff of `collection` from `old` to `new` gives a
/// live, stored item under a key naming an identity, and states in
/// `new` the columns of their rows only a body gives (Annex A.1).
///
/// A body no row of its key in the account held before the batch
/// enters: it is walked, its mark and invitation stated, and its parts
/// answered for [`record`] unless its key is writer-derived, which names
/// no identity (§14.3). A body a row of the account already holds is
/// linked: its size, mark and invitation are that row's, a body no such
/// row states a mark of walked for those two alone. `None` for a
/// collection of another kind, or when nothing enters.
pub(crate) fn gaining(
    conn: &Connection,
    blobs: &PimdirBlobs,
    collection: &str,
    old: &PimdirHub,
    new: &mut PimdirHub,
) -> Result<Option<Entering>, PimdirError> {
    let bodies: Vec<(PimdirLinkId, PimdirHash)> = new
        .items
        .iter()
        .filter_map(|(link, item)| {
            let prev = old.items.get(link);
            let object = item.object.as_ref()?;
            let stored = prev.is_some() || !item.sources.is_empty();
            let moved = prev.and_then(|prev| prev.object.as_ref()) != Some(object);
            (stored && moved && !item.deleted).then(|| (link.clone(), object.clone()))
        })
        .collect();
    if bodies.is_empty() || kind_of(conn, collection)? != MAIL {
        return Ok(None);
    }

    let account: Option<String> = conn
        .query_row(
            sql::LOAD_ACCOUNT,
            named_params! { ":collection": collection },
            |row| row.get(0),
        )
        .optional()?
        .flatten();

    // NOTE: lookup_objects answers the bodies a link id already has in
    // the account, which is what a move, a copy or a second label carry;
    // a writer-derived key never matches, so its body is always read.
    let links: Vec<&str> = bodies.iter().map(|(link, _)| link.as_str()).collect();
    let mut stmt = conn.prepare(sql::LOOKUP_OBJECTS)?;
    let held: BTreeMap<(String, String), i64> = stmt
        .query_map(
            named_params! {
                ":links": serde_json::to_string(&links)?,
                ":account": account.as_deref(),
            },
            |row| {
                Ok((
                    (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                    row.get::<_, i64>(2)?,
                ))
            },
        )?
        .collect::<Result<_, _>>()?;

    let mut entering = Vec::new();
    for (link, hash) in bodies {
        let found = match held.get(&(link.0.clone(), hash.0.clone())) {
            Some(size) => {
                let (mark, invitation) = held_columns(conn, &link, &hash, account.as_deref())?;
                Body::Held(
                    match mark {
                        Some(_) => (mark, invitation),
                        None => walked_columns(blobs, &link, &hash, collection, invitation)?,
                    },
                    *size,
                )
            }
            None => match walk(blobs, &hash)? {
                Some(parts) => Body::Read(parts),
                None => {
                    log::warn!(
                        "body {} of {} in {collection} is not in the blob store, \
                         nothing read off it",
                        hash.0,
                        link.0,
                    );
                    continue;
                }
            },
        };

        let summary = new
            .items
            .get_mut(&link)
            .and_then(|item| item.summary.as_mut());
        if let Some(PimdirSummary::Mail(mail)) = summary {
            match &found {
                Body::Held((attachment, invitation), size) => {
                    mail.size = Some((*size).max(0) as u64);
                    if attachment.is_some() {
                        mail.attachment = *attachment;
                    }
                    if invitation.is_some() {
                        mail.invitation = invitation.clone();
                    }
                }
                Body::Read(parts) => {
                    mail.attachment = Some(parts.attachment);
                    if parts.invitation.is_some() {
                        mail.invitation = parts.invitation.clone();
                    }
                }
            }
        }

        if let Body::Read(parts) = found
            && !parts.attachments.is_empty()
            && !link.is_derived()
        {
            entering.push((link, parts.attachments));
        }
    }

    Ok((!entering.is_empty()).then(|| Entering {
        collection: collection.into(),
        account,
        bodies: entering,
    }))
}

/// What a body gaining a row gives it.
enum Body {
    /// Held by a row of the account: that row's mark and invitation, a
    /// walk's when it states no mark, and the body's size.
    Held((Option<bool>, Option<String>), i64),
    /// Read now, entering the store.
    Read(PimdirParts),
}

/// The mark and invitation a row of the account holding `hash` under
/// `link` states (`held_mail_summary`), each `None` when none does.
fn held_columns(
    conn: &Connection,
    link: &PimdirLinkId,
    hash: &PimdirHash,
    account: Option<&str>,
) -> rusqlite::Result<(Option<bool>, Option<String>)> {
    Ok(conn
        .query_row(
            sql::HELD_MAIL_SUMMARY,
            named_params! {
                ":link_id": link.as_str(),
                ":object": hash.0,
                ":account": account,
            },
            |row| {
                Ok((
                    row.get::<_, Option<i64>>(0)?.map(|mark| mark != 0),
                    row.get::<_, Option<String>>(1)?,
                ))
            },
        )
        .optional()?
        .unwrap_or_default())
}

/// The mark and invitation a walk of a held body gives, for a body no
/// row holding it states a mark of (rows written before marks were read
/// off bodies): those two columns only, its parts standing for nothing,
/// since a body held before is never walked for stand-ins (§14.3), and
/// the held rows left as they are. `(None, held)` when the blob store
/// lacks the body, the writer's mark then kept.
fn walked_columns(
    blobs: &PimdirBlobs,
    link: &PimdirLinkId,
    hash: &PimdirHash,
    collection: &str,
    held: Option<String>,
) -> Result<(Option<bool>, Option<String>), PimdirError> {
    match walk(blobs, hash)? {
        Some(parts) => Ok((Some(parts.attachment), parts.invitation.or(held))),
        None => {
            log::warn!(
                "body {} of {} in {collection} is held by rows stating no mark \
                 and is not in the blob store, the writer's mark kept",
                hash.0,
                link.0,
            );
            Ok((None, held))
        }
    }
}

/// Records the stand-ins of the bodies entering, once the diff that
/// stores them is saved: per part a file at `Meta` in the account's
/// attachments collection, created on demand, and the `attachment`
/// reference from the message. A part the collection holds already
/// keeps its stand-in.
pub(crate) fn record(conn: &Connection, entering: Entering) -> Result<(), PimdirError> {
    let Some(collection) = collection_of(conn, entering.account.as_deref())? else {
        log::warn!(
            "the attachments collection of {} is taken by another kind, \
             no attachment recorded",
            entering.collection,
        );
        return Ok(());
    };

    for (message, parts) in entering.bodies {
        for summary in parts {
            let Some(section) = summary.part.as_deref() else {
                continue;
            };
            let Some(key) = file::part_key(&message, section) else {
                continue;
            };
            let held: Option<i64> = conn
                .query_row(
                    sql::SEQ_BY_LINK,
                    named_params! { ":collection": collection, ":link_id": key.0 },
                    |row| row.get(0),
                )
                .optional()?;
            if held.is_none() {
                insert_file(conn, &collection, &key, &summary)?;
            }
            conn.query_row(
                sql::ADD_REFERENCE,
                named_params! {
                    ":from_kind": MAIL,
                    ":from_link_id": message.0,
                    ":to_kind": file::KIND,
                    ":to_link_id": key.0,
                    ":role": "attachment",
                    ":origin": "auto",
                },
                |_| Ok(()),
            )
            .optional()?;
        }
    }

    Ok(())
}

/// What the walk reads off a stored body, read a chunk at a time and no
/// further than the walk needs; `None` when the blob store lacks it.
fn walk(blobs: &PimdirBlobs, hash: &PimdirHash) -> Result<Option<PimdirParts>, PimdirError> {
    let Some(mut file) = blobs.reader(hash)? else {
        return Ok(None);
    };
    let mut walk = PimdirAttachmentWalk::new();
    let mut buffer = alloc::vec![0; CHUNK];
    while !walk.done() {
        let read = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            // NOTE: read to its end, the body failed its name: no body,
            // never parsed (§7).
            Err(err) if err.kind() == std::io::ErrorKind::InvalidData => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        walk.feed(&buffer[..read]);
    }
    Ok(Some(walk.finish_parts()))
}

/// The account's attachments collection: the file collection holding the
/// role, else the one under [`NAME`], declared and given the role here.
/// `None` when that id is taken by a collection of another kind.
fn collection_of(conn: &Connection, account: Option<&str>) -> Result<Option<String>, PimdirError> {
    let mut stmt = conn.prepare(sql::LIST_COLLECTIONS_BY_ACCOUNT)?;
    let held: Option<String> = stmt
        .query_map(named_params! { ":account": account }, |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(9)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .find(|(_, kind, role)| kind == file::KIND && role.as_deref() == Some(ROLE))
        .map(|(id, _, _)| id);
    if held.is_some() {
        return Ok(held);
    }

    let id = match account {
        Some(account) => format!("{account}/\u{1}{NAME}"),
        None => format!("\u{1}{NAME}"),
    };
    let declared: Option<String> = conn
        .query_row(sql::LOAD_KIND, named_params! { ":collection": id }, |row| {
            row.get(0)
        })
        .optional()?;
    if declared.is_some_and(|kind| !kind.is_empty() && kind != file::KIND) {
        return Ok(None);
    }
    conn.execute(
        sql::SET_COLLECTION_KIND,
        named_params! { ":collection": id, ":account": account, ":kind": file::KIND },
    )?;
    conn.execute(
        sql::SET_COLLECTION_NAME,
        named_params! { ":collection": id, ":account": account, ":name": NAME },
    )?;
    conn.execute(
        sql::SET_COLLECTION_ROLE,
        named_params! { ":collection": id, ":role": ROLE },
    )?;
    Ok(Some(id))
}

/// Writes a file holding no body at `Meta` with its summary, sharing the
/// public id its key has elsewhere (§9.1), answering it.
pub(crate) fn insert_file(
    conn: &Connection,
    collection: &str,
    link: &PimdirLinkId,
    summary: &PimdirFileSummary,
) -> rusqlite::Result<i64> {
    let shared: Option<i64> = conn
        .query_row(
            sql::SEQ_FOR_LINK_ANY,
            named_params! { ":link_id": link.as_str() },
            |row| row.get(0),
        )
        .optional()?;
    let seq = match shared {
        Some(seq) => seq,
        None => conn.query_row(sql::BUMP_NEXT_SEQ, [], |row| row.get(0))?,
    };
    conn.execute(
        sql::INSERT_ITEM,
        named_params! {
            ":collection": collection,
            ":link_id": link.as_str(),
            ":seq": seq,
            ":flags": "[]",
            ":object_hash": None::<String>,
            ":sort_key": summary.sort_key().0,
            ":level": 1,
            ":deleted": 0,
            ":conflicted": 0,
            ":conflict_object": None::<String>,
        },
    )?;
    upsert_file_summary(conn, collection, link, summary)?;
    Ok(seq)
}

/// Writes one placement's `file_summary` row.
pub(crate) fn upsert_file_summary(
    conn: &Connection,
    collection: &str,
    link: &PimdirLinkId,
    summary: &PimdirFileSummary,
) -> rusqlite::Result<()> {
    conn.execute(
        sql::UPSERT_FILE_SUMMARY,
        named_params! {
            ":collection": collection,
            ":link_id": link.as_str(),
            ":name": summary.name,
            ":media_type": summary.media_type,
            ":size": summary.size.map(|size| size as i64),
            ":part": summary.part,
        },
    )?;
    Ok(())
}
