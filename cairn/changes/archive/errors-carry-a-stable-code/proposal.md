---
cairn: change
id: errors-carry-a-stable-code
status: landed
created: 2026-10-30
---

# Errors carry a stable code

## Why

The release audit of 2026-10-30 (report-io-pimdir.md, should-fix "Store and errors" and "API and release engineering"):

- `PimdirError::Stale`'s message and the CLI's `report` (src/client.rs:1327, src/cli/mod.rs:172) advise to "delete the store and let it resync", which calendula, unfog and himalaya relay. A resync cannot restore the queue, unsent mail, retained trash, references, items kept on the device or unfog's time history;
- every app stringifies `PimdirError` to map its JSON codes, each its own way;
- `PimdirError::Version` (a newer store) must say upgrade, never reset;
- `PimdirError::Sql(rusqlite::Error)`, `Unanswerable(rusqlite::Error)` and `Json(serde_json::Error)` tie the public API to those crates' majors;
- no `#[non_exhaustive]`, so every variant the spec adds is a major.

## What

- `PimdirError::code() -> &'static str`: stable kebab-case codes (`store-stale`, `store-newer`, `store-uncreated`, `store-not-current`, `store-busy`, `store-owned`, `unsupported`, and the rest), documented as the contract the apps map to JSON.
- Display: no "delete the store"; `Stale` and `NotCurrent` say to run or upgrade the store's sync engine; `Version` says to upgrade this program; the CLI's `report` the same.
- `#[non_exhaustive]` on the enums apps match but never build: `PimdirError`, `PimdirSyncEvent`, `PimdirEnumerated`, `PimdirQueryErrorCode` (and `PimdirArgError` if the review agrees). Structs stay pub-fielded; 0.x minors break.
- Opaque wrappers in place of `rusqlite::Error` and `serde_json::Error` in public variants, the source kept behind `Error::source`.
- The five open drafts touching the API (an-owner-write-checks-its-fence, a-push-reads-back-the-server-bytes, an-ambiguous-intent-awaits-confirmation, raw-payloads-print-their-length, a-delta-loads-what-it-names) land in the same release only as the user decides; this change does not presume them.
