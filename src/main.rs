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

use std::{env, ffi::OsString};

use anyhow::Result;
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};
use pimalaya_cli::{
    clap::{
        args::{JsonFlag, LogFlags},
        commands::{
            completion::CompletionCommand, json_schema::JsonSchemaCommand, manual::ManualCommand,
        },
    },
    error::ErrorReport,
    footer,
    log::Logger,
    long_version,
    printer::{Printer, StdoutPrinter},
};
use simd_cpu::SimdCpu;

use crate::cli::{
    StoreFlags,
    check::CheckCommand,
    collection::CollectionCommand,
    error::ErrorCode,
    export::ExportCommand,
    gc::GcCommand,
    item::ItemCommand,
    json_schema,
    property::PropertyCommand,
    quarantine::QuarantineCommand,
    queue::QueueCommand,
    search::{CountCommand, IndexCommand, SearchCommand, ShowCommand, TagCommand, ThreadCommand},
    store::StoreCommand,
    version::VersionCommand,
};

fn main() {
    untrack_sqlite_memory();
    // NOTE: the base64 and quoted-printable codecs below read the
    // installed CPU token, the portable baseline until one is installed.
    let _ = SimdCpu::install_detected();

    let cli = Cli::parse_query(env::args_os()).unwrap_or_else(Cli::parse);

    let mut printer = StdoutPrinter::new(&cli.json);

    let result =
        Logger::try_init(&cli.log).and_then(|()| cli.command.execute(&mut printer, &cli.store));
    ErrorReport::<ErrorCode>::eval_coded(&mut printer, result)
}

/// Turns SQLite's memory statistics off before any connection opens: no
/// global mutex nor counter per allocation, and every open parses the
/// store's schema in thousands of them (a twelfth of a query's time). This
/// process reads no statistic nor sets a heap limit. `sqlite3_config` must
/// run before SQLite initializes, so first in `main`.
fn untrack_sqlite_memory() {
    // SAFETY: no thread nor connection exists yet; the option takes an int.
    unsafe {
        rusqlite::ffi::sqlite3_config(
            rusqlite::ffi::SQLITE_CONFIG_MEMSTATUS,
            0 as core::ffi::c_int,
        );
    }
}

/// Top-level command-line interface parser.
#[derive(Parser, Debug)]
#[command(name = env!("CARGO_BIN_NAME"))]
#[command(about = "CLI to inspect, repair and recover a pimdir store")]
#[command(long_about = "CLI to inspect, repair and recover a pimdir store\n\n\
    An operator tool outside io-pimdir's 1.0 contract, but for its JSON \
    output: its commands, flags, text output and operator verbs may change \
    in any release.\n\n\
    The contract is what --json prints for search, count, show, thread, \
    tag, item list, item show, collection list, queue list, queue status and \
    store info, and the failure object (pimdir-error): stable within a \
    major, a minor only adding keys, and values to vocabularies declared \
    open. `pimdir --json version` states that major as formatVersion, and \
    `pimdir json-schema` describes each payload.")]
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

impl Cli {
    /// The arguments of a `count` or a `search` read by a root holding
    /// those two subcommands and the global flags alone, `None` for any
    /// other or any error, help and version included, which the whole tree
    /// reads then and reports: building every verb with every argument
    /// and its help costs a short query a thirtieth of its time. The verbs
    /// are matched by their full names, never inferred, so a line this
    /// root reads is one the whole tree reads alike.
    fn parse_query(args: impl IntoIterator<Item = OsString>) -> Option<Self> {
        let root = clap::Command::new(env!("CARGO_BIN_NAME"))
            .subcommand(CountCommand::augment_args(clap::Command::new("count")))
            .subcommand(SearchCommand::augment_args(clap::Command::new("search")));
        let root = JsonFlag::augment_args(LogFlags::augment_args(StoreFlags::augment_args(root)));
        let mut matches = root.try_get_matches_from(args).ok()?;
        Self::from_arg_matches_mut(&mut matches).ok()
    }
}

/// Top-level subcommands.
#[derive(Subcommand, Debug)]
enum Command {
    /// Inspect the store's collections (mailboxes, address books, calendars).
    #[command(subcommand, alias = "collections")]
    Collection(CollectionCommand),
    /// Inspect, restore, purge and erase items.
    #[command(subcommand, alias = "items")]
    Item(ItemCommand),
    /// List, add and remove an item's properties, its local state.
    #[command(subcommand, alias = "properties")]
    Property(PropertyCommand),
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
    /// Empty the bodies that failed their name.
    #[command(subcommand, alias = "quarantines")]
    Quarantine(QuarantineCommand),
    /// Dump the store to a directory.
    Export(ExportCommand),
    /// Fold the store's changes into its search index.
    Index(IndexCommand),
    /// Search the store's items with a query.
    Search(SearchCommand),
    /// Count the items a query matches.
    Count(CountCommand),
    /// Show one item as the search index reads it.
    Show(ShowCommand),
    /// Show the mail thread of one item.
    Thread(ThreadCommand),
    /// Add or remove flags on every item a query matches.
    Tag(TagCommand),
    Version(VersionCommand),
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
            Self::Property(cmd) => cmd.execute(printer, store),
            Self::Queue(cmd) => cmd.execute(printer, store),
            Self::Store(cmd) => cmd.execute(printer, store),
            Self::Check(cmd) => cmd.execute(printer, store),
            Self::Gc(cmd) => cmd.execute(printer, store),
            Self::Quarantine(cmd) => cmd.execute(printer, store),
            Self::Export(cmd) => cmd.execute(printer, store),
            Self::Index(cmd) => cmd.execute(printer, store),
            Self::Search(cmd) => cmd.execute(printer, store),
            Self::Count(cmd) => cmd.execute(printer, store),
            Self::Show(cmd) => cmd.execute(printer, store),
            Self::Thread(cmd) => cmd.execute(printer, store),
            Self::Tag(cmd) => cmd.execute(printer, store),
            Self::Version(cmd) => cmd.execute(printer),
            Self::Completion(cmd) => cmd.execute(printer, Cli::command()),
            Self::Manual(cmd) => cmd.execute(printer, Cli::command()),
            Self::JsonSchema(cmd) => cmd.execute(printer, json_schema::registry()),
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::Cli;

    /// Every line the query root reads, the whole tree reads alike, and a
    /// line it leaves (another verb, an inferred one, help, version, an
    /// error) is the whole tree's to read.
    #[test]
    fn the_query_root_reads_as_the_whole_tree() {
        let read: &[&[&str]] = &[
            &["-s", "/tmp", "count", "--refresh-timeout", "0s", "kiwi"],
            &[
                "--json",
                "-s",
                "/tmp",
                "search",
                "--refresh-timeout",
                "0s",
                "-n",
                "50",
                "a b",
            ],
            &[
                "search",
                "--threads",
                "-n",
                "5",
                "--sort",
                "oldest-first",
                "q",
                "--json",
            ],
            &[
                "count",
                "--zone",
                "UTC",
                "--log-level",
                "debug",
                "--source",
                "phone",
                "x",
            ],
            &[
                "search",
                "--page-token",
                "t",
                "--floating-zone",
                "Europe/Paris",
                "x",
            ],
            &["--store", "count", "count", "count"],
            &["count", "--", "-x"],
            &["search", "--offset", "t", "--log-file", "/tmp/log", "x"],
        ];
        for args in read {
            let line = || core::iter::once("pimdir").chain(args.iter().copied());
            let quick = Cli::parse_query(line().map(Into::into));
            let whole = Cli::try_parse_from(line());
            assert!(quick.is_some(), "{args:?}");
            assert_eq!(
                format!("{quick:?}"),
                format!("{:?}", whole.ok()),
                "{args:?}"
            );
        }

        let left: &[&[&str]] = &[
            &[],
            &["co", "x"],
            &["sea", "x"],
            &["count"],
            &["count", "--help"],
            &["search", "-V"],
            &["--version"],
            &["help", "count"],
            &["show", "1"],
            &["count", "--limit", "5", "x"],
            &["search", "--sort", "nope", "x"],
        ];
        for args in left {
            let line = core::iter::once("pimdir").chain(args.iter().copied());
            assert!(Cli::parse_query(line.map(Into::into)).is_none(), "{args:?}");
        }
    }
}
