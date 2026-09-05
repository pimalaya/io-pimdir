//! # JSON Schema registry
//!
//! Maps a command's dash-joined path, prefixed `pimdir-`, to the JSON
//! Schema of its `--json` output. [`JsonSchemaCommand`] writes one file
//! per entry, so the map is the CLI's machine-readable contract: one
//! entry per data command, none for the toolkit verbs.
//!
//! [`JsonSchemaCommand`]: pimalaya_cli::clap::commands::JsonSchemaCommand

use std::collections::BTreeMap;

use schemars::schema_for;
use serde_json::Value;

use crate::cli::{
    check::CheckOutput,
    collection::CollectionsOutput,
    export::ExportOutput,
    gc::GcOutput,
    item::{ItemExportOutput, ItemPurgeOutput, ItemRestoreOutput, ItemShowOutput, ItemsOutput},
    queue::{QueueCancelOutput, QueueOutput},
    store::StoreInfoOutput,
};

/// Builds the command-to-schema map consumed by `json-schema`.
///
/// Each value describes the type the command hands to the printer.
pub fn schemas() -> BTreeMap<String, Value> {
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
    insert!("pimdir-queue-list", QueueOutput);
    insert!("pimdir-queue-cancel", QueueCancelOutput);
    insert!("pimdir-store-info", StoreInfoOutput);
    insert!("pimdir-check", CheckOutput);
    insert!("pimdir-gc", GcOutput);
    insert!("pimdir-export", ExportOutput);

    schemas
}
