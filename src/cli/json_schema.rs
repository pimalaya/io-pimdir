//! # JSON Schema registry
//!
//! Maps a command's dash-joined path, prefixed `pimdir-`, to the JSON
//! Schema of its `--json` output. [`JsonSchemaCommand`] writes one file
//! per entry, so the map is the CLI's machine-readable contract: one
//! entry per command, the toolkit verbs included, and `pimdir-error` for
//! the failure object every command prints instead. A flag changing a
//! command's output shape has its own entry, its name appended:
//! `pimdir-search-threads` for `search --threads`.
//!
//! [`JsonSchemaCommand`]: pimalaya_cli::clap::commands::JsonSchemaCommand

use pimalaya_cli::{clap::commands::JsonSchemaRegistry, error::ErrorReport};

use crate::cli::{
    check::CheckOutput,
    collection::CollectionsOutput,
    export::ExportOutput,
    gc::GcOutput,
    item::{
        ItemEraseOutput, ItemExportOutput, ItemPurgeOutput, ItemRestoreOutput, ItemShowOutput,
        ItemsOutput,
    },
    quarantine::QuarantineEmptyOutput,
    queue::{QueueCancelOutput, QueueOutput},
    search::{
        CountOutput, IndexOutput, QueryErrorOutput, SearchOutput, SearchThreadsOutput, ShowOutput,
        TagOutput, ThreadRow,
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
    registry.insert::<ItemExportOutput>("pimdir-item-export");
    registry.insert::<ItemRestoreOutput>("pimdir-item-restore");
    registry.insert::<ItemPurgeOutput>("pimdir-item-purge");
    registry.insert::<ItemEraseOutput>("pimdir-item-erase");
    registry.insert::<QueueOutput>("pimdir-queue-list");
    registry.insert::<QueueCancelOutput>("pimdir-queue-cancel");
    registry.insert::<StoreInfoOutput>("pimdir-store-info");
    registry.insert::<CheckOutput>("pimdir-check");
    registry.insert::<GcOutput>("pimdir-gc");
    registry.insert::<QuarantineEmptyOutput>("pimdir-quarantine-empty");
    registry.insert::<ExportOutput>("pimdir-export");
    registry.insert::<IndexOutput>("pimdir-index");
    registry.insert::<SearchOutput>("pimdir-search");
    registry.insert::<SearchThreadsOutput>("pimdir-search-threads");
    registry.insert::<CountOutput>("pimdir-count");
    registry.insert::<ShowOutput>("pimdir-show");
    registry.insert::<ThreadRow>("pimdir-thread");
    registry.insert::<TagOutput>("pimdir-tag");
    registry.insert::<QueryErrorOutput>("pimdir-query-error");
    registry.insert::<ErrorReport>("pimdir-error");

    registry
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_payload_is_an_object() {
        assert_eq!(super::registry().non_object_roots(), Vec::<&str>::new());
    }
}
