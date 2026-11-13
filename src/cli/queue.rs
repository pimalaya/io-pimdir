//! The `queue` verb group: what is waiting to be applied, and what gave up.
//!
//! The queue is the write door for every process that does not own the
//! store. An action waits there until the owner applies it; one the owner
//! judged permanently unappliable is parked with its error instead of
//! blocking the others, and stays until somebody looks. This is that
//! somebody.

use std::fmt;

use anyhow::Result;
use clap::{Args, Subcommand};
use io_pimdir::{
    client::producer::PimdirActionStatus,
    codec::{PimdirAction, PimdirActionHandle, action_to_payload},
};
use log::warn;
use pimalaya_cli::{
    fields::Fields,
    printer::Printer,
    records::{Record, Records},
    sanitize,
};
use schemars::JsonSchema;
use serde::Serialize;

use crate::cli::{
    StoreFlags, confirm,
    error::{ErrorCode, report},
    or_dash,
};

/// Inspect and prune the store's action queue.
#[derive(Debug, Subcommand)]
pub enum QueueCommand {
    /// List the queued actions.
    List(QueueListCommand),
    /// Drop one queued action by its id.
    Cancel(QueueCancelCommand),
    /// Put one parked action back among the pending ones.
    Retry(QueueRetryCommand),
    /// Show where one action stands, by the id its enqueue answered.
    Status(QueueStatusCommand),
}

impl QueueCommand {
    /// Runs the selected subcommand.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        match self {
            Self::List(cmd) => cmd.execute(printer, store),
            Self::Cancel(cmd) => cmd.execute(printer, store),
            Self::Retry(cmd) => cmd.execute(printer, store),
            Self::Status(cmd) => cmd.execute(printer, store),
        }
    }
}

/// List the actions waiting in the queue, in append order.
///
/// Pending actions are the ones the owner will apply at its next drain.
/// `--parked` shows the ones it refused instead: each carries the failure that
/// parked it, and none of them will ever be applied without an operator. A
/// parked intent whose outcome is unknown, sent and never confirmed, is marked
/// unconfirmed: it may have taken effect.
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
                let unconfirmed = action.unconfirmed().is_some();
                rows.push(QueueRow {
                    row: action.id,
                    created_at: action.created_at,
                    producer: action.producer,
                    collection: action.collection,
                    kind: action.action,
                    target: action.payload.clone(),
                    payload: action.payload,
                    attempts: action.attempts,
                    unconfirmed,
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
                                row: action.id,
                                created_at: action.created_at,
                                producer: action.producer,
                                collection: action.collection,
                                kind: action.action.kind().to_string(),
                                target: summary(&action.action),
                                payload: action_to_payload(&action.action),
                                attempts: action.attempts,
                                unconfirmed: false,
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
            rows.sort_by_key(|row| row.row);
        }

        printer.out(QueueOutput {
            parked: self.parked,
            actions: rows,
            undecodable,
        })
    }
}

/// Drop one queued action, by the id `queue list` prints or the handle
/// its enqueue answered.
///
/// Only the owner pops rows, so an action a producer enqueued cannot be taken
/// back any other way: this is the undo. It works on a pending action and on a
/// parked one alike. Destructive (the action is gone, not deferred), so it asks
/// first unless `--yes` is passed.
#[derive(Debug, Args)]
pub struct QueueCancelCommand {
    /// Id of the queued action to drop, or its handle (the store id, the
    /// incarnation and the id joined by periods): a handle is refused when
    /// the store is no longer the one that answered it.
    #[arg(value_name = "ACTION")]
    pub action: PimdirActionHandle,

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
            &format!("Drop queued action {} for good?", self.action),
        )?;

        let id = self.action.id;
        let dropped = store.owner()?.drop_action(self.action).map_err(report)?;

        if !dropped {
            return Err(ErrorCode::RowNotFound.error(format!("no queued action with id {id}")));
        }

        printer.out(QueueCancelOutput { row: id })
    }
}

/// Put one parked action back among the pending ones, by the id `queue
/// list --parked` prints or the handle its enqueue answered.
///
/// The next drain applies or performs it in its place in the append order.
/// Nothing retries a parked action otherwise. An action parked for a
/// failure that still stands parks again. An intent parked unconfirmed was
/// sent and may have taken effect: retrying it may perform it twice (a
/// message sent twice), so it asks first unless `--yes` is passed.
#[derive(Debug, Args)]
pub struct QueueRetryCommand {
    /// Id of the parked action to retry, or its handle (the store id, the
    /// incarnation and the id joined by periods): a handle is refused when
    /// the store is no longer the one that answered it.
    #[arg(value_name = "ACTION")]
    pub action: PimdirActionHandle,

    /// Do not ask for confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

impl QueueRetryCommand {
    /// Puts the action back.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let id = self.action.id;
        let mut owner = store.owner()?;

        let status = owner.action_status(&self.action).map_err(report)?;
        let unconfirmed = matches!(status, PimdirActionStatus::Unconfirmed { .. });
        if unconfirmed {
            confirm(
                printer,
                self.yes,
                &format!("Action {id} may have taken effect already. Perform it again?"),
            )?;
        }

        if !owner.retry_action(self.action).map_err(report)? {
            return Err(ErrorCode::RowNotFound.error(format!("no parked action with id {id}")));
        }

        printer.out(QueueRetryOutput {
            row: id,
            unconfirmed,
        })
    }
}

/// Show where one action stands, by the handle or the id its enqueue
/// answered.
///
/// Pending or parked while its row is queued, unconfirmed when it is an
/// intent sent whose outcome nobody knows; once the owner applied it,
/// or performed and acknowledged an intent, its receipt says so and, for
/// an add, names the seq of the item it created. A receipt is kept a week
/// at least; an id found nowhere was cancelled, or applied before that. A
/// handle the store did not answer, a copy restored since, reads `reset`.
#[derive(Debug, Args)]
pub struct QueueStatusCommand {
    /// Id of the action, as `queue list` prints it, or its handle as the
    /// enqueue answered it (the store id, the incarnation and the id
    /// joined by periods).
    #[arg(value_name = "ACTION")]
    pub action: PimdirActionHandle,
}

impl QueueStatusCommand {
    /// Prints the action's status.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        let status = store.read()?.action_status(&self.action).map_err(report)?;
        let mut output = QueueStatusOutput {
            row: self.action.id,
            status: QueueStatus::Unknown,
            collection: None,
            kind: None,
            attempts: None,
            error: None,
            applied_at: None,
            seq: None,
        };

        match status {
            PimdirActionStatus::Pending {
                collection,
                kind,
                attempts,
            } => {
                output.status = QueueStatus::Pending;
                output.collection = Some(collection);
                output.kind = Some(kind);
                output.attempts = Some(attempts);
            }
            PimdirActionStatus::Parked {
                collection,
                kind,
                attempts,
                error,
            } => {
                output.status = QueueStatus::Parked;
                output.collection = Some(collection);
                output.kind = Some(kind);
                output.attempts = Some(attempts);
                output.error = Some(error);
            }
            PimdirActionStatus::Unconfirmed {
                collection,
                kind,
                attempts,
                detail,
            } => {
                output.status = QueueStatus::Unconfirmed;
                output.collection = Some(collection);
                output.kind = Some(kind);
                output.attempts = Some(attempts);
                output.error = Some(detail);
            }
            PimdirActionStatus::Applied {
                applied_at,
                collection,
                seq,
            } => {
                output.status = QueueStatus::Applied;
                output.collection = collection;
                output.applied_at = Some(applied_at);
                output.seq = seq;
            }
            PimdirActionStatus::Unknown => (),
            PimdirActionStatus::Reset => output.status = QueueStatus::Reset,
        }

        printer.out(output)
    }
}

/// Where a queue row stands.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum QueueStatus {
    /// Queued, waiting for the owner's next drain.
    Pending,
    /// Queued, refused by the owner until an operator acts.
    Parked,
    /// Queued and parked, an intent sent whose outcome is unknown: it may
    /// have taken effect, and waits for its performer to confirm it, or
    /// for an operator to retry or cancel it.
    Unconfirmed,
    /// Applied, its receipt still kept.
    Applied,
    /// Found nowhere: cancelled, or applied before its receipt was pruned.
    Unknown,
    /// The store is not the one that answered the handle: a copy restored
    /// since, where the id may name another row, so nothing is read of it.
    Reset,
}

/// The `queue status` output.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QueueStatusOutput {
    /// The queue row, by the id its enqueue answered.
    pub row: i64,
    /// Where it stands.
    pub status: QueueStatus,
    /// The collection it was queued on, `None` when unknown or, for an
    /// applied action, deleted since.
    pub collection: Option<String>,
    /// The action kind, while queued.
    pub kind: Option<String>,
    /// Apply attempts so far, while queued.
    pub attempts: Option<i64>,
    /// The failure that parked it, or what its performer recorded of an
    /// unconfirmed intent.
    pub error: Option<String>,
    /// When the owner applied it.
    pub applied_at: Option<String>,
    /// The item an applied add created.
    pub seq: Option<i64>,
}

impl fmt::Display for QueueStatusOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let id = self.row;
        let collection = or_dash(self.collection.as_deref());
        match self.status {
            QueueStatus::Pending => writeln!(f, "Action {id} is pending on {collection}"),
            QueueStatus::Parked => writeln!(
                f,
                "Action {id} is parked on {collection}: {}",
                or_dash(self.error.as_deref())
            ),
            QueueStatus::Unconfirmed => writeln!(
                f,
                "Action {id} is unconfirmed on {collection}, sent and maybe performed: {}",
                or_dash(self.error.as_deref())
            ),
            QueueStatus::Applied => match (self.seq, &self.collection) {
                (Some(seq), Some(_)) => {
                    writeln!(f, "Action {id} was applied: item {seq} in {collection}")
                }
                (Some(seq), None) => writeln!(
                    f,
                    "Action {id} was applied: item {seq} in a collection deleted since"
                ),
                (None, Some(_)) => writeln!(f, "Action {id} was applied on {collection}"),
                (None, None) => {
                    writeln!(f, "Action {id} was applied on a collection deleted since")
                }
            },
            QueueStatus::Unknown => writeln!(
                f,
                "Action {id} is neither queued nor applied recently: cancelled, or applied before its receipt was pruned"
            ),
            QueueStatus::Reset => writeln!(
                f,
                "Action {id} was queued in another store or another incarnation of this one: a copy restored since, where this id may name another action"
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
        PimdirAction::SetFlags { seq, flags, base } => {
            let joined = |flags: &io_pimdir::placement::PimdirFlags| {
                let flags: Vec<&str> = flags
                    .known()
                    .into_iter()
                    .flatten()
                    .map(String::as_str)
                    .collect();
                flags.join(" ")
            };
            match base {
                Some(base) => format!("seq {seq} [{}] -> [{}]", joined(base), joined(flags)),
                None => format!("seq {seq} -> [{}]", joined(flags)),
            }
        }
        PimdirAction::Remove {
            seq,
            if_match,
            resolves,
        } => {
            let mut summary = format!("seq {seq}");
            if let Some(read) = if_match {
                summary.push_str(&format!(", if-match {}", read.0));
            }
            if let Some(resolves) = resolves {
                let names: Vec<&str> = resolves.iter().map(|hash| hash.0.as_str()).collect();
                summary.push_str(&format!(", resolves {}", names.join(" ")));
            }
            summary
        }
        PimdirAction::Move { seq, to } => format!("seq {seq} -> {}", to.0),
        PimdirAction::Copy { seq, to } => format!("seq {seq} -> {}", to.0),
        PimdirAction::Update {
            seq,
            object,
            if_match,
            resolves,
        } => {
            let mut summary = format!("seq {seq}, object {}", object.0);
            if let Some(read) = if_match {
                summary.push_str(&format!(", if-match {}", read.0));
            }
            if let Some(resolves) = resolves {
                let names: Vec<&str> = resolves.iter().map(|hash| hash.0.as_str()).collect();
                summary.push_str(&format!(", resolves {}", names.join(" ")));
            }
            summary
        }
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
    /// The queue row, by its append id.
    pub row: i64,
    /// The instant SQLite stamped the row with.
    pub created_at: String,
    /// The process that enqueued it.
    pub producer: String,
    /// The collection it targets.
    pub collection: String,
    /// The action kind.
    pub kind: String,
    /// The action's payload, as the queue stores it.
    pub payload: String,
    /// What the text list shows of it: ids, hashes and flags, or the
    /// payload of a parked row.
    #[serde(skip)]
    pub target: String,
    /// Apply attempts so far.
    pub attempts: i64,
    /// Whether the row is a parked intent whose outcome is unknown: sent,
    /// and maybe performed.
    pub unconfirmed: bool,
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
            let mut records = Records::default();

            for action in &self.actions {
                let mut fields = Fields::default();

                fields.push("Created", sanitize::text(&action.created_at));
                fields.push("Producer", sanitize::text(&action.producer));
                fields.push("Collection", sanitize::text(&action.collection));
                fields.push("Action", sanitize::text(&action.kind));
                fields.push("Tries", action.attempts);
                fields.push("Target", sanitize::text(&action.target));

                if self.parked {
                    let error = or_dash(action.error.as_deref());
                    fields.push("Error", sanitize::text(error));
                }

                let state = match action.unconfirmed {
                    true => vec![String::from("unconfirmed")],
                    false => Vec::new(),
                };

                records.push(Record {
                    title: action.row.to_string(),
                    state,
                    fields,
                    body: String::new(),
                });
            }

            write!(f, "{records}")?;

            if !self.undecodable.is_empty() {
                writeln!(f)?;
            }
        }

        // NOTE: never silently short: a collection left out of the
        // listing is said out loud.
        for collection in &self.undecodable {
            let collection = sanitize::text(collection);
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
    /// The dropped queue row.
    pub row: i64,
}

impl fmt::Display for QueueCancelOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Dropped queued action {}", self.row)
    }
}

/// The `queue retry` output.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QueueRetryOutput {
    /// The queue row put back among the pending ones.
    pub row: i64,
    /// Whether it was an intent whose outcome is unknown, which may now
    /// be performed a second time.
    pub unconfirmed: bool,
}

impl fmt::Display for QueueRetryOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Queued action {} is pending again", self.row)
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::queue::*;

    #[test]
    fn a_parked_list_prints_one_record_per_action() {
        let output = QueueOutput {
            parked: true,
            actions: vec![QueueRow {
                row: 4,
                created_at: String::from("2026-11-11T18:13:39.904Z"),
                producer: String::from("himalaya"),
                collection: String::from("INBOX"),
                kind: String::from("set-flags"),
                payload: String::from("{\"seq\":1}"),
                target: String::from("{\"seq\":1}"),
                attempts: 3,
                unconfirmed: false,
                error: Some(String::from("no such seq\u{7}")),
            }],
            undecodable: Vec::new(),
        };

        let expected = "\
4
Created:     2026-11-11T18:13:39.904Z
Producer:    himalaya
Collection:  INBOX
Action:      set-flags
Tries:       3
Target:      {\"seq\":1}
Error:       no such seq\u{fffd}
";
        assert_eq!(output.to_string(), expected);
    }

    #[test]
    fn an_applied_action_says_when_its_collection_is_gone() {
        let status = |collection: Option<&str>, seq| QueueStatusOutput {
            row: 4,
            status: QueueStatus::Applied,
            collection: collection.map(String::from),
            kind: None,
            attempts: None,
            error: None,
            applied_at: Some(String::from("2026-11-11T18:13:39.904Z")),
            seq,
        };

        assert_eq!(
            status(Some("Drafts"), Some(7)).to_string(),
            "Action 4 was applied: item 7 in Drafts\n"
        );
        assert_eq!(
            status(None, Some(7)).to_string(),
            "Action 4 was applied: item 7 in a collection deleted since\n"
        );
        assert_eq!(
            status(Some("Drafts"), None).to_string(),
            "Action 4 was applied on Drafts\n"
        );
        assert_eq!(
            status(None, None).to_string(),
            "Action 4 was applied on a collection deleted since\n"
        );
    }
}
