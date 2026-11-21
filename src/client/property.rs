//! An item's properties in the store (STORAGE §4.3): the four reads of
//! §14.1 on the reader, a reader's overlay of the pending `set-property`
//! rows (§15.4), the owner's own writes, and the drain's apply (§15.3).
//! Every write goes through [`property::change`], so the owner, the drain
//! and the overlay agree on what a change leaves.

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use rusqlite::{Connection, OptionalExtension, named_params};

use crate::{
    client::{
        PimdirError, PimdirStore, PreparedCached, begin, busy_or_sql, producer::overlaid_actions,
        reader::PimdirReader, rows,
    },
    codec::PimdirAction,
    property::{self, PimdirProperty, PimdirPropertyChange, PimdirPropertyHolder},
    sql,
};

impl PimdirReader {
    /// An item's properties (`list_item_properties`, §4.3, §14.1): every
    /// key and value its public id holds, in key then value order, byte for
    /// byte, whatever the state of the rows holding the seq. The committed
    /// rows alone: [`item_properties`](Self::item_properties) folds the
    /// queue over them.
    pub fn list_item_properties(&self, seq: i64) -> Result<Vec<PimdirProperty>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::LIST_ITEM_PROPERTIES,
            named_params! { ":seq": seq },
            |r| {
                Ok(PimdirProperty {
                    key: r.get(0)?,
                    value: r.get(1)?,
                })
            },
        )?)
    }

    /// The item `seq` of `collection`'s properties as a read of that
    /// collection shows them: [`list_item_properties`]'s, with the
    /// `set-property` rows pending on `collection` folded in as the owner
    /// would apply each when the reader overlays the queue
    /// ([`with_pending`](Self::with_pending), §15.4). A row the owner would
    /// park folds into nothing: one naming a `seq` no live row of
    /// `collection` holds, or one leaving more than 64 values under its key.
    /// A row pending on another collection holding the seq shows once
    /// applied.
    ///
    /// [`list_item_properties`]: Self::list_item_properties
    pub fn item_properties(
        &self,
        collection: impl AsRef<str>,
        seq: i64,
    ) -> Result<Vec<PimdirProperty>, PimdirError> {
        let collection = collection.as_ref();
        let mut properties = self.list_item_properties(seq)?;
        if !self.overlays_pending() || !live(&self.conn, collection, seq)? {
            return Ok(properties);
        }

        for pending in overlaid_actions(&self.conn)? {
            let PimdirAction::SetProperty {
                seq: named,
                key,
                add,
                remove,
            } = &pending.action
            else {
                continue;
            };
            if *named != seq || pending.collection != collection {
                continue;
            }
            let held: Vec<String> = properties
                .iter()
                .filter(|property| property.key == *key)
                .map(|property| property.value.clone())
                .collect();
            let Ok(change) = property::change(key, &held, remove, add) else {
                continue;
            };
            properties.retain(|property| {
                property.key != *key || !change.removed.contains(&property.value)
            });
            properties.extend(change.added.into_iter().map(|value| PimdirProperty {
                key: key.clone(),
                value,
            }));
        }

        properties.sort();
        Ok(properties)
    }

    /// The values one key holds across the store (`list_property_values`,
    /// §14.1), each once, in byte order: the labels a picker offers.
    pub fn list_property_values(&self, key: &str) -> Result<Vec<String>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::LIST_PROPERTY_VALUES,
            named_params! { ":key": key },
            |r| r.get(0),
        )?)
    }

    /// The live placements whose item holds `key`, whatever its values
    /// (`list_property_holders`, §14.1), each once, by collection and
    /// public id.
    pub fn list_property_holders(
        &self,
        key: &str,
    ) -> Result<Vec<PimdirPropertyHolder>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::LIST_PROPERTY_HOLDERS,
            named_params! { ":key": key },
            holder_from_row,
        )?)
    }

    /// The live placements whose item holds `value` under `key`, compared
    /// byte for byte (`list_value_holders`, §14.1), each once, by
    /// collection and public id. A label matched folded reads
    /// [`list_property_values`](Self::list_property_values) first, then
    /// this per spelling whose [`fold`](property::fold) matches.
    pub fn list_value_holders(
        &self,
        key: &str,
        value: &str,
    ) -> Result<Vec<PimdirPropertyHolder>, PimdirError> {
        Ok(rows(
            &self.conn,
            sql::LIST_VALUE_HOLDERS,
            named_params! { ":key": key, ":value": value },
            holder_from_row,
        )?)
    }
}

impl PimdirStore {
    /// Adds `value` under `key` to the properties of the item `seq` names
    /// (`add_property`, §4.3), on the terms a `set-property` is applied on
    /// (§15.3): a value held already, or under [`property::LABEL`] one
    /// whose fold an item's label has, changes nothing, which is answered
    /// `false`. A value added stamps every row holding the seq (§4.5).
    ///
    /// A key or a value the store cannot hold, a key under `pimdir/` the
    /// standard does not define, or a 65th value under the key is
    /// [`PimdirError::Property`]; a seq no row of the store holds is
    /// refused by the schema.
    pub fn add_property(&mut self, seq: i64, key: &str, value: &str) -> Result<bool, PimdirError> {
        self.write_property(seq, key, &[], &[value.to_string()])
    }

    /// Removes `value` from under `key` in the properties of the item
    /// `seq` names (`remove_property`, §4.3), on the terms of
    /// [`add_property`](Self::add_property): under [`property::LABEL`] the
    /// label held whose fold is the value's goes; a value not held changes
    /// nothing, which is answered `false`.
    pub fn remove_property(
        &mut self,
        seq: i64,
        key: &str,
        value: &str,
    ) -> Result<bool, PimdirError> {
        self.write_property(seq, key, &[value.to_string()], &[])
    }

    /// One change of the owner's own (§4.3), in a transaction of its own.
    fn write_property(
        &mut self,
        seq: i64,
        key: &str,
        remove: &[String],
        add: &[String],
    ) -> Result<bool, PimdirError> {
        if !property::is_writable_key(key) {
            return Err(PimdirError::Property(format!(
                "key {key:?} is neither pimdir/label nor <namespace>/<name> outside pimdir/ of a to z, 0 to 9, '.', '_' and '-', at most 64 bytes"
            )));
        }
        if let Some(value) = remove.iter().chain(add).find(|v| !property::is_value(v)) {
            return Err(PimdirError::Property(format!(
                "value {value:?} is not 1 to 1024 bytes of UTF-8 without NUL"
            )));
        }

        let tx = begin(&mut self.reader.conn, &self.lock)?;
        let change = match write(&tx, seq, key, remove, add)? {
            Ok(change) => change,
            Err(reason) => return Err(PimdirError::Property(reason)),
        };
        tx.commit().map_err(busy_or_sql)?;
        Ok(!change.is_empty())
    }
}

/// Applies a `set-property` queued on `collection` in the drain's
/// transaction (§15.3): parked, as the `Err` reason, when no live row of
/// `collection` holds `seq` or when it would leave more than 64 values
/// under its key; else its rows written, removes then adds.
pub(crate) fn apply(
    tx: &Connection,
    collection: &str,
    seq: i64,
    key: &str,
    remove: &[String],
    add: &[String],
) -> Result<Result<(), String>, PimdirError> {
    if !live(tx, collection, seq)? {
        return Ok(Err(format!("no live item of {collection} holds seq {seq}")));
    }
    Ok(write(tx, seq, key, remove, add)?.map(|_| ()))
}

/// Writes the rows a change leaves under `key` (§15.3), read against the
/// values the item holds there, or the reason it cannot: more than 64
/// values left under the key.
fn write(
    tx: &Connection,
    seq: i64,
    key: &str,
    remove: &[String],
    add: &[String],
) -> Result<Result<PimdirPropertyChange, String>, PimdirError> {
    let held: Vec<String> = rows(
        tx,
        sql::LIST_ITEM_PROPERTIES,
        named_params! { ":seq": seq },
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )?
    .into_iter()
    .filter(|(held, _)| held == key)
    .map(|(_, value)| value)
    .collect();
    let change = match property::change(key, &held, remove, add) {
        Ok(change) => change,
        Err(overflow) => {
            return Ok(Err(format!(
                "item {seq} would hold {} values under {key}, past the 64 a key holds",
                overflow.values
            )));
        }
    };

    for value in &change.removed {
        tx.execute_cached(
            sql::REMOVE_PROPERTY,
            named_params! { ":seq": seq, ":key": key, ":value": value },
        )
        .map_err(busy_or_sql)?;
    }
    for value in &change.added {
        tx.execute_cached(
            sql::ADD_PROPERTY,
            named_params! { ":seq": seq, ":key": key, ":value": value },
        )
        .map_err(busy_or_sql)?;
    }
    Ok(Ok(change))
}

/// Whether a live row of `collection` holds `seq` (`get_item`).
fn live(conn: &Connection, collection: &str, seq: i64) -> Result<bool, PimdirError> {
    Ok(conn
        .query_row_cached(
            sql::GET_ITEM,
            named_params! { ":collection": collection, ":seq": seq },
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .is_some())
}

/// Maps a `(collection, seq)` row of the two holder reads.
fn holder_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<PimdirPropertyHolder> {
    Ok(PimdirPropertyHolder {
        collection: r.get(0)?,
        seq: r.get(1)?,
    })
}
