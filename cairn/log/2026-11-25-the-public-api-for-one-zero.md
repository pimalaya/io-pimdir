---
cairn: log
change: the-public-api-for-one-zero
landed: 2026-11-25
---

# The public API for 1.0

Section 6 of pimdir's freeze plan (pimdir cairn/changes/the-freeze-plan/tasks.md, "io-pimdir's public API"), on the inventory of 2026-11-25: every top-level item and public method, each counted by grep in himalaya, cardamum, calendula, neverest, carillon and android/rust, the CLI and tests/. The lead's rulings applied as given; the judgement calls are below.

## What landed

- Visibility: 57 items and 12 methods made crate-private, none a consumer, a test, a doc test or a public signature names; 37 crate-own statements (`sql`'s 14, `sql::search::OWN`'s 23) crate-private, their text still given by name through `sql::all` and `sql::search::all`. The compiler's `private_interfaces` lint drove it: every item a public signature names went back to public. `#![cfg_attr(not(all(feature = "search", feature = "indexer")), allow(dead_code))]` in src/lib.rs keeps a partial feature set from flagging what only `client` or `search` reads, the full set still checking dead code.
- Removed: `search::plan::is_undated` (no use), `PimdirPlanScope::admits` and `PimdirPushRights::none` (dead once private).
- Renamed: `search::chunk::Chunks` to `PimdirChunks`; `load::PimdirRefusal` to `load::PimdirBindingRefusal` (a refusal standing on a binding, apart from `capability::PimdirRefusal`). `PimdirQueryErrorCode` and `PimdirPushResult` kept: `Code` names the target (the code a `PimdirQueryError` carries, a SEARCH §8.4 token), and `Push` + `Result` already is verb + Ext.
- Re-exports gone: `client::run` is public and `PimdirRunError` is named there; `PimdirSearchCursor` and `PimdirSearchSort` are named by `search::plan`, which owns them.
- `#[non_exhaustive]`: added to `PimdirQueryErrorCode`; kept on `PimdirWaitReason`, which neverest matches (src/sync/driver/itemize.rs:645, a wildcard arm), and added to the requirement; removed from `PimdirPushOutcome`, which consumers build and only a neverest test matches.
- `pending_arrivals` lists a move or copy whose payload does not decode as `PimdirAction::Unknown` (`PimdirQueueRow::listed`, shared with `pending_actions`). tests/queue.rs `a_malformed_arrival_is_listed_opaque` failed on a `git archive` snapshot of 769dfba (the listing's `unwrap` panicked) and passes now.
- `PimdirSourceStore::register(collections)`: neverest's own connection (src/sync/storage.rs:143-182, `register_syncing`) as an owner write: `ensure_collection`, `load_checkpoint`, `upsert_checkpoint` with no checkpoint, in one `begin` transaction. No new statement: the canonical ones answer it.
- `PimdirStore::for_source` answers a `Result`, an empty id refused as `PimdirError::SourceEmpty` (`source-empty`); the 190 call sites under tests/ and the three of src/cli follow.
- Docs: the provider names in src/remote.rs, src/summary/mail.rs and src/placement.rs replaced by the behaviour class; links to items now private unlinked (`cargo doc` with `-D rustdoc::private_intra_doc_links` clean).
- CI: an `msrv` job on Rust 1.89 (`--all-features`, `--no-default-features --features client`), the consumers' feature sets (`cargo check`, `client`, `client,search,indexer`), and `cargo semver-checks` against 1.0.0, run once crates.io answers for it.

## Judgement calls

- Kept public though no consumer names them: the Annex B vocabulary in `capability` and `intent` (`CONTACTS_CARD_UPDATE`, `CONTACTS_CARD_REMOVE`, `CALENDAR_ITEM_UPDATE`, `CALENDAR_ITEM_COPY`, `CALENDAR_ITEM_REMOVE`, `intent::CALENDAR_REPLY`, `CALENDAR_CANCEL`), a registry with holes being worse than four unused names; `security::path_component` (STORAGE §18, what a consumer naming a file from an attachment's name runs); the limits public docs cite (`CHUNK_MAX`, `OCCURRENCES_MAX`, `PERIODS_MAX`, `INSTANCES_MAX`, `LENGTH_MAX`, `GROUPS_MAX`, `TERMS_MAX`, `BODIES_MAX`, `HEADER_MAX`, `LINE_HEAD`, `ATTACHMENTS_MAX`); what a spec requirement states public: `summary::DERIVATION` (store.md), the header parses of `summary::mail` (summaries.md), `hasher` on the store, reader and producer (store.md), `PimdirHub::{project_with, offered, folded}` (hub.md, `PimdirLoaded::folded`), `PimdirLinkId::{landed_hint, is_minted, minted_from, is_landable}` (sync.md), `PimdirMail::read_meta` (search.md), `PimdirSummary::stored` (tests/engine).
- Kept public, test-only: the reader duties tests/generated/reader.rs maps (`PimdirAddressCursor`, `PimdirMailEntry`, `PimdirMailSum`, `like_pattern`), `TIME_COLLECTION` (§14.3), the Annex A walk and its limits (`summary::mail::attachments`, `MIME_DEPTH_MAX`, `MIME_ENTITIES_MAX`, `hash_key`, `strip_instructions`, `unschedule`, `part_key`), `chunk::chunks` (SEARCH §6), `PimdirOpen` (one of the five verbs). `PimdirPlanForce` and `PimdirIndex::force_driver` stay `#[doc(hidden)]` test seams, out of semver-checks' reach: moving tests/search.rs (4,625 lines) and tests/search_plan.rs into the crate was out of proportion. tests/search.rs no longer builds a `PimdirPlan` to skip a forced driver a query has no set for: the plan answers the walk then, so the run is redundant, never wrong.
- The canonical statements generated from spec/queries stay public constants: the module promises them by name to an implementation binding its own SQLite, and naming 190 of them private would need an allowlist in build.rs.
- In the files other lanes edit, only attributes: src/search/query.rs's `is_relative` is `#[expect(dead_code)]` and `first_day`, `day_after` `#[cfg(test)]` rather than removed; `pending_arrivals` in src/client/producer.rs was the one logic change there, as asked.
- The semver job checks for 1.0.0 on crates.io rather than for a version of at least 1.0.1: Cargo.toml stays 1.0.0 after the release until the next one is prepared, so a version test would skip the commits right after 1.0.0.
- The CLI relays no `source-empty`: its sources come from stored rows, which the schema keeps non-empty.

## Left

- After the lanes merge: remove `PimdirQueryDate::is_relative` and gate or remove `first_day`, `day_after` in src/search/query.rs.
- src/rekey.rs:789, a human inline comment, names Graph.
- `cargo test` with the default features fails to build the lib tests (src/client/schema.rs:421 names `sql::search` without `search`), as before; CI never runs that set.
- Consumers move to the renamed and re-homed items and to `for_source(..)?`; neverest replaces `register_syncing` by `register`.

[store](../spec/store.md): new *The public API is what a consumer or a spec duty needs*; *The enums consumers match grow without a major* names `PimdirWaitReason` and `PimdirPushOutcome`; *A handle names a source only where an operation acts as one* gains the `source-empty` refusal and `register`; *Pending actions are readable* gains `pending_arrivals`; the code list gains `source-empty`; the version judgement is crate-internal.
