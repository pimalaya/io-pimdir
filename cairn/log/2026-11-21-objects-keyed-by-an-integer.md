---
cairn: log
change: objects-keyed-by-an-integer
landed: 2026-11-21
---

# Objects keyed by an integer

The crate follows pimdir 8b1523d (cairn/log/2026-11-21-objects-keyed-by-an-integer.md there), the owner's decision of 2026-11-21 on item 2 of pimdir's cairn/changes/the-write-path-before-the-freeze: storage-draft-15 and search-draft-16, edited in place, key an object by an integer. spec/ is re-vendored at 8b1523d (76 files under migrations/ and queries/, no vector, payload nor generator file moved) and spec/REVISION names it.

- **The store**: `objects.id INTEGER PRIMARY KEY AUTOINCREMENT`, the hash unique on the row alone (`objects_by_hash`); `items.object` (was `object_hash`), `items.conflict_object`, `bindings.base_object`, `conflict_object`, `shared_object` and `queue.object` (was `object_hash`) hold the id. The canonical statements keep the hash as their parameters and answered columns, so no Rust signature moved and the crate's API still names a body by its hash.
- **The index**: `object.id` drawn by `AUTOINCREMENT`, `placement.object` (was `hash`) the body's `object.id`, sought by `placement_by_object`.
- **The crate's own statements** join by the ids: src/sql.rs `LIVE_BYTES`, `COUNT_RETAINED_BEFORE`, `REFCOUNT_DRIFT`, `COUNT_BODILESS`, and `DANGLING_ITEM_OBJECTS` and `DANGLING_QUEUE_OBJECTS`, which now name a pointer at no object row as `#` and its id (no row holds its hash; `DANGLING_BINDINGS` names a gone item the same way); src/sql/search.rs `LOAD_PLACEMENT` (the hash read off the `object` row, `NULL` once a pass deleted it, as `placements_gone` answers) and `MESSAGE_HASH` (joining the `object` row); src/client/search/compile.rs, the planner: the text-term probe reads `object_text` by `rowid = p.object` and the `temp.term_source` probe by `s.id = p.object`, neither reading an `object` row, `category:` joins `object_category.object = p.object`, `has:` and `encrypted` `o.id = p.object`, and `has:body` and the date set read `items.object`.
- **A store of the previous text** is refused by every profile as `Damaged` naming `objects` (the schema check compares the text): tests/stale_store.rs `a_store_keying_objects_by_hash_is_damaged`, which fails against the previous schema (it names `objects_count_collect` there). An index of the previous text is rebuilt by the indexer, refused as `IndexOutdated` by a query client until then, as any index of another schema text.
- **Tests**: the raw SQL of tests/refcount_property.rs (the reachable set and the recount, by id, each pointer read back as its row's hash or `#` and the id for one naming no row), tests/generated/replay.rs (`unheld`), tests/generated/index.rs (the dump, a placement's body as its row's hash, `gone` for an id naming no row, since two indexes draw different ids), tests/vectors_sync.rs (the items and bindings rows, every pointer as its hash), tests/search.rs (the missing feed and the model comparison) and tests/store_text.rs. tests/pending_vectors.txt names nothing, as before.

Measured on the Enron tier x1, core 12 idle, a fresh import by this tree's pimbench, against io-pimdir a135cac's run (x1 now holds 518,401 messages in 3,312 folders, the 1,000 incr.sh added, against 517,401 in 3,311 then):

- Import: sync 690 s wall (718), 158 s CPU (177), 36.5 GB written (51.5); index 184 s wall (252), 111 s CPU (123), 25.8 GB written (28.8). pimdir.db 889 MB (1,052), index.db 1,931 MB (1,981).
- Queries, ms, count / page of 50 / threaded page of 50 (hyperfine medians of 10 after 2 warm-ups, process start included): `enron` 516 / 24 / 33 (947 / 26 / 35), `california` 58 / 39 / 50 (106 / 46 / 64), `power california` 121 / 71 / 84 (220 / 89 / 105), `subject:meeting` 34 / 35 / 44 (60 / 42 / 54).

One run each, no A/A check. The `enron` count fell by 431 ms, about the 403 ms pimdir's simulation saved on reading its placements (477 to 74 ms at 517k).

A trap met on the way: FTS5 reads `rowid = NULL` as no bound on the rowid (`fts5GetRowidLimit` falls back to the full range for a non-integer), so the planner's per-candidate probe, joined by `rowid = p.object`, matched every body for a bodiless placement; it now keeps those out first (`p.object IS NOT NULL`). tests/search.rs `every_search_vector_answers` caught it under the forced walk.

Capabilities moved: store.md (a new requirement, a new scenario, the column names), search.md (a new requirement), spec-fidelity.md (the revision). The Android app follows with its own index statements (pimdir's log lists them).
