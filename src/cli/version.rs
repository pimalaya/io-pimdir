//! The `version` verb: the one probe telling a program which major of the
//! JSON contract this binary prints, beside the drafts it implements.
//!
//! No payload carries its own version: a program reads `formatVersion`
//! here once, and refuses a major it does not know.

use std::fmt;

use anyhow::Result;
use clap::Args;
use io_pimdir::{sql, sync::SYNC_VERSION};
use pimalaya_cli::{fields::Fields, printer::Printer};
use schemars::JsonSchema;
use serde::Serialize;

/// The major of the JSON contract: io-pimdir's own major version, so a
/// Rust API break moves it even when no payload changed.
pub const FORMAT_VERSION: u64 = match u64::from_str_radix(env!("CARGO_PKG_VERSION_MAJOR"), 10) {
    Ok(major) => major,
    Err(_) => panic!("the crate's major version is not a number"),
};

/// Print the JSON contract's version and the drafts this build implements.
///
/// `formatVersion` is the major of the JSON contract the `--json` output of
/// the reading and queueing commands follows (`search`, `count`, `show`,
/// `thread`, `tag`, `item list`, `item show`, `collection list`, `queue
/// list`, `queue status`, `store info` and the failure object): a program
/// reads it once and refuses a major it does not know. It is io-pimdir's
/// major version, so it moves with any break of the crate.
#[derive(Debug, Args)]
pub struct VersionCommand;

impl VersionCommand {
    /// Prints the versions; opens no store.
    pub fn execute(self, printer: &mut impl Printer) -> Result<()> {
        printer.out(VersionOutput {
            format_version: FORMAT_VERSION,
            version: String::from(env!("CARGO_PKG_VERSION")),
            storage_version: String::from(sql::FORMAT_VERSION),
            search_version: String::from(sql::search::FORMAT_VERSION),
            sync_version: String::from(SYNC_VERSION),
        })
    }
}

/// The `version` output.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VersionOutput {
    /// The major of the JSON contract, io-pimdir's major version.
    pub format_version: u64,
    /// The binary's version, io-pimdir's.
    pub version: String,
    /// The STORAGE draft whose stores this build opens.
    pub storage_version: String,
    /// The SEARCH draft whose index and queries this build reads.
    pub search_version: String,
    /// The SYNC draft this build's engine follows.
    pub sync_version: String,
}

impl fmt::Display for VersionOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut fields = Fields::default();

        fields.push("Format version", self.format_version);
        fields.push("Version", &self.version);
        fields.push("Storage version", &self.storage_version);
        fields.push("Search version", &self.search_version);
        fields.push("Sync version", &self.sync_version);

        write!(f, "{fields}")
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::version::*;

    /// The contract's major is the crate's.
    #[test]
    fn the_format_version_is_the_crate_major() {
        let major: u64 = env!("CARGO_PKG_VERSION")
            .split('.')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(FORMAT_VERSION, major);
    }

    #[test]
    fn the_versions_print_as_fields() {
        let output = VersionOutput {
            format_version: 1,
            version: String::from("1.2.3"),
            storage_version: String::from("storage-v1.0.0"),
            search_version: String::from("search-v1.0.0"),
            sync_version: String::from("sync-v1.0.0"),
        };

        let expected = "\
Format version:   1
Version:          1.2.3
Storage version:  storage-v1.0.0
Search version:   search-v1.0.0
Sync version:     sync-v1.0.0
";
        assert_eq!(output.to_string(), expected);
    }
}
