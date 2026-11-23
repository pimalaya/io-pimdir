---
cairn: delta
change: errors-carry-a-stable-code
---

## MODIFIED Requirements

### Requirement: Schema version
The store schema is version 1 (`user_version` 1), migrations/storage/0001_init.sql verbatim. build.rs SHALL generate `sql::MIGRATIONS`, every canonical migration in file order, and the owner open SHALL run STORAGE §6's runner over it: read `user_version`, then for each migration above it, in order, one transaction running the script and setting `user_version`, the first one seeding `store_meta`. A store stamped above the current version is refused with `PimdirError::Version`, whose message says to upgrade this program and never to reset the store. A store stamped current by an earlier draft is reconciled by the owner open ("Open reconciles against the canonical text"); one lacking a core table or trigger the schema declares is refused with `PimdirError::Stale` naming it, by every role, whose message says to run or upgrade the store's sync engine. The table scan SHALL run before `store_meta` is read, so a store lacking `store_meta` names that table rather than failing the read of its stamp.

## ADDED Requirements

### Requirement: An error carries a stable code
`PimdirError::code()` SHALL answer a stable kebab-case code per failure, the contract a consumer maps to its own output rather than matching the message: `store-stale`, `store-newer` (a `Version` or `NotCurrent` above this crate's), `store-older`, `store-uncreated` (an `Uncreated` or an absent `NotCurrent`), `store-not-current`, `store-corrupt`, `store-hash-algo`, `store-busy`, `store-owned`, `store-staging`, `store-in-flight`, `sql`, `io`, `json`, `action-malformed`, `binding-rebind`, `placement-unnamed`, `scope-not-mail`, `collection-not-files`, `collection-synced`, `collection-not-unbound`, `item-duplicate`, `unsupported`, `no-performer`, `ambiguous-performer`, and with `search` `query-refused`, `tag-refused`, `index-fts5-missing`, `query-unanswerable`, `index-busy`. A code never changes meaning; a new failure gets a new code.

No message SHALL advise deleting the store: a resync restores neither the queue, unsent mail, retained items, references nor items kept on the device. `Stale` and `NotCurrent` say to run or upgrade the store's sync engine, `Version` above this crate's to upgrade this program; the CLI's report the same.

#### Scenario: A newer store
- GIVEN a store stamped above this crate's version
- WHEN an owner, a producer or a reader opens it
- THEN the error's code is `store-newer` and its message ends "upgrade this program", naming no reset (tests/queue.rs `a_store_stamped_with_a_higher_version_is_refused`); a store missing a core table is `store-stale`, its message ending "run or upgrade the store's sync engine" (tests/stale_store.rs)

### Requirement: The public API names no foreign error type
`PimdirError::Sql`, `Unanswerable` and `Json` SHALL wrap the opaque `PimdirSqlError` and `PimdirJsonError`, which display the cause and give it through `Error::source` without naming rusqlite or serde_json types, so a major of either is not one of this crate. The `From` conversions the crate's own `?` relies on are hidden from the documentation and are no contract.

### Requirement: The enums consumers match grow without a major
`PimdirError`, `PimdirSyncEvent`, `PimdirEnumerated` and `PimdirQueryErrorCode` SHALL be `#[non_exhaustive]`: a consumer matching one keeps a wildcard arm, and a variant the spec adds is a minor. Structs stay pub-fielded and exhaustive, built as literals.
