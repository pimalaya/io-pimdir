//! The `property` verb group: list, add and remove an item's properties,
//! the local state no source carries (pimdir STORAGE.md §4.3), a local
//! label (`pimdir/label`) among them.
//!
//! A change is staged as a `set-property` action through the store's queue,
//! as `tag` stages its flags, then applied when no sync holds the store.

use std::fmt;

use anyhow::Result;
use clap::{Args, Subcommand};
use io_pimdir::{
    client::{PimdirError, producer::PimdirActionStatus, reader::PimdirReader},
    codec::PimdirAction,
};
use pimalaya_cli::{printer::Printer, sanitize};
use schemars::JsonSchema;
use serde::Serialize;

use crate::cli::{
    StoreFlags,
    error::{ErrorCode, partial, report},
};

/// Read and change an item's properties, its local state.
///
/// A property is one value under one key (`<namespace>/<name>`), held by
/// the item's public id in every collection filing it and never synced.
/// `pimdir/label` holds the item's local labels, compared ignoring case;
/// an application keeps its own state under its own namespace
/// (`himalaya/snooze-until`).
#[derive(Debug, Subcommand)]
pub enum PropertyCommand {
    /// List an item's properties.
    List(PropertyListCommand),
    /// Add values under a key of an item.
    Add(PropertyChangeCommand),
    /// Remove values from under a key of an item.
    Remove(PropertyChangeCommand),
}

impl PropertyCommand {
    /// Runs the selected subcommand.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        match self {
            Self::List(cmd) => cmd.execute(printer, store),
            Self::Add(cmd) => cmd.execute(printer, store, true),
            Self::Remove(cmd) => cmd.execute(printer, store, false),
        }
    }
}

/// List the properties an item holds, every key and value in key then
/// value order, as the store holds them.
#[derive(Debug, Args)]
pub struct PropertyListCommand {
    /// Public id of the item (`seq`).
    #[arg(value_name = "SEQ")]
    pub seq: i64,
}

impl PropertyListCommand {
    /// Prints the item's properties.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let properties = store
            .read()?
            .list_item_properties(self.seq)
            .map_err(report)?
            .into_iter()
            .map(|property| PropertyRow {
                key: property.key,
                value: property.value,
            })
            .collect();

        printer.out(PropertiesOutput {
            seq: self.seq,
            properties,
        })
    }
}

/// Add or remove values under one key of an item.
///
/// Stages one `set-property` action through the store's queue, as a
/// producer, then applies it when no sync holds the store; otherwise it
/// waits in the queue and applies at the next sync. Under `pimdir/label`
/// a value is compared ignoring case: adding `RECEIPTS` to an item labelled
/// `Receipts` changes nothing, and removing `receipts` removes it.
///
/// Exits 3 once the result is printed when the store refused the action,
/// which `pimdir queue list --parked` shows: no live placement of the
/// collection holds the item, or the key would hold more than 64 values.
#[derive(Debug, Args)]
pub struct PropertyChangeCommand {
    /// Public id of the item (`seq`).
    #[arg(value_name = "SEQ")]
    pub seq: i64,

    /// The key, `pimdir/label` or `<namespace>/<name>` outside `pimdir/`.
    #[arg(value_name = "KEY")]
    pub key: String,

    /// The values, each 1 to 1,024 bytes.
    #[arg(value_name = "VALUE", required = true)]
    pub values: Vec<String>,

    /// The collection to queue the action on, one holding the item live;
    /// the first that does by default.
    #[arg(long, short = 'c', value_name = "COLLECTION")]
    pub collection: Option<String>,
}

impl PropertyChangeCommand {
    /// Enqueues the change, then applies it if the owner role is free.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags, add: bool) -> Result<()> {
        let read = store.read()?;
        let collection = holding(&read, self.seq, self.collection.as_deref())?;
        let source = match &store.source {
            Some(source) => Some(source.clone()),
            None => single(read.distinct_sources().map_err(report)?),
        };
        drop(read);

        let (added, removed) = match add {
            true => (self.values, Vec::new()),
            false => (Vec::new(), self.values),
        };
        let action = PimdirAction::SetProperty {
            seq: self.seq,
            key: self.key.clone(),
            add: added,
            remove: removed,
        };
        let id = store
            .producer()?
            .enqueue(&collection, &action, None)
            .map_err(report)?
            .handle;

        let status = match (store.owner_if_free()?, source) {
            (Some(owner), Some(source)) => {
                let mut owner = owner.for_source(source);
                match owner.drain() {
                    Err(PimdirError::Busy) => PropertyStatus::Queued,
                    Err(err) => return Err(report(err)),
                    Ok(_) => match owner.action_status(id.clone()).map_err(report)? {
                        PimdirActionStatus::Applied { .. } => PropertyStatus::Applied,
                        PimdirActionStatus::Parked { .. } => PropertyStatus::Refused,
                        _ => PropertyStatus::Queued,
                    },
                }
            }
            _ => PropertyStatus::Queued,
        };

        printer.out(PropertyChangeOutput {
            seq: self.seq,
            collection,
            key: self.key,
            row: id.id,
            handle: id.to_string(),
            queued: status == PropertyStatus::Queued,
            status,
        })?;

        match status {
            PropertyStatus::Refused => Err(partial()),
            PropertyStatus::Applied | PropertyStatus::Queued => Ok(()),
        }
    }
}

/// The collection a change is queued on: the one asked for, else the first
/// holding the item live, the properties being the item's in every one.
fn holding(read: &PimdirReader, seq: i64, collection: Option<&str>) -> Result<String> {
    if let Some(collection) = collection {
        return Ok(collection.to_string());
    }
    for collection in read.list_collections().map_err(report)? {
        if read
            .get_item(&collection.id, seq)
            .map_err(report)?
            .is_some()
        {
            return Ok(collection.id);
        }
    }
    Err(ErrorCode::ItemNotFound.error(format!("no live item with seq {seq} in this store")))
}

/// The one source of a store syncing exactly one, the drain's handle; a
/// `set-property` runs as no source, but the drain applies every pending
/// row, so a store syncing none or several leaves it queued unless
/// `--source` names one.
fn single(sources: Vec<String>) -> Option<String> {
    match sources.len() {
        1 => sources.into_iter().next(),
        _ => None,
    }
}

/// One property as the listing prints it.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PropertyRow {
    /// The key, `<namespace>/<name>`.
    pub key: String,
    /// One value held under it.
    pub value: String,
}

/// The `property list` output.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PropertiesOutput {
    /// The item.
    pub seq: i64,
    /// Its properties, in key then value order.
    pub properties: Vec<PropertyRow>,
}

impl fmt::Display for PropertiesOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.properties.is_empty() {
            return writeln!(f, "No property on item {}", self.seq);
        }
        for property in &self.properties {
            let key = sanitize::text(&property.key);
            writeln!(f, "{key} = {}", sanitize::text(&property.value))?;
        }
        Ok(())
    }
}

/// What became of a change, which the text form says and the JSON tells by
/// `queued` and the exit status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PropertyStatus {
    /// The action was applied.
    Applied,
    /// Another writer holds the store lock, or no source to drain as was
    /// resolved, so the action waits in the queue for its sync engine.
    Queued,
    /// The drain ran and parked the action.
    Refused,
}

/// The `property add` and `property remove` output.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PropertyChangeOutput {
    /// The item.
    pub seq: i64,
    /// The collection the action was queued on.
    pub collection: String,
    /// The key changed.
    pub key: String,
    /// The queue row the change was appended as.
    pub row: i64,
    /// The row's handle, its id under the store's id and incarnation:
    /// what `queue status` and `queue cancel` check the store against.
    pub handle: String,
    /// Whether the row waits for the store's sync engine. `false` with exit
    /// status 3 when the drain ran and parked it.
    pub queued: bool,
    /// What became of the row, for the text form.
    #[serde(skip)]
    pub status: PropertyStatus,
}

impl fmt::Display for PropertyChangeOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let seq = self.seq;
        let key = sanitize::text(&self.key);
        match self.status {
            PropertyStatus::Applied => writeln!(f, "Changed {key} of item {seq} (applied)"),
            PropertyStatus::Queued => writeln!(
                f,
                "Change of {key} of item {seq} queued as action {}: it applies at the next sync",
                sanitize::text(&self.handle)
            ),
            PropertyStatus::Refused => writeln!(
                f,
                "Change of {key} of item {seq} was not applied: see `pimdir queue list --parked`"
            ),
        }
    }
}
