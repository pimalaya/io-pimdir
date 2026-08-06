---
cairn: spec
capability: store
status: current
---

# Store

A pimdir store is a SQLite index (`pimdir.db`, `STRICT` tables, schema version in
`user_version`) plus a content-addressed blob directory. It implements
io-replica's storage seam (`load` / `lookup_objects` / `write`) for one source,
and is the portable, cross-implementation form of that seam (the pimdir spec).

### Requirement: A write batch is one transaction
`write` SHALL apply its `ReplicaWriteOp` batch as a single SQLite transaction:
object bytes are written to the blob file (temp → fsync → rename) before the row
that references them; placement upserts/drops fold into the collection's hub and
are saved **by diffing the loaded hub against the absorbed one, touching only the
items and bindings that changed** (never a whole-collection delete-and-reinsert);
**object refcounts are maintained incrementally, applying only the per-hash
difference between the hub's object references before and after the batch** (never
a global recompute); zero-refcount objects are collected, their rows dropped
inside the transaction and their blob files unlinked only after commit. The
incremental refcount is cross-collection correct: a batch adjusts a hash's count
by this collection's change alone, leaving other collections' references counted.
The write SHALL be O(changed rows), not O(collection size), so an incremental
sync that changed a handful of items does not rewrite the whole mailbox.
A crash SHALL leave at worst an orphan blob, never a row without its body.
The transaction SHALL begin with `BEGIN IMMEDIATE`, taking the store's single
writer lock (SPEC §7) up front: under WAL readers never block, concurrent writers
serialise on the busy timeout, and a writer that cannot acquire the lock within it
SHALL fail with a clear `PimdirError::Busy` rather than a raw SQL error or a
failure deep inside the batch. The busy timeout SHALL be generous enough (30s) to
let a single process fan work across several same-source handles — one per worker,
to overlap network while the writes serialise — without a burst of large writes
tripping `Busy`. Coordinating who writes (one owning process, or a front daemon
fronting a UI and a sync) is a platform decision, not enforced here.

### Requirement: Blobs are content-addressed and sharded
An object's bytes SHALL live at `objects/<hash[0:2]>/<hash[2:4]>/<hash>`,
immutable once written, so an identical body delivered twice is stored once.
`PimdirBlobs` reads a blob independently of the SQLite connection, so a body can
be read while the store is mutably borrowed to service a sync.

### Requirement: Several sources share one store
A store MAY be opened as several source handles (`"left"`, `"right"`, …) over the
same files; each services the seam for its own source, and the shared database is
the multi-source hub. `load_hub` reads a collection's whole hub (every source's
bindings) for a consumer that projects each side.

### Requirement: Collections declare a media type
A collection SHALL carry a `kind` (an IANA media type). `ensure_collection` sets
it; the lazy collection creation inside `write` uses `ON CONFLICT DO NOTHING` and
never clobbers a declared kind. This makes the store self-describing and lets one
store hold several item kinds.

### Requirement: A body may be ingested and emitted by streaming
The store SHALL be able to persist an object from a byte stream (`Read`),
computing its content hash incrementally, with the same temp → fsync → rename
durability as a buffered write, so a large body is never held whole; and it SHALL
expose a stored object as a readable stream for the same reason on the read side.

### Requirement: A byteless object write indexes an already-stored blob
A `StoreObject` carrying no bytes — its blob already persisted by a streaming
fetch under its content-addressed path — SHALL record the object row and refcount
without writing bytes. Refcounting and garbage collection are unchanged.

### Requirement: A client reads the store by indexed, paginated getters
The store SHALL expose a read-only query surface for a client projecting the
store as a local backend, distinct from the sync seam's load-all:

- `list_collections` SHALL return every collection's `id`, `kind`, `name`,
  `parent`, `color`, `description` and `sort_order`.
- `list_items` SHALL return a page of a collection's **live** items (`deleted =
  0`), keyset-paginated by `link_id` (`link_id > after`, ordered by `link_id`,
  at most `limit`), each carrying its public `seq`, its `link_id`, flags, raw
  `meta`, object hash and detail `level`.
- `get_item` SHALL return one live item by its public id `(collection, seq)`, or
  nothing; `seq_for_link` SHALL resolve the inverse (`link_id` → `seq`).
- `count_items` SHALL return a collection's live item count.

These reads are kind-agnostic (raw `meta`, string flags, opaque object hash) and
observe only — they never mutate; all writes remain io-replica `ReplicaWriteOp`s
through `write`.

### Requirement: Items carry a message-scoped public id
Each item SHALL carry a `seq`: an integer id a consumer shows and accepts in place
of the internal `link_id`. It is a property of the **message**, not the placement:
a message filed in several mailboxes (the same `link_id`) SHALL keep the **same**
`seq` in every one, so a merged / cross-mailbox view shows it once and ids never
clash between mailboxes. The store SHALL assign a message's `seq` the first time it
inserts an item with that `link_id` (in any collection) — drawing from the
**store-global** `store_meta.next_seq` counter — and reuse it for every later
placement of the same `link_id`. The counter only ever increases, so a `seq` is
**never reused** even after the message is deleted everywhere. `(collection, seq)`
SHALL be unique (one placement per message per collection). The sync seam still
keys on `link_id`; `seq` is assigned transparently on insert and is never a sync
key.

### Requirement: A client can discover the store's sources
The store SHALL expose the distinct source names it has synced against (across all
collections) via `distinct_sources`, so a client can attribute its writes to a
source without configuration — a store synced as a single source returns exactly
one. This is a kind-agnostic read; it never mutates.

### Requirement: A reader can open the store read-only
`PimdirStore::open_read_only(dir, source)` SHALL open an existing store with
`SQLITE_OPEN_READ_ONLY`: it never creates the schema (that is the owner's
opening write), and refuses a schema version other than the current one with
the version error. The returned handle exposes the full read surface; any write
through it fails at the SQLite layer.

### Requirement: Reads are availability-aware
A read result SHALL carry each item's detail `level` (`Probed`/`Meta`/`Full`), so
a caller knows a body is not local (`level < Full`, `object` absent) without
probing the blob store, and can trigger a hydrate through the sync engine rather
than treating the absence as data loss.

### Requirement: Schema version
The store schema is version 1 (`user_version` 1) and includes the `queue` table
and `collections.generation`: the spec is a draft, so there is no earlier schema
and no upgrade path. An owner open creates the schema in a fresh database and
refuses a store stamped with a higher `user_version` with `PimdirError::Version`;
a draft store stamped otherwise is recreated, never migrated.

### Requirement: Producers append, only the owner pops
The store SHALL support the pimdir action queue: any process may act as a
producer whose sole write is the single enqueue transaction (ensure_collection,
at most one object upsert pinning a pre-written blob, one queue insert). Only the
owner SHALL read-and-remove queue rows: each pending action is applied to items
and bindings and its row deleted in the same transaction, so application is
exactly-once and never partially visible. Failing actions accumulate `attempts`;
permanently failing actions are parked with `error` set, skipped without blocking
later actions, queryable, and never silently deleted.

### Requirement: Queued bodies are pinned
An object referenced by a pending queue row's `object_hash` SHALL count as
referenced under the incremental refcount scheme, so garbage collection never
sweeps a body between enqueue and apply. The pin is taken at enqueue and released
when the row is deleted, with the applied item's own reference taken in the same
transaction.

### Requirement: Collection generation is the handle-space epoch
`collections.generation` SHALL start at 1 and be bumped only by the owner, in the
same transaction as a handle-space rebuild (rekey). It SHALL be exposed on the
read surface so frontends derive epoch-dependent protocol values (an IMAP
UIDVALIDITY) from the store alone. Ordinary syncs, full resyncs from an expired
checkpoint, and content changes never bump it.

### Requirement: Pending actions are readable
The read surface SHALL expose a collection's pending (non-parked) actions in
append order, so a frontend can overlay them on its item projection for
read-your-writes.

> Initial seed spec (Cairn adopted 2026-07-31): captures the store's core
> guarantees; further capabilities may be spelled out as they are touched.
