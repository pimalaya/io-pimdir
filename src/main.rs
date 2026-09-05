//! # pimdir
//!
//! The operator command line over a pimdir store: inspect it, restore
//! what a delete retained, purge what should really go, check what a
//! crash left behind, and collect what nothing references. It is to a
//! store what `sqlite3` is to a database, a tool for the person
//! maintaining the data rather than the person reading it.
//!
//! ## What it does not do
//!
//! It never interprets item content. A pimdir store is kind-agnostic, so
//! this binary prints the public id, the link id, the flags, the detail
//! level and the summary row as stored, and exports bodies as raw bytes.
//! Rendering a message or a vCard belongs to the per-kind clients
//! (himalaya, cardamum), which know the kind they hold.
//!
//! ## Roles
//!
//! Every read opens the store read-only, so inspecting one while a sync
//! runs is always safe. An item mutation is appended to the store's
//! action queue, as any other non-owner process does, and applied by
//! this process when the owner role is free; when a sync holds it, the
//! action stays queued for the next drain. Purge, queue cancellation,
//! repair and collection have no queue action kind, so they take the
//! owner role directly and say so plainly when they cannot.
//!
//! ## Layout
//!
//! [`cli`] holds the command tree, one module per verb group, plus the
//! shared store flags, the role handling and the JSON Schema registry.
//! The library underneath is [`io_pimdir`], and this tool's
//! specification lives in the repository's cairn/spec/cli.md.

mod cli;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use pimalaya_cli::{
    clap::{
        args::{JsonFlag, LogFlags},
        commands::{CompletionCommand, JsonSchemaCommand, ManualCommand},
    },
    error::ErrorReport,
    footer,
    log::Logger,
    long_version,
    printer::{Printer, StdoutPrinter},
};

use crate::cli::{
    StoreFlags, check::CheckCommand, collection::CollectionCommand, export::ExportCommand,
    gc::GcCommand, item::ItemCommand, json_schema, queue::QueueCommand, store::StoreCommand,
};

fn main() {
    let cli = Cli::parse();

    Logger::try_init(&cli.log).expect("init logger");
    let mut printer = StdoutPrinter::new(&cli.json);

    let result = cli.command.execute(&mut printer, &cli.store);
    ErrorReport::eval(&mut printer, result)
}

/// Top-level command-line interface parser.
#[derive(Parser, Debug)]
#[command(name = env!("CARGO_BIN_NAME"))]
#[command(about = "CLI to inspect, repair and recover a pimdir store")]
#[command(author, version, long_version = long_version!())]
#[command(after_help = footer!())]
#[command(propagate_version = true, infer_subcommands = true)]
struct Cli {
    /// The subcommand to run.
    #[command(subcommand)]
    pub command: Command,
    /// Where the store is and which source to act as.
    #[command(flatten)]
    pub store: StoreFlags,
    /// The log level and format.
    #[command(flatten)]
    pub log: LogFlags,
    /// Whether the output is rendered as JSON.
    #[command(flatten)]
    pub json: JsonFlag,
}

/// Top-level subcommands.
#[derive(Subcommand, Debug)]
enum Command {
    /// Inspect the store's collections (mailboxes, address books, calendars).
    #[command(subcommand, alias = "collections")]
    Collection(CollectionCommand),
    /// Inspect, restore and purge items.
    #[command(subcommand, alias = "items")]
    Item(ItemCommand),
    /// Inspect and prune the action queue.
    #[command(subcommand, alias = "queues")]
    Queue(QueueCommand),
    /// Inspect the store as a whole.
    #[command(subcommand, alias = "stores")]
    Store(StoreCommand),
    /// Check the store's internal consistency.
    Check(CheckCommand),
    /// Reclaim what nothing references any more.
    Gc(GcCommand),
    /// Dump the store to a directory.
    Export(ExportCommand),
    #[command(alias = "completions")]
    Completion(CompletionCommand),
    #[command(alias = "manuals")]
    Manual(ManualCommand),
    #[command(alias = "json-schemas")]
    JsonSchema(JsonSchemaCommand),
}

impl Command {
    /// Runs the selected subcommand against the store the flags name.
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        match self {
            Self::Collection(cmd) => cmd.execute(printer, store),
            Self::Item(cmd) => cmd.execute(printer, store),
            Self::Queue(cmd) => cmd.execute(printer, store),
            Self::Store(cmd) => cmd.execute(printer, store),
            Self::Check(cmd) => cmd.execute(printer, store),
            Self::Gc(cmd) => cmd.execute(printer, store),
            Self::Export(cmd) => cmd.execute(printer, store),
            Self::Completion(cmd) => cmd.execute(printer, Cli::command()),
            Self::Manual(cmd) => cmd.execute(printer, Cli::command()),
            Self::JsonSchema(cmd) => cmd.execute(printer, json_schema::schemas()),
        }
    }
}
