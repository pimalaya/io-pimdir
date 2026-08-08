# Changelog

All notable changes to this project are documented in this file. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0/).

## [Unreleased]

### Added

- **A collection can be paged in its kind's own order.** `items` gained a `sort_key` column and an `items_by_sort` index, and `list_items_page_asc` / `list_items_page_desc` return a keyset page ordered by it: newest first for mail, A to Z for contacts, a date range for calendars. Until now the only orderings a store could serve were by `link_id` or `seq`, neither of which means anything to a reader, so every consumer had to scan a whole collection into memory to show fifty rows.

  The cursor is the `(sort_key, seq)` pair rather than the key alone, because a key is not unique: two messages share a timestamp, two contacts share a name. `seq` breaks the tie, which is what stops a page boundary that lands inside a tie from skipping an item or serving it twice. The first page takes no cursor.

  An empty key means unknown and is the default, so an item is orderable before it has been summarised: it sorts to the end of a newest-first listing and to the head of an A-to-Z one.

  `set_sort_key` restates one item's key, for a store written before its kind had a convention or a consumer whose sync engine does not carry the key inline yet. An ordinary write never resets a key it does not carry.

- **`rename_collection`**, which gives a collection a new id and carries its items, bindings, sources, queue rows and child collections with it. Every foreign key onto `collections(id)` is now `ON UPDATE CASCADE`, as is `bindings(collection, link_id)`, which is a parent one level down and refuses the cascade without it.

  This is the only safe way to change an id, and it matters because the obvious alternative is destructive: deleting a collection and recreating it under a new id cascades every item and binding away, turning a rename into a full re-download and discarding staged local changes. A server renaming a folder and an owner renaming an account both land here.

- **A spec-fidelity test suite** comparing the inlined `sql` module against the canonical pimdir specification checked out beside it: the schema semantically (columns, defaults, foreign-key actions and indexes, through SQLite's pragmas rather than by text), the presence of every canonical statement by name, and that every inlined statement prepares against the inlined schema. Statement *text* is deliberately not compared, since the specification permits an equivalent substitution; the three this crate uses are listed explicitly instead. Skips when the specification is not checked out beside this crate.

### Fixed

- **The inlined schema had drifted from the specification.** `sql::MIGRATION_0001` carried neither the `sort_key` column nor any `ON UPDATE CASCADE`, so this crate was creating stores that did not match the format it implements, and nothing detected it. The point of `sql` is to be the canonical copy a consumer runs on its own SQLite driver, so a silent disagreement is the worst failure it has; the fidelity test above exists so it cannot recur.

### Changed

- **`PimdirItem` and `PimdirRetainedItem` gained a `sort_key` field.** Breaking for anyone constructing them.

## [0.2.0] - 2026-08-07

### Added

- **A `pimdir` operator CLI**, shipped from this crate behind the `cli` feature (a `[[bin]]` with `required-features`, so a library consumer never compiles clap or any terminal dependency). It is to a store what `sqlite3` is to a database: `collection list`, `item list` (live or `--retained`, keyset-paged), `item show`, `item export`, `item restore`, `item purge` (one `seq`, `--older-than <DURATION>` or `--all`), `queue list [--parked]`, `queue cancel`, `store info`, `check [--fix]` and `export`, plus `completions` and `manuals`, each rendering as JSON under `--json` with logs on stderr.

  It **never interprets item content**: a store is kind-agnostic, so the tool prints ids, flags, levels, object hashes and the raw meta, and exports bodies byte for byte. Rendering a message or a vCard belongs to himalaya and cardamum.

  Reads open the store read-only, so inspecting a store while a sync runs is always safe. `item restore` goes through the queue as a producer and then drains that collection as the owner, reading the item back to report *applied* rather than trusting collection-wide drain counters; when a sync holds the lock the action stays queued and applies at the next drain. Purge, queue cancel and the orphan sweep have no action kind, so they take the owner role directly, and `PimdirError::Busy` reports as "another writer holds the store lock (a sync is running?)". Destructive verbs confirm (with counts and bytes when the store can price them) unless `--yes`, and refuse to prompt into a pipe or under `--json`. Listings never truncate silently.

  `check` closes two gaps the format leaves open: orphan blob files (a crash may leave one and nothing cleaned them) and refcount or reference drift. Only orphan files are reclaimable (`--fix`, guarded by a `--grace` window because a body is written before the row referencing it); drift and dangling rows are reported, never repaired.

- **The store retains items instead of deleting them.** An item whose last source binding vanishes is now soft-deleted rather than removed: `items` gained `retained_at` (RFC 3339, stamped by SQLite so the crate stays clock-free) and `retained_by`, plus a partial `items_retained` index. The row keeps its `object_hash`, so its body keeps a reference and survives garbage collection. A remote expunge therefore never destroys the local copy, which is what makes a store usable as a backup of a source it does not control. Retention is unconditional: whether a removal is terminal must read identically to every process that opens the store, so it is not configurable. How long to keep, and when to sweep, is the owner's schedule.

  `LOAD_ITEMS` hides retained rows from the sync seam. That is the condition of correctness, not an optimisation: io-replica's storage spec states that the merge reconciles only what `load` returns, so a hidden row is never re-derived, on a delta sync or on a full one. io-replica itself needed no change.

- **Purge, the only true delete.** `purge(collection, seq)` takes one retained item, `purge_retained_before(cutoff)` sweeps every item retired strictly before an RFC 3339 instant the caller computes from its own retention policy. Both release the row's object pin and let the ordinary refcount sweep unlink the body, and both refuse to touch a live item. `list_retained`, `count_retained` and `retained_bytes` are the trash view beside the live reads; `retained_bytes` is an upper bound on what a purge would reclaim, since a body a live item also points at survives.

- **A reappearing link id revives its retained row** (clearing `deleted`, `retained_at` and `retained_by`, adopting the new content, keeping the message's `seq`) instead of colliding on the primary key. One branch serves a source-side resurrection and a client restore alike, so restoring an item is an ordinary `Add` over the values retention preserved: no new action kind, no network.

- **An owner now skips the queue actions it cannot apply.** An unrecognised action kind decodes as `PimdirAction::Unknown { kind, payload, object_hash }`, payload verbatim and body still pinned, and the drain leaves the row pending (counted in `PimdirDrainReport.skipped`) instead of parking it, without blocking the actions behind it. Parking claims an action is permanently unappliable, which is wrong for an intent another owner can perform: this is what lets one queue carry store mutations any owner applies beside capability-bound intents such as a mail submission. Malformed payloads still park.

- `drop_action(id)` removes one queue row, pending or parked, releasing its object pin in the same transaction: one verb for cancelling a queued action and for acknowledging an intent performed out of band. `fail_action(id, error)` records a failed attempt, bumping `attempts` for a transient failure or parking with the reason for a permanent one.

- **The store persists a per-source content conflict.** `bindings` gained `conflicted` and `conflict_revision`, round-tripped through `ReplicaSourceBinding`, so the sync layer's memory of "this source and its own remote diverged, unresolved" survives a restart. Without it the merge re-derived the push its remote had already rejected on every run, never converging, and a client could not tell which items needed a human. Distinct from the item-level `conflicted` / `conflict_object`, which is the cross-source divergence; the two are persisted independently. Carried on `ReplicaSourceBinding` as of io-replica 0.3.0.

  The revision is meaningful only while conflicted (spec §11), so a resolved binding cannot hand a stale one to the next sync.

- A store written by an earlier draft of schema version 1 is now reconciled on open. The columns above were **folded into version 1** rather than added as version 2, the pimdir spec being still `draft`, so `PRAGMA user_version` stays `1`, which means an older store is not detectably out of date and would otherwise fail on a query much later. `init_schema` now adds any folded-in column it finds missing (and any index over one), guarded by `PRAGMA table_info` so it is a no-op for every store after the first open (spec §6's draft allowance). This machinery lapses when the spec freezes its first version.

### Changed

- **BREAKING**: bumped io-replica to `0.3`, whose `ReplicaSourceBinding` carries the per-source conflict this release persists.
- `PimdirAction::kind()` returns `&str` rather than `&'static str`, since an owner-defined kind is carried as data.

### Removed

- `PimdirActionError::UnknownKind`: an unrecognised action kind is no longer an error, so nothing can construct it.

- `sql::DELETE_ITEM`: a per-item hard delete has no caller and no counterpart in the format spec's canonical queries any more. An item no source holds is retained (`sql::RETAIN_ITEM`), and the only true deletes are `sql::PURGE_ITEM` and `sql::PURGE_RETAINED_BEFORE`.

## [0.1.0] - 2026-08-06

### Added

- Initial pimdir store: a SQLite index plus a content-addressed, two-level-sharded blob directory, implementing io-replica's storage seam (load, lookup_objects, write) for one source.
- no_std core reusable without the SQLite client: the canonical schema and statements (sql) and the model-to-column encodings (codec).
- Store-global public ids (seq): one per message, shared across every collection it is filed in, monotonic and never reused.
- Streaming blob ingest and read, so a large body is never held whole; a byteless object write indexes a body already streamed to its content-addressed path.
- Incremental, cross-collection-correct reference counting with blob garbage collection inside the write transaction; a crash leaves at worst an orphan blob, never a row without its body.
- Single-writer serialisation via BEGIN IMMEDIATE and a generous busy timeout, so several same-source handles overlap network while their writes serialise.
- An availability-aware, paginated client read surface (list_items, get_item, count_items, distinct_sources, seq_for_link) projecting the store as a local backend.
- The action queue table and collections.generation as part of the draft v1 schema, with user_version and store_meta.version kept in agreement and a store stamped with a higher schema version refused on open (the spec is a draft: draft stores are recreated, never migrated).
- The action queue (spec §14): PimdirProducer (the single enqueue transaction any non-owner process may run, pinning a pre-written body against garbage collection) and the owner's drain (drain_collection applies each action and deletes its row in one transaction, parking permanently failing actions), plus queued_collections, pending_actions (the read-your-writes overlay) and parked_actions.
- The action payload codec in the no_std core: PimdirAction (add, set-flags, remove, move, copy, update, addressing items by public seq) with a strict, versioned JSON round-trip.
- Collection generations (spec §15): the handle-space epoch on PimdirCollection and generation(), bumped atomically with a rebuild batch by write_rekeyed().
- Read-only store open (open_read_only): opens an existing store with SQLITE_OPEN_READ_ONLY, never creates anything, refuses any other schema version, and exposes the full read surface for frontend processes that must be unable to write.

[unreleased]: https://github.com/pimalaya/io-pimdir/compare/v0.2.0..HEAD
[0.2.0]: https://github.com/pimalaya/io-pimdir/compare/v0.1.0..v0.2.0
[0.1.0]: https://github.com/pimalaya/io-pimdir/compare/root..v0.1.0
