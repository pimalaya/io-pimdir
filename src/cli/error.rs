//! # Error codes
//!
//! The stable names of the failures a caller is expected to act on,
//! printed as `code` beside the message under `--json`. The message is for
//! a person and may be reworded; the code is the contract. A failure with
//! none prints `code: null`.
//!
//! A store failure takes the code io-pimdir gives it
//! ([`PimdirError::code`]), spelled the same, so a caller reads one
//! vocabulary across `pimdir` and every app over a pimdir store. The other
//! codes are this binary's own.

use std::fmt;

use io_pimdir::{client::PimdirError, search::query::PimdirQueryError};
use pimalaya_cli::error::{CodedError, PartialOutcome};
use schemars::JsonSchema;
use serde::Serialize;

/// The stable codes, each serialised in kebab-case.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorCode {
    /// No item has the `seq` asked for, or none is live in the account
    /// asked for.
    ItemNotFound,
    /// The `seq` is placed in several collections: name one with
    /// `--collection`.
    CollectionRequired,
    /// The item is live where the verb takes only a retained one, or a
    /// source still binds it.
    ItemLive,
    /// The item holds no body at its detail level.
    BodyAbsent,
    /// The item's body is missing from the blob store or fails its name:
    /// `pimdir check` reports it.
    BodyMissing,
    /// No queue row has the id asked for.
    RowNotFound,
    /// The store syncs no source, or several, and `--source` names none.
    SourceRequired,
    /// The verb destroys data and runs without a terminal to confirm on:
    /// pass `--yes`.
    ConfirmationRequired,
    /// The confirmation was declined.
    Cancelled,
    /// The page token is not one a listing printed.
    PageTokenInvalid,
    /// The export directory already holds a dump: pass `--force`.
    DumpExists,
    /// The queue row's payload is malformed.
    ActionMalformed,
    /// A write resolved a binding to a different handle.
    BindingRebind,
    /// A write carried a placement no link id names.
    PlacementUnnamed,
    /// The conflict holds a body nobody has read: fetch it first.
    ConflictUnread,
    /// A bounded sync scope was asked of a collection other than mail.
    ScopeNotMail,
    /// A file was written to a collection of another kind than files.
    CollectionNotFiles,
    /// An item was put into a collection a source syncs.
    CollectionSynced,
    /// An item was put into a collection of a kind no unbound write
    /// derives.
    CollectionNotUnbound,
    /// A role outside the collection kind's vocabulary, or an undeclared
    /// collection.
    CollectionRole,
    /// A collection holding queue rows was deleted: cancel them first.
    CollectionQueued,
    /// The collection already holds the item, or has its creation queued.
    ItemDuplicate,
    /// The store is of another draft of the format than this build's.
    StoreVersion,
    /// The store has no schema yet, or no database at all: its sync engine
    /// creates it.
    StoreUncreated,
    /// The database is not a pimdir store.
    StoreForeign,
    /// The store's version records disagree, or its schema is not the one
    /// its draft declares.
    StoreCorrupt,
    /// The store is not one the owner role opens as it stands: only its
    /// sync engine creates or upgrades it.
    StoreNotCurrent,
    /// The store names its bodies with a hash this build does not compute.
    StoreHashAlgo,
    /// The store's sync state was written by another version of its sync
    /// engine.
    SyncVersion,
    /// Another writer holds the store's write lock: retry.
    StoreBusy,
    /// A sync owns the store: retry once it ends.
    StoreOwned,
    /// A producer is staging a body in the store: retry.
    StoreStaging,
    /// A store verb is between two of its chunks: retry.
    StoreInFlight,
    /// A path of the store is not its user's alone.
    StoreExposed,
    /// A source of the store does not support the write.
    Unsupported,
    /// No source of the store can perform the intent.
    NoPerformer,
    /// Several sources of the store can perform the intent and none is
    /// chosen.
    AmbiguousPerformer,
    /// A purge cutoff is not an instant written as the store's stamps are.
    CutoffInvalid,
    /// The store cannot hold the flag: empty, or holding a space, a control
    /// or a non-ASCII character.
    FlagInvalid,
    /// The search query is malformed or names an unknown field or value:
    /// nothing was searched.
    QueryRefused,
    /// The tag names no flag, or adds and removes one flag.
    TagRefused,
    /// The linked SQLite lacks FTS5.
    IndexFts5Missing,
    /// The index could not answer an accepted query.
    QueryUnanswerable,
    /// Another indexer holds the index lock while this one has to create or
    /// rebuild the index: retry.
    IndexBusy,
    /// The store has no search index yet.
    IndexMissing,
    /// The index was built by another version.
    IndexOutdated,
    /// The index is of another draft or shape.
    IndexVersion,
    /// A zone stated for floating times or a query's dates is not an IANA
    /// zone.
    IndexZoneUnknown,
    /// A refresh was asked of an index opened for queries alone.
    IndexQueryOnly,
    /// The index refresh was resumed out of turn, an implementation fault.
    IndexRefreshBroken,
}

/// The store codes this binary relays, by io-pimdir's spelling.
const PIMDIR_CODES: &[(&str, ErrorCode)] = &[
    ("action-malformed", ErrorCode::ActionMalformed),
    ("binding-rebind", ErrorCode::BindingRebind),
    ("placement-unnamed", ErrorCode::PlacementUnnamed),
    ("conflict-unread", ErrorCode::ConflictUnread),
    ("scope-not-mail", ErrorCode::ScopeNotMail),
    ("collection-not-files", ErrorCode::CollectionNotFiles),
    ("collection-synced", ErrorCode::CollectionSynced),
    ("collection-not-unbound", ErrorCode::CollectionNotUnbound),
    ("collection-role", ErrorCode::CollectionRole),
    ("collection-queued", ErrorCode::CollectionQueued),
    ("item-duplicate", ErrorCode::ItemDuplicate),
    ("store-version", ErrorCode::StoreVersion),
    ("store-uncreated", ErrorCode::StoreUncreated),
    ("store-foreign", ErrorCode::StoreForeign),
    ("store-corrupt", ErrorCode::StoreCorrupt),
    ("store-not-current", ErrorCode::StoreNotCurrent),
    ("store-hash-algo", ErrorCode::StoreHashAlgo),
    ("sync-version", ErrorCode::SyncVersion),
    ("store-busy", ErrorCode::StoreBusy),
    ("store-owned", ErrorCode::StoreOwned),
    ("store-staging", ErrorCode::StoreStaging),
    ("store-in-flight", ErrorCode::StoreInFlight),
    ("store-exposed", ErrorCode::StoreExposed),
    ("unsupported", ErrorCode::Unsupported),
    ("no-performer", ErrorCode::NoPerformer),
    ("ambiguous-performer", ErrorCode::AmbiguousPerformer),
    ("cutoff-invalid", ErrorCode::CutoffInvalid),
    ("flag-invalid", ErrorCode::FlagInvalid),
    ("query-refused", ErrorCode::QueryRefused),
    ("tag-refused", ErrorCode::TagRefused),
    ("index-fts5-missing", ErrorCode::IndexFts5Missing),
    ("query-unanswerable", ErrorCode::QueryUnanswerable),
    ("index-busy", ErrorCode::IndexBusy),
    ("index-missing", ErrorCode::IndexMissing),
    ("index-outdated", ErrorCode::IndexOutdated),
    ("index-version", ErrorCode::IndexVersion),
    ("index-zone-unknown", ErrorCode::IndexZoneUnknown),
    ("index-query-only", ErrorCode::IndexQueryOnly),
    ("index-refresh-broken", ErrorCode::IndexRefreshBroken),
];

impl ErrorCode {
    /// The code of a store failure, `None` for one a caller cannot act on
    /// (an SQL, I/O or JSON failure).
    pub fn of_pimdir(err: &PimdirError) -> Option<Self> {
        let code = err.code();
        PIMDIR_CODES
            .iter()
            .find(|(pimdir, _)| *pimdir == code)
            .map(|(_, ours)| *ours)
    }

    /// A failure of this code with its message.
    pub fn error(self, message: impl fmt::Display) -> anyhow::Error {
        CodedError::new(self, message.to_string()).into()
    }
}

/// Turns a store error into an operator-facing one, carrying its code.
///
/// Lock contention is not a failure of the command but somebody else
/// writing, so it reads as a sentence naming the likely cause rather than
/// the library's.
pub fn report(err: PimdirError) -> anyhow::Error {
    match err {
        PimdirError::Owned(store) => ErrorCode::StoreOwned.error(format!(
            "another process owns the store at {} (a sync is running?); retry once it releases",
            store.display()
        )),
        PimdirError::Staging(store) => ErrorCode::StoreStaging.error(format!(
            "a producer is staging a body in the store at {} (a frontend is open?); \
             retry once it is done",
            store.display()
        )),
        PimdirError::Busy => ErrorCode::StoreBusy.error(
            "another writer holds the store lock (a sync is running?); retry once it releases",
        ),
        err => match ErrorCode::of_pimdir(&err) {
            Some(code) => code.error(err),
            None => err.into(),
        },
    }
}

/// What a command returns once it printed a result short of full success:
/// the exit status 2, nothing more printed.
pub fn partial() -> anyhow::Error {
    PartialOutcome::default().into()
}

/// A refused query (SEARCH §8.4) as a failure: its fault, where in the
/// query, and what would have been valid there.
pub fn refusal(query: &str, err: PimdirQueryError) -> anyhow::Error {
    let mut message = format!("query {query:?} refused: {err}");

    if !err.expected.is_empty() {
        message.push_str(&format!("; expected {}", err.expected.join(", ")));
    }

    if !err.fields.is_empty() {
        message.push_str(&format!("; fields are {}", err.fields.join(", ")));
    }

    ErrorCode::QueryRefused.error(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relayed_code_keeps_io_pimdirs_spelling() {
        for (pimdir, ours) in PIMDIR_CODES {
            assert_eq!(serde_json::to_value(ours).unwrap(), *pimdir);
        }
    }
}
