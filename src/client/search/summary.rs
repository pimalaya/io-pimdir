//! # Summary reads
//!
//! An item's summary and address rows as the index reads them (SEARCH §6).

use alloc::{string::String, vec::Vec};

use rusqlite::{Connection, OptionalExtension, named_params};

use crate::{
    client::{PimdirError, bind_list, rows, write::tables_of},
    sql,
    summary::{PimdirAddress, PimdirAddressRole, PimdirSummary},
};

/// An item's address rows, by role.
pub(crate) type Addresses = Vec<(PimdirAddressRole, PimdirAddress)>;

/// One live item's summary as the fold reads it (SEARCH §6): its summary
/// row by key, a seek on each statement, its lists, and its address rows.
pub(crate) fn summary(
    conn: &Connection,
    collection: &str,
    seq: i64,
    kind: &str,
) -> Result<(Option<PimdirSummary>, Addresses), PimdirError> {
    let key = named_params! { ":collection": collection, ":seq": seq };
    let mut link_id = None;
    let mut found = None;
    for table in tables_of(kind) {
        let item = conn
            .prepare_cached(table.get_sql())?
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
        return Ok((None, Vec::new()));
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

    Ok((found, addresses))
}
