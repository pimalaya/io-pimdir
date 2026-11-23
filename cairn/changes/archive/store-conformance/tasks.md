---
cairn: tasks
change: store-conformance
---

- [x] `hash_key`: the FNV-1a 64 prime; tests/summaries.rs compares the pinned `hash:` key.
- [x] `load`: a Tombstone's destination from `DESTINATION_FOR_LINK`; a Handles scope reads `LOAD_PROBES_BY_HANDLE`.
- [x] `write`: every upserted handle resolved, a handle bound to another link id retired first; bindings released before bindings inserted.
- [x] Drain: a store failure parks, an environment failure retries and stops; an unbound live `remove` skips; a lost claim is not applied; `enqueue` takes the object whole.
- [x] Overlay: an undecodable pending row is skipped.
- [x] Blobs: every shard directory synced; the collector under the process's writer lock; the walk leaves foreign names alone.
- [x] Schema: the table scan before `store_meta`; `MIGRATIONS` generated and run in order; a missing file is `Uncreated`.
- [x] Delete policy `Auto` resolved from the binding count, once the engine carries it.
- [x] One collection-id type; unknown roles skipped; busy mapped on single statements; `read_hub` propagates a serialisation error.
- [x] Summaries: windows-1252 decoded by its table; folded whitespace kept.
- [x] store.md, spec-fidelity.md, summaries.md folded; the log entry written.
