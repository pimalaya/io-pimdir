//! # JSON Schema registry
//!
//! Maps a command's dash-joined path, prefixed `pimdir-`, to the JSON
//! Schema of its `--json` output. [`JsonSchemaCommand`] writes one file
//! per entry, so the map is the CLI's machine-readable contract: one
//! entry per data command, none for the toolkit verbs, and `pimdir-error`
//! for the failure object every command prints instead. A flag changing a
//! command's output shape has its own entry, its name appended:
//! `pimdir-search-threads` for `search --threads`.
//!
//! [`JsonSchemaCommand`]: pimalaya_cli::clap::commands::JsonSchemaCommand

use std::collections::BTreeMap;

use pimalaya_cli::{clap::commands::JsonSchemaRegistry, error::ErrorReport};
use schemars::schema_for;

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
    let mut schemas = BTreeMap::new();

    macro_rules! insert {
        ($key:expr, $ty:ty) => {
            schemas.insert(
                $key.to_string(),
                serde_json::to_value(schema_for!($ty)).unwrap(),
            );
        };
    }

    insert!("pimdir-collection-list", CollectionsOutput);
    insert!("pimdir-item-list", ItemsOutput);
    insert!("pimdir-item-show", ItemShowOutput);
    insert!("pimdir-item-export", ItemExportOutput);
    insert!("pimdir-item-restore", ItemRestoreOutput);
    insert!("pimdir-item-purge", ItemPurgeOutput);
    insert!("pimdir-item-erase", ItemEraseOutput);
    insert!("pimdir-queue-list", QueueOutput);
    insert!("pimdir-queue-cancel", QueueCancelOutput);
    insert!("pimdir-store-info", StoreInfoOutput);
    insert!("pimdir-check", CheckOutput);
    insert!("pimdir-gc", GcOutput);
    insert!("pimdir-quarantine-empty", QuarantineEmptyOutput);
    insert!("pimdir-export", ExportOutput);
    insert!("pimdir-index", IndexOutput);
    insert!("pimdir-search", SearchOutput);
    insert!("pimdir-search-threads", SearchThreadsOutput);
    insert!("pimdir-count", CountOutput);
    insert!("pimdir-show", ShowOutput);
    insert!("pimdir-thread", ThreadRow);
    insert!("pimdir-tag", TagOutput);
    insert!("pimdir-query-error", QueryErrorOutput);
    insert!("pimdir-error", ErrorReport);

    JsonSchemaRegistry {
        bin: "pimdir",
        version: env!("CARGO_PKG_VERSION"),
        schemas,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_payload_is_an_object() {
        assert_eq!(super::registry().non_object_roots(), Vec::<&str>::new());
    }
}
