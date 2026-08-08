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
inside the transaction and their blob files unlinked only after commit. An item
the batch leaves held by no source is **retained, not deleted**, and keeps its
object references pinned. The
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

### Requirement: A collection records the account it belongs to
`collections` SHALL carry `account` (TEXT, nullable): an opaque owner-chosen id,
`NULL` in a single-account store. A partial index `collections_by_account ON
collections(account) WHERE account IS NOT NULL` SHALL back the by-account reads,
so a single-account store writes no account and pays for no index.

`collections.id` SHALL remain unique store-wide. An owner filing several
accounts in one store namespaces their collection ids; the column makes that
grouping queryable rather than replacing it. The store SHALL NOT interpret the
value: it is neither parsed, nor validated, nor matched against configuration.

`PimdirStore` and `PimdirProducer` SHALL each carry an optional account, `None`
by default and set by `for_account`, and SHALL bind it on every collection they
create, including the lazy `ENSURE_COLLECTION` inside the write seam and the
producer's enqueue transaction. A multi-account owner opens one handle per
account. This SHALL NOT change the single-owner rule: how many handles a process
holds is unrelated to how many processes may own a store.

`ENSURE_COLLECTION` and `SET_COLLECTION_KIND` SHALL bind `:account`, and neither
SHALL overwrite an existing one, so a collection cannot change account as a side
effect of a sync declaring its media type. Regrouping is the deliberate
`set_collection_account`, safe at any time because the account partitions no
identifier.

### Requirement: The account partitions no identifier
Link ids, hashes and `seq`s SHALL keep their store-wide meaning whatever account
a collection belongs to. The `seq` lookup SHALL remain unscoped: two placements
sharing a `link_id` share a `seq` whether or not they sit in the same account,
because the `seq` is the short form of the link id and the link id is equal.
Object deduplication SHALL likewise stay store-wide: one body reaching two
accounts is one object, refcounted per placement.

### Requirement: Multiplicity is reported, never resolved
The store SHALL expose where one identity or one body occurs across every
collection and account, and SHALL take no position on what that means:

- `link_placements(link_id)` returns each live placement of one link id with its
  collection, account, `seq`, object, flags and level.
- `object_placements(hash)` returns the same by body, with `link_id` in place of
  the object, pairing placements two servers gave different link ids.

Both SHALL exclude tombstones and retained rows, and SHALL order by account then
collection with the `NULL` account first. Merging, hiding or pairing placements
is the consumer's decision, and the store SHALL implement none of them: a mail
view lists every placement, a contact view may offer to merge them, and both
read the same rows.

### Requirement: A body may be ingested and emitted by streaming
The store SHALL be able to persist an object from a byte stream (`Read`),
computing its content hash incrementally, with the same temp → fsync → rename
durability as a buffered write, so a large body is never held whole; and it SHALL
expose a stored object as a readable stream for the same reason on the read side.

### Requirement: A byteless object write indexes an already-stored blob
A `StoreObject` carrying no bytes — its blob already persisted by a streaming
fetch under its content-addressed path — SHALL record the object row and refcount
without writing bytes. Refcounting and garbage collection are unchanged.

### Requirement: The client read surface exposes accounts
`list_collections` SHALL return `account` alongside the existing columns.
`list_collections_by_account(account)` SHALL return one account's collections in
the same shape, matching with `IS` so binding `None` selects the collections of
a single-account store. `list_accounts` SHALL return the accounts owning at
least one collection.

Because the store learns an account only through its collections, `list_accounts`
is not a configured roster: an account with no collection does not appear, and a
consumer needing the full roster reads it from its own configuration. The store
records which account a collection belongs to and nothing else about it: no
credentials, no endpoints, no display name.

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

Every item a read returns SHALL carry its `sort_key` alongside the columns above.

These reads are kind-agnostic (raw `meta`, string flags, opaque object hash) and
observe only — they never mutate; all writes remain io-replica `ReplicaWriteOp`s
through `write`.

### Requirement: A collection pages in its kind's own order
`list_items_page_asc` and `list_items_page_desc` SHALL return a keyset page
ordered by `(sort_key, seq)` (spec §9.3): ascending is A to Z for contacts and
earliest first for mail and calendars, descending is the reverse. `list_items`
keeps its `link_id` ordering and is the sweep page, for a pass that must see
every item exactly once.

The cursor SHALL be the `(sort_key, seq)` pair, since a sort key is not unique
and `seq`, unique per collection, is what makes the page total: no item skipped
or repeated across a boundary. The first page SHALL be requestable with no
cursor, so a caller never invents a sentinel above every representable key.

#### Scenario: A limit splits a tie
- GIVEN three items sharing one sort key and a page limit of two
- WHEN the collection is paged to exhaustion in either direction
- THEN each item is returned exactly once

### Requirement: An ordinary write preserves an item's ordering key
A `write` SHALL leave an existing item's `sort_key` alone. The save is diffed
rather than replace-all, and the update statement names no `sort_key`, so a key
survives every sync that does not deliberately restate it. Were it otherwise,
ordering would be reset on every pass and a consumer restating keys afterwards
would race its own sync indefinitely.

`set_sort_key(collection, link_id, sort_key)` SHALL restate one item's key, for a
store written before its kind had a convention, one whose convention changed, or
a consumer whose sync engine does not carry the key inline and derives it from
the `meta` it wrote itself.

### Requirement: A collection can be renamed without losing its contents
`rename_collection(collection, new_id)` SHALL give a collection a new id and
carry its items, bindings, sources, queue rows and child collections with it, by
way of `ON UPDATE CASCADE` on every foreign key onto `collections(id)` **and** on
`bindings(collection, link_id)`, which is a parent one level down and refuses the
cascade without it.

This SHALL be the only id change offered. Deleting a collection and recreating it
under a new id destroys the cache, since `ON DELETE CASCADE` takes every item and
binding with it, turning a rename into a full re-download and discarding staged
local changes.

#### Scenario: A server renames a folder
- GIVEN a synced collection holding items and a binding per item
- WHEN it is renamed
- THEN the items and their bindings follow, so the next sync is a delta rather
  than a re-download

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
key. This holds across accounts too: equal link ids share a `seq` wherever they
sit, which reports their equality without asserting the placements are one
thing.

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
later actions, queryable, and never silently deleted. An action the owner cannot
apply at all (a kind it does not recognise, or one it recognises but lacks the
capability to perform) is **skipped and left pending**, never parked, so another
owner can perform it.

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

### Requirement: A binding's unresolved conflict is persisted
The `bindings` table SHALL carry `conflicted` (INTEGER, `0`/`1`) and
`conflict_revision` (TEXT, nullable), and the store SHALL round-trip both
through `ReplicaSourceBinding`. `conflict_revision` SHALL be written and read as
meaningful only while `conflicted` is set, so a resolved binding cannot hand a
stale revision to the next sync.

This is distinct from the item-level `conflicted` / `conflict_object`, which
records a cross-source divergence; a store SHALL persist the two independently.
Without the binding pair the sync layer loses its memory of an unresolved
conflict across a restart, re-derives the push its remote already rejected on
every run, and never converges.

### Requirement: A store from an earlier draft of the current version is reconciled on open
While the pimdir spec is `draft`, a schema change MAY be folded into version 1
rather than added as a new version (spec §6). A store written by an earlier
draft is then stamped with the current `user_version` yet lacks the folded-in
columns, so the version check alone cannot detect it.

On open, the store SHALL reconcile its shape: every folded-in column found
missing SHALL be added (`ALTER TABLE … ADD COLUMN`, which requires the column to
be nullable or carry a constant default), guarded so the check is a no-op for an
up-to-date store, together with any index over a folded-in column. Failing a
later query on a missing column is not acceptable. This requirement lapses when
the spec leaves `draft` and versions are frozen.

### Requirement: An item is retained, never deleted, when its last binding goes
`items` SHALL carry `retained_at` (TEXT, RFC 3339, nullable) and `retained_by`
(TEXT, nullable). When a write batch leaves an item held by no source, the store
SHALL **retire** the row rather than delete it: `deleted` set, `retained_at`
stamped by SQLite (`strftime('%Y-%m-%dT%H:%M:%fZ','now')`, so the crate needs no
clock), `retained_by` set to the source whose removal retired it, and
`object_hash` kept. The item's now source-less bindings SHALL be deleted with it,
so a retained row carries `deleted = 1` and no binding at all: the persisted form
of a removal that has finished propagating.

`retained_at` records when the **last binding vanished**, not when a server
deleted the item (unknowable). A revive clears it, so restore-then-redelete
restarts the clock.

A retained row SHALL pin its bodies: retiring compensates the object references
the hub diff released, so `object_hash` and `conflict_object` keep their
refcount and garbage collection never sweeps a retained body. Revive and purge
release that pin.

### Requirement: Retained rows are hidden from the sync seam
`LOAD_ITEMS` SHALL exclude retained rows (`retained_at IS NULL`), so a retained
item is absent from `load_hub`, from `load` and from every projection. This is
io-replica's "hiding rows from load is safe": the merge reconciles only what
`load` returns, so a retained item is never re-derived, re-added or re-pushed,
on a delta sync or a full resync. It is likewise absent from the live client
read surface, which already filters `deleted`.

### Requirement: Purge is the only true delete
The store SHALL expose the retained set and the only operation that destroys
data:

- `list_retained(collection, after, limit)` SHALL return a keyset page of a
  collection's retained items (`seq > after`, ordered by `seq`, at most `limit`),
  each carrying its `seq`, `link_id`, flags, level, raw `meta`, object hash,
  object size, `retained_at` and `retained_by`.
- `count_retained(collection)` SHALL count a collection's retained items, and
  `retained_bytes()` SHALL total the distinct object sizes the store's retained
  items hold, an upper bound on what a purge reclaims (a body a live item also
  points at survives).
- `purge(collection, seq)` SHALL delete one **retained** row, reporting whether
  it existed; a live item is never purged through it.
- `purge_retained_before(cutoff)` SHALL delete every retained row across the
  store whose `retained_at` is **strictly before** the caller's RFC 3339 cutoff
  (an item retained exactly at the cutoff is kept), reporting the items removed
  and the bytes reclaimed.

Both purges SHALL release the retained row's object pin and let the existing
refcount and garbage collection path unlink the blob once nothing references it;
there is no second collector. Retention itself is unconditional: *whether* a
delete is terminal must be identical for every opener, so only *when* to reclaim
is the caller's policy.

### Requirement: A reappearing link id revives its retained row
An item inserted while a retained row holds the primary key
`(collection, link_id)` SHALL revive that row: `deleted`, `retained_at` and
`retained_by` cleared, the new content adopted, the message's `seq` kept (ids are
never reused). One branch serves both a source-side resurrection and a
client-staged `add`, so restoring a retained item needs no new action kind: `Add`
over the values the row still holds is a restore. A duplicate-link-id check on
the apply path SHALL exempt retained rows.

### Requirement: An owner skips the actions it cannot apply
An action kind the store does not recognise SHALL decode as an opaque action
(kind, raw payload, and the body hash the payload pins) rather than an error, so
one queue can carry store mutations any owner applies beside capability-bound
intents (a mail submission) only a specific owner can perform. Genuinely
malformed payloads (not JSON, no supported `v`) SHALL still park.

The drain SHALL **skip** a row it cannot apply: the row stays pending, is never
parked (parking means permanently unappliable), and never blocks later actions
in the same collection. The drain report SHALL count skips beside applies and
parks.

### Requirement: A queued action can be cancelled or acknowledged
`drop_action(id)` SHALL delete one queue row, pending or parked, releasing its
object pin in the same transaction, and report whether the row existed. It
serves both cancelling a queued action and acknowledging an intent an owner
performed out of band. `fail_action(id, error)` SHALL record a failed attempt:
`None` bumps `attempts` and leaves the row pending (transient), `Some(error)`
parks it (permanent). A collection's pending actions SHALL expose each row's
`id`, since callers act on rows by id.

### Requirement: The canonical SQL is reachable by name
`sql` SHALL expose `ALL`, a `&[(&str, &str)]` pairing every statement constant's
name with its text, `MIGRATION_0001` included and `VERSION` excluded. A consumer
without the `client` feature, holding its own SQLite driver, SHALL be able to
recover any statement from it by name without a per-statement accessor.

The index SHALL be covered by tests derived from the module's own source: one
asserting every declared constant is indexed, one asserting the index follows the
declaration order. Two statements MAY legitimately carry identical text under
different names (`DELETE_ACTION` and `CANCEL_ACTION` are one delete under two
intents), so text uniqueness SHALL NOT be asserted.
