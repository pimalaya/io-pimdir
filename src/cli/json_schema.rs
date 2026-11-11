//! # JSON Schema registry
//!
//! Maps a command's dash-joined path, prefixed `pimdir-`, to the JSON
//! Schema of its `--json` output. [`JsonSchemaCommand`] writes one file
//! per entry, so the map is the CLI's machine-readable contract: one
//! entry per command, the toolkit verbs included, and `pimdir-error` for
//! the failure object every command prints instead. A command printing
//! one of two shapes is described by both, through [`OneOf`], their keys
//! telling them apart.
//!
//! [`JsonSchemaCommand`]: pimalaya_cli::clap::commands::json_schema::JsonSchemaCommand

use pimalaya_cli::{
    clap::commands::json_schema::JsonSchemaRegistry, error::ErrorReport, output::Bytes,
};
use schemars::JsonSchema;

use crate::cli::{
    check::CheckOutput,
    collection::CollectionsOutput,
    error::ErrorCode,
    export::ExportOutput,
    gc::GcOutput,
    item::{
        ItemEraseOutput, ItemExportOutput, ItemPurgeOutput, ItemRestoreOutput, ItemShowOutput,
        ItemsOutput,
    },
    quarantine::QuarantineEmptyOutput,
    queue::{QueueCancelOutput, QueueOutput, QueueStatusOutput},
    search::{
        CountOutput, IndexOutput, SearchOutput, SearchThreadsOutput, ShowOutput, TagOutput,
        ThreadRow,
    },
    store::StoreInfoOutput,
};

/// Builds the command-to-schema registry consumed by `json-schema`.
///
/// Each value describes the type the command hands to the printer, and
/// `pimdir-error` the failure object every command prints instead.
pub fn registry() -> JsonSchemaRegistry {
    let mut registry = JsonSchemaRegistry::new("pimdir", env!("CARGO_PKG_VERSION"));

    registry.insert::<CollectionsOutput>("pimdir-collection-list");
    registry.insert::<ItemsOutput>("pimdir-item-list");
    registry.insert::<ItemShowOutput>("pimdir-item-show");
    // NOTE: the receipt of a body written to `--output`, else the body.
    registry.insert::<OneOf<ItemExportOutput, Bytes>>("pimdir-item-export");
    registry.insert::<ItemRestoreOutput>("pimdir-item-restore");
    registry.insert::<ItemPurgeOutput>("pimdir-item-purge");
    registry.insert::<ItemEraseOutput>("pimdir-item-erase");
    registry.insert::<QueueOutput>("pimdir-queue-list");
    registry.insert::<QueueCancelOutput>("pimdir-queue-cancel");
    registry.insert::<QueueStatusOutput>("pimdir-queue-status");
    registry.insert::<StoreInfoOutput>("pimdir-store-info");
    registry.insert::<CheckOutput>("pimdir-check");
    registry.insert::<GcOutput>("pimdir-gc");
    registry.insert::<QuarantineEmptyOutput>("pimdir-quarantine-empty");
    registry.insert::<ExportOutput>("pimdir-export");
    registry.insert::<IndexOutput>("pimdir-index");
    // NOTE: `hits`, or under `--threads` the `threads` they collapse to.
    registry.insert::<OneOf<SearchOutput, SearchThreadsOutput>>("pimdir-search");
    registry.insert::<CountOutput>("pimdir-count");
    registry.insert::<ShowOutput>("pimdir-show");
    registry.insert::<ThreadRow>("pimdir-thread");
    registry.insert::<TagOutput>("pimdir-tag");
    registry.insert::<ErrorReport<ErrorCode>>("pimdir-error");

    registry
}

/// The schema of a command printing one of two types, told apart by their
/// keys.
#[derive(JsonSchema)]
#[serde(untagged)]
#[allow(dead_code)]
enum OneOf<A, B> {
    A(A),
    B(B),
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use crate::Cli;

    use super::*;

    /// Every runnable command is described, and every key names one: a
    /// renamed subcommand cannot leave a schema nobody can ask for, nor a
    /// new one print undescribed data.
    #[test]
    fn the_registry_matches_the_command_tree() {
        let problems = registry().check(&Cli::command(), &[]);
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn every_payload_is_an_object() {
        assert_eq!(registry().non_object_roots(), Vec::<String>::new());
    }
}
