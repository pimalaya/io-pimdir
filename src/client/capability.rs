//! # Capabilities
//!
//! The declaration rows (STORAGE §15.6) read and written over SQLite: what
//! each source can do where an action lands, the gate an action passes
//! before a producer enqueues it and again before the owner applies it,
//! and the performer of an intent.

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};

use rusqlite::{Connection, OptionalExtension, named_params};

use crate::{
    capability::{
        self, MAIL_SUBMIT, MAIL_SUBMIT_COPY, PimdirCapability, PimdirPartial, PimdirRefusal,
        PimdirSourceCapabilities, PimdirSupport,
    },
    client::{PimdirError, blobs::PimdirBlobs, rows, write},
    codec::{self, PimdirAction},
    object::PimdirHash,
    placement::PimdirFlags,
    sql,
};

/// Whether the store holds the declaration tables: a store from an owner
/// predating §15.6 does not, and every source in it is undeclared.
pub(crate) fn declarable(conn: &Connection) -> Result<bool, PimdirError> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'capabilities'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// What every source syncing `collection` can do there.
pub(crate) fn at_collection(
    conn: &Connection,
    collection: &str,
) -> Result<Vec<PimdirSourceCapabilities>, PimdirError> {
    if !declarable(conn)? {
        return Ok(Vec::new());
    }
    let params = named_params! { ":collection": collection };
    group(rows(conn, sql::LOAD_CAPABILITIES, params, declaration_row)?)
}

/// What every source binding the item `seq` of `collection` can do there.
pub(crate) fn at_item(
    conn: &Connection,
    collection: &str,
    seq: i64,
) -> Result<Vec<PimdirSourceCapabilities>, PimdirError> {
    if !declarable(conn)? {
        return Ok(Vec::new());
    }
    let params = named_params! { ":collection": collection, ":seq": seq };
    group(rows(
        conn,
        sql::LOAD_ITEM_CAPABILITIES,
        params,
        declaration_row,
    )?)
}

/// The sources of `account` declaring an intent capability with some
/// support at `collection`, its own row winning over the source-wide one,
/// or anywhere in the account when `collection` is `None`: the candidates
/// to perform it.
pub(crate) fn candidates(
    conn: &Connection,
    account: Option<&str>,
    collection: Option<&str>,
    capability: &str,
) -> Result<Vec<String>, PimdirError> {
    if !declarable(conn)? {
        return Ok(Vec::new());
    }
    let params = named_params! {
        ":account": account,
        ":collection": collection,
        ":capability": capability,
    };
    Ok(rows(conn, sql::LIST_CAPABILITY_SOURCES, params, |r| {
        r.get(0)
    })?)
}

/// The source the user chose to perform `capability` for `account`.
pub(crate) fn performer(
    conn: &Connection,
    account: Option<&str>,
    capability: &str,
) -> Result<Option<String>, PimdirError> {
    if !declarable(conn)? {
        return Ok(None);
    }
    Ok(conn
        .query_row(
            sql::LOAD_PERFORMER,
            named_params! { ":account": account, ":capability": capability },
            |r| r.get(0),
        )
        .optional()?)
}

/// The user's choice of performer for `capability` and `account`: the
/// latest `set-performer` still queued over the recorded one (§15.4), so
/// a choice holds from the moment it is made rather than from the owner's
/// next run.
pub(crate) fn chosen(
    conn: &Connection,
    account: Option<&str>,
    capability: &str,
) -> Result<Option<String>, PimdirError> {
    let queued = rows(conn, sql::LIST_PENDING_ACTIONS, [], |r| {
        Ok((
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
        ))
    })?;

    let mut pending = None;
    for (collection, kind, payload) in queued {
        let Ok(PimdirAction::SetPerformer {
            capability: queued,
            source,
        }) = codec::action_from_payload(&kind, &payload)
        else {
            continue;
        };
        if queued == capability && account_of(conn, &collection)?.as_deref() == account {
            pending = Some(source);
        }
    }

    match pending {
        Some(source) => Ok(source),
        None => performer(conn, account, capability),
    }
}

/// Resolves the performer of an intent capability anchored on
/// `collection` (§15.6): the source the payload names when it is a
/// candidate there, else the single candidate, else the user's recorded
/// choice while it still is one; several left unchosen are ambiguous.
pub(crate) fn resolve_performer(
    conn: &Connection,
    collection: &str,
    capability: &str,
    named: Option<&str>,
) -> Result<String, PimdirError> {
    let account = account_of(conn, collection)?;
    let candidates = candidates(conn, account.as_deref(), Some(collection), capability)?;

    if let Some(named) = named {
        if candidates.iter().any(|c| c == named) {
            return Ok(named.to_string());
        }
        return Err(PimdirError::Unsupported(PimdirRefusal {
            capability: capability.to_string(),
            source: named.to_string(),
            detail: None,
        }));
    }

    match candidates.as_slice() {
        [] => Err(PimdirError::NoPerformer {
            capability: capability.to_string(),
        }),
        [only] => Ok(only.clone()),
        _ => match chosen(conn, account.as_deref(), capability)? {
            Some(chosen) if candidates.contains(&chosen) => Ok(chosen),
            _ => Err(PimdirError::Ambiguous {
                capability: capability.to_string(),
                candidates,
            }),
        },
    }
}

/// Runs an action through the gate of §15.6 against the sources it
/// concerns, returning the partial supports to show. A refusal is
/// [`PimdirError::Unsupported`]; an intent whose performer cannot be
/// resolved is [`PimdirError::NoPerformer`] or [`PimdirError::Ambiguous`].
/// A store whose sources are all undeclared passes everything.
pub(crate) fn gate(
    conn: &Connection,
    blobs: &PimdirBlobs,
    collection: &str,
    action: &PimdirAction,
) -> Result<Vec<PimdirPartial>, PimdirError> {
    let kind = write::kind_of(conn, collection)?;
    let partials = gate_by_kind(conn, &kind, collection, action)?;
    if kind != "text/calendar" {
        return Ok(partials);
    }

    // NOTE: a calendar write also needs what its resources ask for
    // (Annex B.1), read from the bodies the producer wrote before the
    // enqueue and from the item's current one.
    let new = match action.object_hash() {
        Some(hash) if !matches!(action, PimdirAction::Unknown { .. }) => blobs.get(hash)?,
        _ => None,
    };
    let current = match action {
        PimdirAction::Update { seq, .. } | PimdirAction::Remove { seq } => {
            match item_object(conn, collection, *seq)? {
                Some(hash) => blobs.get(&hash)?,
                None => None,
            }
        }
        _ => None,
    };
    let needed = capability::required_by_content(&kind, action, current.as_deref(), new.as_deref());
    if needed.is_empty() {
        return Ok(partials);
    }

    let sources = match action {
        PimdirAction::Update { seq, .. } | PimdirAction::Remove { seq } => {
            at_item(conn, collection, *seq)?
        }
        _ => at_collection(conn, collection)?,
    };
    let mut partials = partials;
    partials.extend(capability::check(&needed, &sources)?);
    Ok(partials)
}

/// The gate of the capabilities an action needs by its kind and flags
/// alone, before anything is read from its bodies.
fn gate_by_kind(
    conn: &Connection,
    kind: &str,
    collection: &str,
    action: &PimdirAction,
) -> Result<Vec<PimdirPartial>, PimdirError> {
    match action {
        PimdirAction::SetPerformer { .. } => Ok(Vec::new()),
        PimdirAction::Unknown { kind, payload, .. } => {
            let Some(capability) = capability::intent_capability(kind) else {
                return Ok(Vec::new());
            };
            // NOTE: an account whose sources declare nothing predates §15.6,
            // and its owner picks the performer as it always did.
            let account = account_of(conn, collection)?;
            if !account_declared(conn, account.as_deref())? {
                return Ok(Vec::new());
            }
            let named = intent_source(payload);
            let performer = resolve_performer(conn, collection, capability, named.as_deref())?;

            // NOTE: a copy asked for is filed by the same performer, which
            // has to declare it beside the send (Annex B.2).
            if capability == MAIL_SUBMIT && intent_copy(payload).is_some() {
                let able =
                    candidates(conn, account.as_deref(), Some(collection), MAIL_SUBMIT_COPY)?;
                if !able.contains(&performer) {
                    return Err(PimdirError::Unsupported(PimdirRefusal {
                        capability: MAIL_SUBMIT_COPY.to_string(),
                        source: performer,
                        detail: None,
                    }));
                }
            }

            Ok(Vec::new())
        }
        PimdirAction::Add { .. } => {
            let needed = capability::required(kind, action, &PimdirFlags::Unknown);
            Ok(capability::check(
                &needed,
                &at_collection(conn, collection)?,
            )?)
        }
        PimdirAction::SetFlags { seq, .. }
        | PimdirAction::Remove { seq }
        | PimdirAction::Move { seq, .. }
        | PimdirAction::Copy { seq, .. }
        | PimdirAction::Update { seq, .. } => {
            let current = item_flags(conn, collection, *seq)?;
            let holders = at_item(conn, collection, *seq)?;
            let needed = capability::required(kind, action, &current);
            let mut partials = capability::check(&needed, &holders)?;

            if let PimdirAction::Move { to, .. } | PimdirAction::Copy { to, .. } = action {
                let arriving = PimdirAction::Add {
                    link_id: None,
                    flags: current,
                    object: None,
                };
                let needed = capability::required(kind, &arriving, &PimdirFlags::Unknown);
                let others: Vec<_> = at_collection(conn, &to.0)?
                    .into_iter()
                    .filter(|target| holders.iter().all(|h| h.source != target.source))
                    .collect();
                partials.extend(capability::check(&needed, &others)?);
            }

            Ok(partials)
        }
    }
}

/// Replaces `source`'s declaration with `capabilities` (§15.6), under the
/// account it syncs for.
pub(crate) fn declare(
    conn: &Connection,
    account: Option<&str>,
    source: &str,
    capabilities: &[PimdirCapability],
) -> Result<(), PimdirError> {
    conn.execute(
        sql::DELETE_CAPABILITIES,
        named_params! { ":source": source },
    )?;
    for row in capabilities {
        conn.execute(
            sql::SET_CAPABILITY,
            named_params! {
                ":account": account,
                ":source": source,
                ":collection": row.collection,
                ":capability": row.name,
                ":support": row.support.as_str(),
                ":detail": row.detail,
            },
        )?;
    }
    Ok(())
}

/// Applies a `set-performer` (§15.3): the chosen source has to be a
/// candidate for the account of `collection`, else the action parks.
pub(crate) fn set_performer(
    conn: &Connection,
    collection: &str,
    capability: &str,
    source: Option<&str>,
) -> Result<(), PimdirError> {
    let account = account_of(conn, collection)?;
    let params = named_params! { ":account": account, ":capability": capability };
    let Some(source) = source else {
        conn.execute(sql::DELETE_PERFORMER, params)?;
        return Ok(());
    };
    if !candidates(conn, account.as_deref(), None, capability)?
        .iter()
        .any(|c| c == source)
    {
        return Err(PimdirError::Unsupported(PimdirRefusal {
            capability: capability.to_string(),
            source: source.to_string(),
            detail: None,
        }));
    }
    conn.execute(
        sql::SET_PERFORMER,
        named_params! { ":account": account, ":capability": capability, ":source": source },
    )?;
    Ok(())
}

/// The source an intent's payload names, `None` when absent or unreadable.
pub(crate) fn intent_source(payload: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    value.get("source")?.as_str().map(String::from)
}

/// Whether any source of `account` has declared its capabilities.
fn account_declared(conn: &Connection, account: Option<&str>) -> Result<bool, PimdirError> {
    if !declarable(conn)? {
        return Ok(false);
    }
    Ok(conn
        .query_row(
            "SELECT 1 FROM capabilities WHERE account IS :account LIMIT 1",
            named_params! { ":account": account },
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// The collection a `submit` asks its copy filed in, `None` when it asks
/// for none.
fn intent_copy(payload: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    value.get("copy")?.as_str().map(String::from)
}

fn account_of(conn: &Connection, collection: &str) -> Result<Option<String>, PimdirError> {
    Ok(conn
        .query_row(
            sql::LOAD_ACCOUNT,
            named_params! { ":collection": collection },
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten())
}

fn item_object(
    conn: &Connection,
    collection: &str,
    seq: i64,
) -> Result<Option<PimdirHash>, PimdirError> {
    let hash = conn
        .query_row(
            sql::GET_ITEM,
            named_params! { ":collection": collection, ":seq": seq },
            |r| r.get::<_, Option<String>>(3),
        )
        .optional()?
        .flatten();
    Ok(hash.map(PimdirHash))
}

fn item_flags(conn: &Connection, collection: &str, seq: i64) -> Result<PimdirFlags, PimdirError> {
    let flags = conn
        .query_row(
            sql::GET_ITEM,
            named_params! { ":collection": collection, ":seq": seq },
            |r| r.get::<_, Option<String>>(2),
        )
        .optional()?
        .flatten();
    Ok(codec::flags_from_json(flags.as_deref()))
}

type DeclarationRow = (String, Option<String>, Option<String>, Option<String>);

fn declaration_row(r: &rusqlite::Row) -> rusqlite::Result<DeclarationRow> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
}

/// Folds the statements' rows, one per source and capability, an
/// undeclared source once with a `NULL` capability, into one entry per
/// source.
fn group(rows: Vec<DeclarationRow>) -> Result<Vec<PimdirSourceCapabilities>, PimdirError> {
    let mut sources: Vec<PimdirSourceCapabilities> = Vec::new();
    for (source, capability, support, detail) in rows {
        if sources.last().is_none_or(|last| last.source != source) {
            sources.push(PimdirSourceCapabilities {
                source: source.clone(),
                declared: None,
            });
        }
        let entry = sources.last_mut().expect("pushed above");
        if let (Some(capability), Some(support)) = (capability, support) {
            entry
                .declared
                .get_or_insert_with(BTreeMap::new)
                .insert(capability, (PimdirSupport::parse(&support), detail));
        }
    }
    Ok(sources)
}
