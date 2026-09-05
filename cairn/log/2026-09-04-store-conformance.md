---
cairn: log
change: store-conformance
date: 2026-09-04
---

# The std client meets the revised standard

The standard revised on 2026-09-04 was re-vendored and the client read against it. The load now derives a tombstone's destination through `destination_for_link` and reads a `Handles` scope's probes through `load_probes_by_handle`; the write resolves every upserted handle and retires a binding whose handle moved to another key, releasing bindings before it inserts any under the now unique index; the drain parks a store failure, retries an environment one, skips a foreign remove and a lost claim, and `enqueue` takes the body whole; the overlay skips a malformed row; the blob write syncs every shard directory and the collector runs under a per-store writer lock, unlinking only files at their shard path; the migration runner is STORAGE §6's, a store lacking `store_meta` is stale and a missing database is uncreated; the `hash:` key's FNV prime was wrong by four bits and is pinned by the vector now; `windows-1252` decodes by its table and a folded header keeps its whitespace; `Auto` resolves from the source list; and every collection parameter is `impl AsRef<str>`.

store.md, spec-fidelity.md and summaries.md moved with it, store.md losing the `sql::ALL`, `open_read_only`, `write_rekeyed` and bytes-reclaimed sentences that described nothing.
