---
cairn: log
change: errors-carry-a-stable-code
date: 2026-10-30
---

# Errors carry a stable code

Every frontend stringified `PimdirError` to build its JSON codes, three relayed "delete the store and let it resync" (which loses the queue, unsent mail, retention and items kept on the device), and the public API named rusqlite and serde_json types and grew a major with every variant (report-io-pimdir.md, should-fix "Store and errors").

## What landed

- src/client.rs: `PimdirError::code() -> &'static str`, a stable kebab-case code per variant, documented in its rustdoc table: `sql`, `io`, `json`, `action-malformed`, `binding-rebind`, `placement-unnamed`, `scope-not-mail`, `collection-not-files`, `collection-synced`, `collection-not-unbound`, `item-duplicate`, `store-newer`, `store-older`, `store-uncreated`, `store-corrupt`, `store-not-current`, `store-stale`, `store-hash-algo`, `store-busy`, `store-owned`, `store-staging`, `store-in-flight`, `unsupported`, `no-performer`, `ambiguous-performer`, and with `search` `query-refused`, `tag-refused`, `index-fts5-missing`, `query-unanswerable`, `index-busy`. `Version` and `NotCurrent(Version)` above this crate's are `store-newer`, an absent `NotCurrent` is `store-uncreated`.
- Display: `Stale` and `NotCurrent` say to run or upgrade the store's sync engine, `Version` above this crate's to upgrade this program, below it to run the sync engine; src/cli/mod.rs `report` the same for `Stale`.
- `PimdirSqlError` and `PimdirJsonError`, opaque with `Error::source`, in `Sql`, `Unanswerable` and `Json`; `PimdirError` now implements `Error::source`. The `From<rusqlite::Error>` and `From<serde_json::Error>` impls stay for the crate's `?`, `#[doc(hidden)]`.
- `#[non_exhaustive]` on `PimdirError`, `PimdirSyncEvent`, `PimdirEnumerated`, `PimdirQueryErrorCode`.
- Tests: tests/queue.rs `a_store_stamped_with_a_higher_version_is_refused` (code and message), tests/stale_store.rs, tests/open_current.rs `another_schema_version_is_refused_and_not_migrated`, tests/staging.rs (`item-duplicate`).
- Capabilities moved: store ("Schema version", modified; "An error carries a stable code", "The public API names no foreign error type", "The enums consumers match grow without a major", added).

## Migration for the apps

- Every app: map JSON error codes from `err.code()` instead of the message; drop any "delete the store" advice relayed from the old `Stale` text; an exhaustive `match` on `PimdirError` needs a wildcard arm.
- himalaya src/pimdir/client.rs:144-195 (`Owned`, `NotCurrent`, `Ambiguous` matches) keep compiling; its own texts can now key on `store-owned`, `store-not-current`, `store-newer`.
- calendula src/pimdir/backend.rs:772-873 and unfog src/pimdir/backend.rs:783, src/time/place.rs:378-391: guarded matches on `Unsupported`, `Ambiguous`, `Owned`, `NotCurrent` unchanged.
- neverest: src/offline/driver.rs:220-223 (`Version`, `Stale` contexts) can rely on the new wording, and src/offline/state.rs:121, :140 still advise "let it resync" in its own text; `match` on `PimdirSyncEvent` (driver.rs:2338, :2913, :2944, :3046) needs a wildcard arm where exhaustive; `PimdirEnumerated` is only built there, unaffected.
- A caller matching `PimdirError::Sql(rusqlite::Error::…)` reads `PimdirSqlError` through `Error::source` and a downcast instead.
