//! # Summary reads
//!
//! An item's summary and address rows as the index reads them (SEARCH §6).

use alloc::{string::String, vec::Vec};

use rusqlite::{Connection, OptionalExtension, named_params};

use crate::{
    client::{
        PimdirError, rows,
        write::{PimdirSummaryValue, attach_value, tables_of},
    },
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
    let read = rows(conn, sql::LOAD_VALUES, item, |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut addresses = Vec::new();
    for (role, value, name) in read {
        match (PimdirSummaryValue::of(&role, value, name), &mut found) {
            (Some(PimdirSummaryValue::Address(role, address)), _) => {
                addresses.push((role, address));
            }
            (Some(value), Some(summary)) => attach_value(summary, value),
            _ => {}
        }
    }

    Ok((found, addresses))
}
