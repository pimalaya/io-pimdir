---
cairn: delta
change: store-conformance
---

## ADDED Requirements

### Requirement: A load derives a tombstone's destination and reads its own probes
`load` SHALL set a `Tombstone` placement's origin to the collection `DESTINATION_FOR_LINK` names, under the tombstone's own handle, so the engine derives `Remove { to }` from it (SYNC §3); a `Created` placement's origin stays `ORIGIN_FOR_LINK`'s. A `Handles` scope SHALL read its probes with `LOAD_PROBES_BY_HANDLE` bound to the handles asked for (STORAGE §14).

### Requirement: A handle names one item per source
`write` SHALL resolve every upserted handle with `LINK_FOR_HANDLE`, and a handle bound to a different link id SHALL retire that binding first, in the same transaction, exactly as a `Deleted` drop of the handle would (STORAGE §10). The diff SHALL release bindings before it inserts any, since `bindings_by_handle` is unique.

### Requirement: A drain has three outcomes per row
An `Err` from applying one row SHALL NOT stop the pass unless it is `Busy` or an I/O failure, which bump `attempts` and stop it; every other failure parks the row (STORAGE §15.2). A `remove` of a live item the draining source does not bind SHALL skip; only an absent item is success. A claim that deletes nothing SHALL count as skipped, never as applied. `enqueue` SHALL take the body's `PimdirObject`, hash and size together.

### Requirement: An overlay survives a malformed pending row
A reader built with `with_pending` SHALL skip a pending row whose payload it cannot decode; the drain parks it.

### Requirement: The collector is serialised against the process's own writers
Every shard directory a blob write creates SHALL be synced, root included. The lock registry SHALL hold a per-store `RwLock` writers hold shared from blob staging through commit and the collector holds exclusively across its row deletion and its file walk (STORAGE §8). The collector SHALL unlink only files named like an object under the store's algorithm, leaving a period-prefixed temporary and any foreign file alone (STORAGE §3).

### Requirement: The migration runner
build.rs SHALL generate `MIGRATIONS`, every migration in file order, and the owner open SHALL apply each one above `user_version` in order, setting `user_version` after each (STORAGE §6). A store lacking a canonical table SHALL be `Stale` before `store_meta` is read, so a store lacking `store_meta` names it. A reader or a producer opening a directory with no pimdir.db SHALL be `Uncreated`.

### Requirement: One collection-id type
Every read and write taking a collection SHALL take `impl AsRef<str>`; `PimdirCollectionId` implements it. An unknown role in `item_address` SHALL be skipped by the address reads, never mapped to `from`. Every single-statement owner write SHALL map SQLITE_BUSY to `PimdirError::Busy`. `read_hub` SHALL propagate a serialisation failure.

### Requirement: The delete policy `Auto` reads the binding count
When the engine carries `PimdirDeletePolicy::Auto`, `sync` SHALL resolve it to `Keep` when `LIST_SOURCES` names more than one source and to `Revert` otherwise, before handing the options to the engine (SYNC §5).

## MODIFIED Requirements

### Requirement: An ordinary write preserves an item's ordering key
The sentence "the update statement names no `sort_key`" goes: `update_item` binds it, and the diff rule below is what keeps a key.

### Requirement: The store has a reader role of its own
The `open_read_only` paragraph goes: the method was removed.

### Requirement: Purge is the only true delete
`purge_retained_before` reports the rows removed and never bytes.

### Requirement: A store never collects itself
`write_rekeyed` goes from the list of writes.

### Requirement: The constants are generated, never written (spec-fidelity)
`sql::OWN` holds `DELETE_DANGLING_BINDINGS`, `OBJECT_SIZE` and `COUNT_RETAINED_BEFORE` beside the diagnostics.

### Requirement: Decoding follows Annex A.0 (summaries)
`windows-1252` decodes by its table; a charset the crate cannot decode reads its bytes as UTF-8, lossily, a known deviation; a folded header keeps its whitespace (RFC 5322 §2.2.3).

## REMOVED Requirements

### Requirement: The canonical SQL is reachable by name
`sql::ALL` and the tests derived from the module's own source no longer exist; spec-fidelity.md describes `CANONICAL`, `OWN` and `all()`.

### Requirement: The generated schema is semantically identical to the canonical one (spec-fidelity)
The pragma comparison and its test went: the vendored copy is the specification's byte for byte, which says more.
