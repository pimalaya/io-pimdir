//! # pimdir
//!
//! The operator command line over a pimdir store: inspect it, restore what a
//! delete retained, purge what should really go, and check what a crash left
//! behind. It is to a store what `sqlite3` is to a database, a tool for the
//! person maintaining the data rather than for the person reading it.
//!
//! ## What it does not do
//!
//! It never interprets item content. A pimdir store is kind-agnostic (mail,
//! contacts, calendars all live in the same shape), so this binary prints the
//! public id, the link id, the flags, the detail level and the **raw** meta,
//! and exports bodies as raw bytes. Rendering a message or a vCard belongs to
//! the per-kind clients (himalaya, cardamum), which know the kind they hold.
//!
//! ## Roles
//!
//! Every read opens the store read-only, so inspecting a store while a sync is
//! running is always safe. An item mutation is appended to the store's action
//! queue, exactly as any other non-owner process does, and then applied by this
//! process when the owner role is free; when a sync holds it, the action stays
//! queued and applies at the next drain. Purge, queue cancellation and the
//! orphan-blob sweep have no queue action kind, so they take the owner role
//! directly and say so plainly when they cannot.
//!
//! ## Layout
//!
//! [`cli`] holds the command tree, one module per verb group, plus the shared
//! store flags, the role handling and the read-only diagnostic connection the
//! consistency checks use. The library the whole thing is a skin over is
//! [`io_pimdir`]; the specification of this tool lives in the repository's
//! `cairn/spec/cli.md`.

mod cli;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use pimalaya_cli::{
    clap::{
        args::{JsonFlag, LogFlags},
        commands::{CompletionCommand, ManualCommand},
    },
    error::ErrorReport,
    log::Logger,
    long_version,
    printer::{Printer, StdoutPrinter},
};

use crate::cli::{
    StoreFlags, check::CheckCommand, collection::CollectionCommand, export::ExportCommand,
    item::ItemCommand, queue::QueueCommand, store::StoreCommand,
};

fn main() {
    let cli = Cli::parse();

    Logger::try_init(&cli.log).expect("init logger");
    let mut printer = StdoutPrinter::new(&cli.json);

    let result = cli.command.execute(&mut printer, &cli.store);
    ErrorReport::eval(&mut printer, result)
}

#[derive(Parser, Debug)]
#[command(name = env!("CARGO_BIN_NAME"))]
#[command(about = "CLI to inspect, repair and recover a pimdir store")]
#[command(author, version, long_version = long_version!())]
#[command(propagate_version = true, infer_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    pub command: Command,
    #[command(flatten)]
    pub store: StoreFlags,
    #[command(flatten)]
    pub log: LogFlags,
    #[command(flatten)]
    pub json: JsonFlag,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Inspect the store's collections (mailboxes, address books, calendars).
    #[command(subcommand)]
    Collection(CollectionCommand),
    /// Inspect, restore and purge items.
    #[command(subcommand)]
    Item(ItemCommand),
    /// Inspect and prune the action queue.
    #[command(subcommand)]
    Queue(QueueCommand),
    /// Inspect the store as a whole.
    #[command(subcommand)]
    Store(StoreCommand),
    /// Check the store's internal consistency.
    Check(CheckCommand),
    /// Dump the store to a directory.
    Export(ExportCommand),
    Completions(CompletionCommand),
    Manuals(ManualCommand),
}

impl Command {
    pub fn execute(self, printer: &mut impl Printer, store: &StoreFlags) -> Result<()> {
        match self {
            Self::Collection(cmd) => cmd.execute(printer, store),
            Self::Item(cmd) => cmd.execute(printer, store),
            Self::Queue(cmd) => cmd.execute(printer, store),
            Self::Store(cmd) => cmd.execute(printer, store),
            Self::Check(cmd) => cmd.execute(printer, store),
            Self::Export(cmd) => cmd.execute(printer, store),
            Self::Completions(cmd) => cmd.execute(printer, Cli::command()),
            Self::Manuals(cmd) => cmd.execute(printer, Cli::command()),
        }
    }
}
