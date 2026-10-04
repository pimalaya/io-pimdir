//! The `queue` verb group: what is waiting to be applied, and what gave up.
//!
//! The queue is the write door for every process that does not own the
//! store. An action waits there until the owner applies it; one the owner
//! judged permanently unappliable is parked with its error instead of
//! blocking the others, and stays until somebody looks. This is that
//! somebody.

use std::fmt;

use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use io_pimdir::{client::producer::PimdirActionStatus, codec::PimdirAction};
use log::warn;
use pimalaya_cli::{
    printer::Printer,
    table::{Cell, ContentArrangement, Table, presets::UTF8_FULL},
};
use schemars::JsonSchema;
use serde::Serialize;

use crate::cli::{StoreFlags, confirm, or_dash, report};

/// Inspect and prune the store's action queue.
#[derive(Debug, Subcommand)]
pub enum QueueCommand {
    /// List the queued actions.
    List(QueueListCommand),
    /// Drop one queued action by its id.
    Cancel(QueueCancelCommand),
    /// Show where one action stands, by the id its enqueue answered.
    Status(QueueStatusCommand),
}

impl QueueCommand {
    /// Runs the selected subcommand.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        match self {
            Self::List(cmd) => cmd.execute(printer, store),
            Self::Cancel(cmd) => cmd.execute(printer, store),
            Self::Status(cmd) => cmd.execute(printer, store),
        }
    }
}

/// List the actions waiting in the queue, in append order.
///
/// Pending actions are the ones the owner will apply at its next drain.
/// `--parked` shows the ones it refused instead: each carries the failure that
/// parked it, and none of them will ever be applied without an operator.
#[derive(Debug, Args)]
pub struct QueueListCommand {
    /// List the parked actions instead of the pending ones.
    #[arg(long)]
    pub parked: bool,
}

impl QueueListCommand {
    /// Prints the queue.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let store = store.read()?;
        let mut rows = Vec::new();
        let mut undecodable = Vec::new();

        if self.parked {
            for action in store.parked_actions().map_err(report)? {
                rows.push(QueueRow {
                    id: action.id,
                    created_at: action.created_at,
                    producer: action.producer,
                    collection: action.collection,
                    kind: action.action,
                    summary: action.payload,
                    attempts: action.attempts,
                    error: Some(action.error),
                });
            }
        } else {
            for collection in store.list_collections().map_err(report)? {
                let collection = collection.id;
                match store.pending_actions(&collection) {
                    Ok(actions) => {
                        for action in actions {
                            rows.push(QueueRow {
                                id: action.id,
                                created_at: action.created_at,
                                producer: action.producer,
                                collection: action.collection,
                                kind: action.action.kind().to_string(),
                                summary: summary(&action.action),
                                attempts: action.attempts,
                                error: None,
                            });
                        }
                    }
                    Err(err) => {
                        // NOTE: one undecodable payload must not hide the
                        // whole queue, so the collection is reported.
                        warn!("cannot decode the pending actions of {collection}: {err}");
                        undecodable.push(collection);
                    }
                }
            }
            rows.sort_by_key(|row| row.id);
        }

        printer.out(QueueOutput {
            parked: self.parked,
            actions: rows,
            undecodable,
        })
    }
}

/// Drop one queued action, by the id `queue list` prints.
///
/// Only the owner pops rows, so an action a producer enqueued cannot be taken
/// back any other way: this is the undo. It works on a pending action and on a
/// parked one alike. Destructive (the action is gone, not deferred), so it asks
/// first unless `--yes` is passed.
#[derive(Debug, Args)]
pub struct QueueCancelCommand {
    /// Id of the queued action to drop.
    #[arg(value_name = "ID")]
    pub id: i64,

    /// Do not ask for confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

impl QueueCancelCommand {
    /// Drops the action.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        confirm(
            printer,
            self.yes,
            &format!("Drop queued action {} for good?", self.id),
        )?;

        let dropped = store.owner()?.drop_action(self.id).map_err(report)?;

        if !dropped {
            bail!("no queued action with id {}", self.id);
        }

        printer.out(QueueCancelOutput { id: self.id })
    }
}

/// Show where one action stands, by the id its enqueue answered.
///
/// Pending or parked while its row is queued; once the owner applied it,
/// its receipt says so and, for an add, names the seq of the item it
/// created. A receipt is kept a week at least; an id found nowhere was
/// cancelled, or applied before that.
#[derive(Debug, Args)]
pub struct QueueStatusCommand {
    /// Id of the action, as the enqueue answered or `queue list` prints it.
    #[arg(value_name = "ID")]
    pub id: i64,
}

impl QueueStatusCommand {
    /// Prints the action's status.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let status = store.read()?.action_status(self.id).map_err(report)?;
        let output = match status {
            PimdirActionStatus::Pending {
                collection,
                kind,
                attempts,
            } => QueueStatusOutput {
                id: self.id,
                status: "pending",
                collection: Some(collection),
                kind: Some(kind),
                attempts: Some(attempts),
                ..Default::default()
            },
            PimdirActionStatus::Parked {
                collection,
                kind,
                attempts,
                error,
            } => QueueStatusOutput {
                id: self.id,
                status: "parked",
                collection: Some(collection),
                kind: Some(kind),
                attempts: Some(attempts),
                error: Some(error),
                ..Default::default()
            },
            PimdirActionStatus::Applied {
                applied_at,
                collection,
                seq,
            } => QueueStatusOutput {
                id: self.id,
                status: "applied",
                collection: Some(collection),
                applied_at: Some(applied_at),
                seq,
                ..Default::default()
            },
            PimdirActionStatus::Unknown => QueueStatusOutput {
                id: self.id,
                status: "unknown",
                ..Default::default()
            },
        };
        printer.out(output)
    }
}

/// The `queue status` output.
#[derive(Debug, Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QueueStatusOutput {
    /// The action's id.
    pub id: i64,
    /// `pending`, `parked`, `applied` or `unknown` (cancelled, or applied
    /// before its receipt was pruned).
    pub status: &'static str,
    /// The collection it was queued on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
    /// The action kind, while queued.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Apply attempts so far, while queued.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempts: Option<i64>,
    /// The failure that parked it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// When the owner applied it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied_at: Option<String>,
    /// The item an applied add created.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,
}

impl fmt::Display for QueueStatusOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let id = self.id;
        let collection = or_dash(self.collection.as_deref());
        match self.status {
            "pending" => writeln!(f, "Action {id} is pending on {collection}"),
            "parked" => writeln!(
                f,
                "Action {id} is parked on {collection}: {}",
                or_dash(self.error.as_deref())
            ),
            "applied" => match self.seq {
                Some(seq) => writeln!(f, "Action {id} was applied: item {seq} in {collection}"),
                None => writeln!(f, "Action {id} was applied on {collection}"),
            },
            _ => writeln!(
                f,
                "Action {id} is neither queued nor applied recently: cancelled, or applied before its receipt was pruned"
            ),
        }
    }
}

/// A one-line, content-free summary of an action: ids, hashes and flags, never
/// what the item says.
fn summary(action: &PimdirAction) -> String {
    match action {
        PimdirAction::Add {
            link_id, object, ..
        } => {
            let link = link_id
                .as_ref()
                .map(|link| link.0.clone())
                .unwrap_or_else(|| String::from("(from object)"));
            match object {
                Some(hash) => format!("link {link}, object {}", hash.0),
                None => format!("link {link}"),
            }
        }
        PimdirAction::SetFlags { seq, flags } => {
            let flags: Vec<&str> = flags
                .known()
                .into_iter()
                .flatten()
                .map(String::as_str)
                .collect();
            format!("seq {seq} -> [{}]", flags.join(" "))
        }
        PimdirAction::Remove { seq } => format!("seq {seq}"),
        PimdirAction::Move { seq, to } => format!("seq {seq} -> {}", to.0),
        PimdirAction::Copy { seq, to } => format!("seq {seq} -> {}", to.0),
        PimdirAction::Update { seq, object } => format!("seq {seq}, object {}", object.0),
        PimdirAction::SetPerformer { capability, source } => {
            format!("{capability} -> {}", source.as_deref().unwrap_or("none"))
        }
        // NOTE: an owner-defined intent this build has no semantics for,
        // so its payload is printed verbatim.
        PimdirAction::Unknown { payload, .. } => payload.clone(),
    }
}

/// One queued action as the listing prints it.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QueueRow {
    /// The row's append id.
    pub id: i64,
    /// The instant SQLite stamped the row with.
    pub created_at: String,
    /// The process that enqueued it.
    pub producer: String,
    /// The collection it targets.
    pub collection: String,
    /// The action kind.
    pub kind: String,
    /// A content-free summary (the raw payload for a parked row).
    pub summary: String,
    /// Apply attempts so far.
    pub attempts: i64,
    /// The failure that parked the row, when parked.
    pub error: Option<String>,
}

/// The `queue list` output.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QueueOutput {
    /// Whether the listing shows parked actions.
    pub parked: bool,
    /// The actions.
    pub actions: Vec<QueueRow>,
    /// Collections whose pending payloads could not be decoded, so their rows
    /// are missing from the listing.
    pub undecodable: Vec<String>,
}

impl fmt::Display for QueueOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.actions.is_empty() {
            let what = if self.parked { "parked" } else { "pending" };
            writeln!(f, "No {what} action in this store's queue")?;
        } else {
            let mut table = Table::new();
            let mut header = vec![
                Cell::new("ID"),
                Cell::new("CREATED"),
                Cell::new("PRODUCER"),
                Cell::new("COLLECTION"),
                Cell::new("ACTION"),
                Cell::new("TARGET"),
                Cell::new("TRIES"),
            ];
            if self.parked {
                header.push(Cell::new("ERROR"));
            }

            table
                .load_style(UTF8_FULL)
                .set_content_arrangement(ContentArrangement::Dynamic)
                .set_header(header);

            for action in &self.actions {
                let mut row = vec![
                    Cell::new(action.id),
                    Cell::new(&action.created_at),
                    Cell::new(&action.producer),
                    Cell::new(&action.collection),
                    Cell::new(&action.kind),
                    Cell::new(&action.summary),
                    Cell::new(action.attempts),
                ];
                if self.parked {
                    row.push(Cell::new(or_dash(action.error.as_deref())));
                }
                table.add_row(row);
            }

            writeln!(f, "{table}")?;
        }

        // NOTE: never silently short: a collection left out of the
        // listing is said out loud.
        for collection in &self.undecodable {
            writeln!(
                f,
                "The pending actions of {collection} could not be decoded and are not listed; the next drain will park them"
            )?;
        }

        Ok(())
    }
}

/// The `queue cancel` output.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QueueCancelOutput {
    /// The dropped action's id.
    pub id: i64,
}

impl fmt::Display for QueueCancelOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Dropped queued action {}", self.id)
    }
}
