---
cairn: spec
capability: store
status: current
---

# Store

A pimdir store is a SQLite index (`pimdir.db`, `STRICT` tables, schema version in
`user_version`) plus a content-addressed blob directory: the storage part of the
pimdir standard, whose schema and statements it inlines verbatim. It services the
engine's seam (`load` / `lookup_objects` / `write`) for one source (see [seam](seam.md)).

### Requirement: A write batch is one transaction
`write` SHALL apply its `PimdirWriteOp` batch as a single SQLite transaction:
object bytes are written to the blob file (temp → fsync → rename) before the row
that references them; placement upserts/drops fold into the collection's hub and
are saved **by diffing the loaded hub against the absorbed one, touching only the
items and bindings that changed** (never a whole-collection delete-and-reinsert);
**object refcounts are maintained incrementally, applying only the per-hash
difference between the hub's object references before and after the batch** (never
a global recompute). The batch reclaims nothing: see the collector below. An
item the batch leaves held by no source is **retained, not deleted**, and keeps
its object references pinned. The
incremental refcount is cross-collection correct: a batch adjusts a hash's count
by this collection's change alone, leaving other collections' references counted.
The write SHALL be O(changed rows), not O(collection size), so an incremental
sync that changed a handful of items does not rewrite the whole mailbox.
A crash SHALL leave at worst an orphan blob, never a row without its body.
The transaction SHALL begin with `BEGIN IMMEDIATE`, taking the store's single
writer lock (STORAGE §8) up front: under WAL readers never block, concurrent writers
serialise on the busy timeout, and a writer that cannot acquire the lock within it
SHALL fail with a clear `PimdirError::Busy` rather than a raw SQL error or a
failure deep inside the batch. The busy timeout SHALL be generous enough (30s) to
let a single process fan work across several same-source handles, one per worker,
to overlap network while the writes serialise, without a burst of large writes
tripping `Busy`. Which process writes is not left to a convention: see the
advisory lock below.

### Requirement: One owner, enforced, and failing fast
A store SHALL have at most one owner process at a time, and every owning handle
SHALL take an exclusive advisory lock on the store directory (STORAGE §8) for its
lifetime. The lock lives on the open file description, so the kernel releases it
when the process dies: a crashed owner leaves a lock file that locks nothing, and
no stale-lock recovery is needed or offered.

An owner that cannot take the lock SHALL fail immediately with
`PimdirError::Owned`, naming the store. It SHALL NOT wait: a daemon syncing a
large mailbox holds its transaction long enough that a bounded wait is neither a
wait nor a failure, just a stall with no signal, and retrial policy (retry, back
off, queue the intent, tell the user) belongs to the program on top.

The rule is about processes, so several handles of one process SHALL share one
lock: opening one handle per source, or one per account, is one owner. A
read-only handle SHALL take no lock, and any number of readers MAY run against a
store an owner holds.

A producer is not an owner. It SHALL take a *shared* lock on the store directory
for its handle's lifetime, so several producers run at once and none of them
keeps the owner out. What that lock delimits is the window a collector must not
run inside: a body is written to the blob tree before the queue row that pins it,
and between the two it is a file nothing references.

`PRAGMA busy_timeout` and `PimdirError::Busy` are unchanged by this: owners and
producers still contend at the SQLite layer, and that contention is worth waiting
out.

### Requirement: Blobs are content-addressed and sharded
An object's bytes SHALL live at `objects/<hash[0:2]>/<hash[2:4]>/<hash>`, immutable once written, so an identical body delivered twice is stored once. `PimdirBlobs` reads a blob independently of the SQLite connection, so a body can be read while the store is mutably borrowed to service a sync. It SHALL be read-only: `get`, `reader`, `path`, `files` and the hash, with no writer, so a handle that holds no lock (a reader's, or one opened from a path) cannot put a body in the tree.

`get` SHALL check the bytes it read against their name (STORAGE §7): a file that does not hash to its name answers `None`, the state of a missing one, so no parser, index, gate or drain is handed corrupt content. `reader` SHALL answer a `PimdirBlobReader` hashing what it streams and failing the read that reaches the end of the file with `ErrorKind::InvalidData` when the bytes do not hash to the name, so a streamed upload or export never hands on corrupt content as sound; its `metadata` and `len` size the body first. A read of part of a body is not verified. The store's own streamed reads (the verification before a push, the attachment walk) read a body failing its name as none.

A body SHALL be written only through `PimdirBlobStage`, which a producer (`PimdirProducer::stage_body`, `blob_writer`) or an owner (`PimdirStore::blob_stage`) hands out and which holds that handle's lock for as long as it lives: a producer's shared staging lock, an owner's owner lock (STORAGE §8). A body written through it is therefore never in the window a collector runs in.

### Requirement: A handle names a source only where an operation acts as one
`PimdirStore::open(dir)` and `PimdirReader::open(dir)` SHALL take no source. The handle they return serves what an operation means for the store as a whole: every client read, retention and its purges, the queue rows a cancellation or an acknowledgement removes, and the collection generations. None of those consult a source, and none SHALL require one to be named.

`for_source(source)` SHALL yield the source-bound handle (`PimdirSourceStore`), which carries the seam (`load` / `lookup_objects` / `write`), the driver that runs the verbs against a `PimdirRemote`, and the queue drain: the operations that mean "as this side". It SHALL dereference to the source-less handle, so the store-wide surface stays reachable through it, and a caller that named no source SHALL NOT be able to reach the source-bound one.

No API SHALL invent a source name to satisfy a constructor. A store an operator reads, sweeps and purges therefore records no source, and `distinct_sources` on it stays empty.

### Requirement: Several sources share one store
A store MAY be opened as several source handles (`"left"`, `"right"`, …) over the
same files, each made by binding an open store to a source; each services the seam
for its own source, and the shared database is the multi-source hub. `load_hub`
reads a collection's whole hub (every source's bindings) for a consumer that
projects each side.

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
The store SHALL be able to persist an object from a byte stream (`PimdirBlobWriter`, `Write`), computing its content hash incrementally under the store's algorithm, with the same temp, fsync, rename durability as a buffered write, so a large body is never held whole: `commit` answers the `PimdirObject` (hash and size) the bytes written name, so a body is never filed under a hash its caller computed apart. It SHALL expose a stored object as a readable stream for the same reason on the read side. The stream (`PimdirBlobReader`) SHALL verify the body as the bytes read reach the length the file stated when opened (`len`), and at the end of the file before it, failing the read with `InvalidData` on a mismatch, and SHALL fail a read of a byte past that length, so a consumer reading a known length is verified as one reading to the end (tests/store_rules.rs `a_streamed_body_read_to_its_length_is_verified`, which failed before: the read of exactly `len` corrupted bytes answered them).

### Requirement: A byteless object write indexes an already-stored blob
A `StoreObject` carrying no bytes, its blob already persisted by a streaming
fetch under its content-addressed path, SHALL record the object row and refcount
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
  object hash and detail `level`; `list_summaries` and `get_item` join the kind's
  summary row and its addresses.
- `get_item` SHALL return one live item by its public id `(collection, seq)`, or
  nothing; `seq_for_link` SHALL resolve the inverse (`link_id` → `seq`).
- `count_items` SHALL return a collection's live item count.

Every item a read returns SHALL carry its `sort_key` alongside the columns above.

These reads observe only and never mutate; every write is a `PimdirWriteOp` batch
through `write`.

### Requirement: A collection pages in its kind's own order
`list_items_page_asc` and `list_items_page_desc` SHALL return a keyset page
ordered by `(sort_key, seq)` (STORAGE §9.3): ascending is A to Z for contacts and
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
A `write` SHALL leave an existing item's `sort_key` alone unless the batch
carries a new one (STORAGE §9.3). The save is diffed rather than replace-all
and an unknown key never replaces a known one, so a key survives every sync
that does not deliberately restate it. Were it otherwise, ordering would be
reset on every pass and a consumer restating keys afterwards would race its
own sync indefinitely.

`set_sort_key(collection, link_id, sort_key)` SHALL restate one item's key, for a
store written before its kind had a convention, one whose convention changed, or
a consumer restating keys after a convention changed.

### Requirement: A collection can be renamed without losing its contents
`rename_collection(collection, new_id)` SHALL give a collection a new id and
carry its items, bindings, sources, queue rows and child collections with it, by
way of `ON UPDATE CASCADE` on every foreign key onto `collections(id)` **and** on
`bindings(collection, link_id)`, which is a parent one level down and refuses the
cascade without it. The rename SHALL also retarget the queued `move` and
`copy` rows naming the old id (`RENAME_QUEUE_TARGETS`, STORAGE §15), in the same
transaction, and the canonical trigger restamps the collection's items for the
change feed.

This SHALL be the only id change offered. Deleting a collection and recreating it
under a new id destroys the cache, since `ON DELETE CASCADE` takes every item and
binding with it, turning a rename into a full re-download and discarding staged
local changes.

#### Scenario: A server renames a folder
- GIVEN a synced collection holding items and a binding per item
- WHEN it is renamed
- THEN the items and their bindings follow, so the next sync is a delta rather
  than a re-download

### Requirement: A collection can be deleted
`delete_collection(collection)` SHALL remove a collection the operator no longer wants, `DELETE_COLLECTION` cascading over its items, bindings and sources with retention not applying, then `RECOMPUTE_REFCOUNTS` in the same transaction, since the cascade releases pins no statement returns (STORAGE §14). It answers whether a row went. A collection holding a pending or parked queue row SHALL be refused as `PimdirError::Queued { collection }` (code `collection-queued`), nothing deleted: `collections_keep_queued` refuses the delete, which would drop the action with no receipt; the operator cancels the rows first.

#### Scenario: A queued action holds its collection
- tests/store_rules.rs `a_collection_holding_a_queued_action_is_not_deleted` (pending, then parked, then dropped and deleted)

### Requirement: Items carry a message-scoped public id
Each item SHALL carry a `seq`: an integer id a consumer shows and accepts in place
of the internal `link_id`. It is a property of the **message**, not the placement:
a message filed in several mailboxes (the same `link_id`) SHALL keep the **same**
`seq` in every one, so a merged / cross-mailbox view shows it once and ids never
clash between mailboxes. The store SHALL assign a message's `seq` the first time it
inserts an item with that `link_id` (in any collection), drawing from the
**store-global** `store_meta.next_seq` counter, and reuse it for every later
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
source without configuration, a store synced as a single source returns exactly
one. This is a kind-agnostic read; it never mutates.

### Requirement: The store has a reader role of its own
A consumer that only reads SHALL be able to hold a handle that can only read.
`PimdirReader::open(dir)` SHALL open an existing store with
`SQLITE_OPEN_READ_ONLY`: it never creates the schema (that is the owner's
opening write), and refuses a schema version other than the current one with
the version error. It takes no lock (STORAGE §8), so any number of readers run
against a store an owner is syncing and none of them waits.

The handle SHALL carry the read surface alone: it SHALL NOT expose the drain,
the object sweep, a purge, a retention write, or an enqueue. `PimdirStore`
SHALL dereference to one, so a reader and an owner run the same statements and
the two roles never disagree about what the store holds.

A reader or a producer opening a directory holding no pimdir.db SHALL report
`PimdirError::Uncreated`, the same refusal as an empty database: neither role
creates the schema, and a missing file is not a SQLite failure to relay.

### Requirement: Reads are availability-aware
A read result SHALL carry each item's detail `level` (`Meta`/`Full`), so
a caller knows a body is not local (`level < Full`, `object` absent) without
probing the blob store, and can trigger a hydrate through the sync engine rather
than treating the absence as data loss.

### Requirement: Schema version
The store schema is version 1 (`user_version` 1), migrations/storage/0001_init.sql verbatim. build.rs SHALL generate `sql::MIGRATIONS`, every canonical migration in file order, and the owner open SHALL run STORAGE §6's steps in order, nothing else changing the schema:

1. Read `user_version`. A store above `sql::VERSION` SHALL be refused as `PimdirError::Version { found }` (code `store-newer`, its message saying to upgrade this program) before anything is written, the journal mode included. A database at version 0 that declares any table is not a pimdir store and SHALL be refused as `PimdirError::Foreign` (code `store-foreign`), as SHALL one whose `store_meta.format` is not `pimdir`. A store lacking a core table or trigger is refused as `PimdirError::Stale` by every role, the table scan running before `store_meta` is read. The declared algorithm (`open_with_hash`) is checked here, so a mismatch is `PimdirError::HashAlgo` with nothing reconciled.
2. A version 1 store is reconciled to 0001_init.sql alone ("Open reconciles against the canonical text"), before any migration; no other version is reconciled.
3. Each migration above the store's version runs in its own transaction, which also sets `user_version` and, through the canonical `set_store_version`, `store_meta.version`; a fresh database runs `init_store_meta` (`:version`, `:hash_algo`, `:derivation`) in the transaction applying 0001_init.sql.

`PRAGMA journal_mode = WAL` is set only after step 1 passed, by the owner, `open_current` and the producer alike, since it writes the database header.

### Requirement: The two schema stamps must agree
`PRAGMA user_version` and `store_meta.version` mirror one another (STORAGE §4.2), so a store where they differ is corrupt rather than a store at either version. Every role SHALL compare `store_meta.version` with the store's own `user_version`, not with this crate's version, and refuse a disagreement with `PimdirError::VersionMismatch`.

A store carrying no `store_meta` row SHALL be left alone: the row is seeded by whoever created the schema, and refusing there would make a missing stamp unrepairable.

### Requirement: An unknown flag set is stored as NULL
The `flags` column keeps two absences apart (STORAGE §13): `NULL` means nothing has
read the item's markers, `'[]'` means it is known to carry none. The store SHALL
write `NULL` for an unknown set and decode `NULL` back to one, so a placement
whose markers were never read never claims to carry none.

In a queue payload an unknown set SHALL encode as `null` rather than `[]`, since
an action states an intent: every payload the format defines carries a known
set, and an unknown one must not read as a deliberate clearing of every flag.

### Requirement: Producers append, only the owner pops
The store SHALL support the pimdir action queue: any process may act as a
producer whose sole write is the single enqueue transaction (ensure_collection,
the object upsert of a pre-written blob, `PIN_OBJECT`, the queue insert, in that
order, so the refcount invariant holds at every statement, STORAGE §15.1).
`enqueue` SHALL take the body the action names as a `PimdirObject`, hash and
size together, so a first-seen body cannot be pinned without its row; a body
the store already indexes MAY be left out. Only the owner SHALL read-and-remove
queue rows: each pending action is applied to items and bindings and its row
deleted in the same transaction, so application is exactly-once and never
partially visible.

The drain is store-wide: `drain` reads `LIST_PENDING_ACTIONS` across every
collection in append order (STORAGE §15.2), a per-collection drain breaking the
order a `move` into one collection followed by an edit there relies on, and
`PimdirPendingAction` names its `collection`. A drain has three outcomes per
row. A failure of the store (a refused rebind, a constraint, a malformed
payload, a `move` or `copy` into a collection of no declared kind) is permanent
for the row and SHALL park it, `error` set, the attempt counted, the rows behind
it proceeding. A failure of the environment (`PimdirError::Busy`, an I/O
failure, SQLite busy or locked) SHALL bump `attempts`, leave the row pending and
proceed to the next row, the report counting it `retried`; a row reaching
`PimdirProducer::MAX_ATTEMPTS` is parked instead. The handle draining is not the
source an action is for: an action against an existing item SHALL run as a
source binding it, this handle's own where it does and else the first that
does, and an `add` as a source syncing the collection (`COLLECTION_SOURCES`),
this handle's own where nothing does yet, so a store-wide drain never binds a
source to a collection it does not sync. A `remove` of an item that is absent
is success. A claim that deletes nothing
(`CLAIM_ACTION`) is a row another handle applied or a cancellation removed, and
SHALL count as skipped, never as applied. Parked rows are queryable and never
silently deleted.

An action the owner cannot apply at all (a kind it does not recognise, or one it
recognises but lacks the capability to perform) is **skipped and left pending**,
never parked, so another owner can perform it.

#### Scenario: A drain by a source that holds no binding
- GIVEN a queued flag change and a queued `add` on a collection one source binds
- WHEN another source's handle drains the queue first
- THEN both apply as the binding source, which binds the create, and the
  draining source binds nothing there

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
read-your-writes, and the store's as a whole (`list_pending_actions`), each
naming its collection.

### Requirement: A reader may overlay a collection's pending actions
A reader built with `with_pending` SHALL project a collection's pending actions
over its committed items, in row `id` order, so a consumer sees its own staged
writes before the owner applies them. The fold SHALL cover the kinds that
address an existing item: `set-flags`, `remove`, `move`, `copy` and `update`.
Each keeps the item's public id, a `seq` following the link id store-wide, so
the overlay SHALL NOT invent an identifier.

A pending `add` SHALL NOT be projected as an item: it has no `seq` until the
owner applies it, and it is a request to create an item rather than one. The
reader SHALL report pending creates apart, as rows and as a count.

An `update` stating `if_match` SHALL fold only when it names the body the item holds at that point of the fold, the edits folded before it included: one naming another will park at the drain (STORAGE §15.3), so the overlay shows the item as the drain will leave it (tests/overlay.rs `an_edit_that_will_park_is_not_folded`).

A parked row SHALL NOT overlay, its error asserting that it will not be applied
without an operator. A pending row whose payload the reader cannot decode
SHALL be skipped by the overlay, never fail the read: the drain is what parks
it, and the strict `pending_actions` read still reports it as an error.

The overlay SHALL be chosen when the reader is built rather than per call, so
one handle cannot answer two ways about one collection.

An overlaid page SHALL keep the meaning an unoverlaid one has: it comes back
short only where the collection ends. A staged removal drops a row the statement
returned, so the fold SHALL read past the limit by the number of pending
removals and cut back to the limit afterwards, rather than returning a page
shortened in the middle of a collection. A caller therefore pages until a short
page, as it always did, and a whole-collection scan written that way does not end
early.

> Initial seed spec (Cairn adopted 2026-07-31): captures the store's core
> guarantees; further capabilities may be spelled out as they are touched.

### Requirement: A binding's unresolved conflict is persisted
The `bindings` table SHALL carry `conflicted` (INTEGER, `0`/`1`) and
`conflict_revision` (TEXT, nullable), and the store SHALL round-trip both
through `PimdirBinding`. `conflict_revision` SHALL be written and read as
meaningful only while `conflicted` is set, so a resolved binding cannot hand a
stale revision to the next sync.

The table SHALL additionally carry `conflict_object` (TEXT, nullable,
referencing `objects(hash)`), round-tripped through `PimdirBinding` and,
like `conflict_revision`, meaningful only while `conflicted` is set. It holds
the diverging remote body at the revision beside it, so that resolution can read
base, local and remote from the store alone.

This is distinct from the item-level `conflicted` / `conflict_object`, which
records a cross-source divergence; a store SHALL persist the two independently.
Without the binding pair the sync layer loses its memory of an unresolved
conflict across a restart, re-derives the push its remote already rejected on
every run, and never converges.

#### Scenario: The three bodies are readable without a remote
- GIVEN a conflicted binding written by a sync
- WHEN it is read back after a restart
- THEN its base, its local body and its diverging body are all readable from the store

### Requirement: An item is retained, never deleted, when its last binding goes
`items` SHALL carry `retained_at` (TEXT, RFC 3339, nullable) and `retained_by`
(TEXT, nullable). When a write batch leaves an item held by no source, the store
SHALL **retire** the row rather than delete it, unless another collection of the
same account holds the identity live: that is a move, or one of two filings, and
the row is retired then purged in the same transaction (`HELD_ELSEWHERE` bound
to the retiring body, so a holder of another body or none keeps the row,
`PURGE_ITEM`, `release_pins`), the holder pinning the body and the purge counting
in `store_meta.purges` (STORAGE §11). Otherwise the retirement is: `deleted` set, `retained_at`
stamped by SQLite (`strftime('%Y-%m-%dT%H:%M:%fZ','now')`, so the crate needs no
clock), `retained_by` set to the source whose removal retired it, and
`object_hash` kept. The item's now source-less bindings SHALL be deleted with it,
so a retained row carries `deleted = 1` and no binding at all: the persisted form
of a removal that has finished propagating.

`retained_at` records when the **last binding vanished**, not when a server
deleted the item (unknowable). A revive clears it, so restore-then-redelete
restarts the clock.

The trash (`list_retained`, `count_retained`) SHALL list every `deleted = 1`
row, a tombstone a source that may not push still holds included, so a delete
nothing can deliver is visible rather than lost (STORAGE §11); `PimdirRetention::at`
is `None` until the last binding goes.

A retained row SHALL keep its bodies pinned: the hub keeps an unbound item, so the
diff releases only the bindings' references and the row's own stay counted, and
garbage collection never sweeps a retained body. Revive and purge release the row's
pins.

### Requirement: Retained rows are hidden from the sync seam
`LOAD_ITEMS` SHALL exclude retained rows (`retained_at IS NULL`), so a retained
item is absent from `load_hub`, from `load` and from every projection. This is
SYNC §3's "a retained item is projected for nobody": the merge reconciles only what
`load` returns, so a retained item is never re-derived, re-added or re-pushed,
on a delta sync or a full resync. It is likewise absent from the live client
read surface, which already filters `deleted`.

### Requirement: Purge is the only true delete
The store SHALL expose the retained set and the only operation that destroys
data:

- `list_retained(collection, after, limit)` SHALL return a keyset page of a
  collection's retained items (`seq > after`, ordered by `seq`, at most `limit`),
  each carrying its `seq`, `link_id`, flags, level, object hash, object size,
  `retained_at` and `retained_by`, and its summary and addresses.
- `count_retained(collection)` SHALL count a collection's retained items, and
  `retained_bytes()` SHALL total the distinct object sizes the store's retained
  items hold, an upper bound on what a purge reclaims (a body a live item also
  points at survives).
- `purge(collection, seq)` SHALL delete one **retained** row, reporting whether
  it existed; a live item is never purged through it.
- `purge_retained_before(cutoff)` SHALL delete every retained row across the
  store whose `retained_at` is **strictly before** the caller's RFC 3339 cutoff
  (an item retained exactly at the cutoff is kept), reporting the rows removed
  and never bytes, which the collector frees.

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
the apply path SHALL exempt retained rows. A revive over an incoming placement
carrying no body SHALL keep the retained body at `Full`, an immutable kind's
bases adopting it (STORAGE §11.1): a `Meta` fetch reviving a row must not
discard what the store still holds.

### Requirement: An owner skips the actions it cannot apply
An action kind the store does not recognise SHALL decode as an opaque action (kind, raw payload, and the body hash the payload pins) rather than an error, so one queue can carry store mutations any owner applies beside capability-bound intents (a mail submission) only a specific owner can perform. A payload stating a version `v` this owner does not know, for any kind, SHALL be skipped the same way (STORAGE §15.2). A payload that is not JSON, or states no `v`, SHALL still park.

The drain SHALL **skip** a row it cannot apply: the row stays pending, `error` `NULL` and `attempts` unbumped, is never parked (parking means permanently unappliable), and never blocks later actions in the same collection. The drain report SHALL count skips beside applies and parks.

#### Scenario: A later producer's payload
- tests/store_rules.rs `an_unknown_payload_version_is_skipped_not_parked`

### Requirement: A queued action can be cancelled or acknowledged
`drop_action(id)` SHALL delete one queue row, pending or parked, releasing its
object pin in the same transaction, and report whether the row existed. It
withdraws a row by request and records no receipt.
`acknowledge_action(id, seq)` SHALL acknowledge an intent an owner performed
out of band: delete the row, pending or parked, release its pin and record its
receipt (`record_receipt`: the row's id, the collection it was queued on,
`seq` the item it left when the performer knows it), in one transaction,
recording nothing when the row is gone (STORAGE §15.5). `fail_action(id, error)` SHALL record a failed attempt:
`None` bumps `attempts` and leaves the row pending (transient), `Some(error)`
parks it (permanent). A collection's pending actions SHALL expose each row's
`id`, since callers act on rows by id.

Cancelling is an owner write, and it is the only retraction a queued create
has: the kinds that address an existing item are retracted by their inverse
instead, `set-flags` being absolute rather than a delta.
`PimdirStore::cancel_action(dir, id)` SHALL therefore perform it as a scoped
operation, opening the store as owner, cancelling one row and releasing the lock
before returning, so a consumer that is otherwise a reader and a producer never
holds a handle that could drain the queue or sweep the objects. It SHALL NOT
create a store it cannot find (`PimdirError::Uncreated`), and a store another
process owns SHALL fail fast rather than wait.

It SHALL open the store through `open_current`, never `open`: a consumer cancelling a row owns the store for one write and SHALL NOT create, migrate or reconcile it. A store of another `user_version` or shape is refused as `PimdirError::NotCurrent`, left as it was.

#### Scenario: A cancel on a store another revision keeps
- GIVEN a store whose `task_summary` carries a column a later revision added, and a queued row
- WHEN a consumer cancels the row through `cancel_action`
- THEN it answers `NotCurrent(Shape { object: "task_summary" })`, and the table and the row are left as they were (tests/open_current.rs `a_cancel_leaves_a_store_needing_a_reconcile_as_it_was`)

### Requirement: The store owns the content hash
The crate SHALL implement the hashes the format admits (STORAGE §4.3: `blake3`,
recommended, and `sha256-128`) and encode them as STORAGE §5 requires, in lowercase
base32 (RFC 4648, no padding), since the hash is also a path component and a
single-case, filesystem-safe alphabet is what keeps the blob path valid
everywhere.

A store, a producer and the algorithm itself SHALL expose the digest, whole and
incremental, so a consumer hashes through the store it writes to instead of
choosing an algorithm of its own. An object's name is its hash: two processes
disagreeing about it write bodies neither finds and dedup against nothing, and
nothing errors while they do it.

### Requirement: A store declares its algorithm once and is refused on a mismatch
The algorithm SHALL be recorded in `store_meta.hash_algo` when the store is
created, and every blob being a file named by it, it cannot change afterwards.
An open SHALL adopt what an existing store records; an open declaring a
different algorithm, or meeting one this crate does not compute, SHALL be
refused with `PimdirError::HashAlgo` rather than return a handle that names
bodies the store does not use.

### Requirement: A write reads only the rows its batch names
`write` SHALL load the hub narrowed to the link ids its batch touches (`LOAD_ITEMS_BY_LINK`, `LOAD_BINDINGS_BY_LINK`), resolving each dropped handle and each upserted handle to its link id first (`LINK_FOR_HANDLE`, served by `bindings_by_handle`), so a handle bound to another link id is in the batch's diff.

The diff only ever names rows the batch named, so the rest of the collection would be read, cloned and merged to conclude that nothing changed, and that read, not the writes, is what a small write actually costs: it grows with the mailbox instead of with the batch. Both sides of the diff are narrowed the same way, so every comparison the persistence step makes and every object reference the refcount step counts sees exactly what it would have seen in full.

#### Scenario: One flag on one message in a large mailbox
- GIVEN a collection holding many items
- WHEN a batch upserts one placement
- THEN the rows read do not grow with the collection

### Requirement: Every column the update writes is in the diff
The row diff that decides whether an item needs an `UPDATE` SHALL compare every column `UPDATE_ITEM` writes. A column left out is a column that can never change again: the diff reports the row unchanged and no statement is issued for it.

#### Scenario: A restated sort key
- GIVEN a stored item whose key was derived once
- WHEN a write carries a different key and nothing else changed
- THEN the stored key becomes the new one

### Requirement: A descending page reserves no key
The first descending page SHALL bind a `NULL` cursor rather than a key no real one is expected to outrank. A sort key is arbitrary text a writer derives, so no value is reserved: a sentinel hides everything sorting above it from every descending page, permanently, while the count still reports it.

#### Scenario: A key above the sentinel
- GIVEN an item whose key outranks any fixed sentinel
- WHEN the collection is paged in both directions
- THEN both directions page every item

### Requirement: The drain claims a row before applying it
`drain` SHALL delete the queue row it is about to apply as the **first** statement of the applying transaction (`CLAIM_ACTION`), and skip the action when that delete returns nothing.

The pending rows are read outside any transaction, so a second owner may hold the same list; deleting at the end has both apply the row, and `add` and `copy` are not idempotent. Claiming first makes exactly-once a property of the statement rather than a convention about who runs the drain.

### Requirement: A blob rename is durable
Writing a blob SHALL go through a temporary file in the shard directory named uniquely to the writer (the body's name, the process id and a per-process counter), so two processes storing one body never write, then rename, each other's bytes; a temporary another writer holds is left alone, and a failed write removes its own. It SHALL sync the shard directory after the rename, and every directory above it up to objects/, since the write may have created the shards themselves. Syncing the file makes its bytes durable and says nothing about the name that reaches them, while the database commit is durable, so without it a crash can leave a committed row pointing at a body that never arrived: the one asymmetry the write order exists to prevent.

#### Scenario: Another writer's temporary
- GIVEN a temporary file another writer holds under the body's old shared name
- WHEN a producer stages the same body
- THEN the body lands under its name and the other temporary keeps its bytes (tests/store_rules.rs `a_blob_write_never_takes_another_writers_temporary`)

### Requirement: An unreadable flag set holds no opinion
A `flags` column this crate cannot decode SHALL read as unknown, never as a known-empty set. Malformed JSON is a column written by something whose format this does not share, or a corrupted one, and neither is evidence about the item's markers. Reading it as known-empty makes it an authoritative "this item carries no markers", which the merge takes as one side's opinion: it clears every marker the other side reports and persists the result, so a read failure becomes permanent loss.

### Requirement: A binding never changes handle, and nothing is recorded in its place
A write that resolves an existing `(collection, link_id, source)` to a different handle SHALL be refused with a typed error, leaving the bound handle and every other column untouched. The store SHALL record no trace of the incoming handle.

A binding pins one handle, so a write carrying a second one is either a rebuild (licensed per handle by the drop reason, below) or a source holding two resources whose identities resolved to one key. The second is now the engine's to resolve before it writes, by minting a key for the second copy, so the store's whole obligation is to refuse the collision rather than to describe it. Applying it would repoint the binding from the copy it held to another, which is where the evidence used to die: silently, at the write, before any later rule could act on it.

#### Scenario: A colliding write is refused
- GIVEN a stored binding for an identity
- WHEN a write carries the same `(collection, link_id, source)` under a different handle
- THEN the write fails, the bound handle is unchanged, and no ambiguity is stored

### Requirement: A load by key answers with the rows the source binds
`load` under `PimdirLoadScope::Links` SHALL return the placements this source binds under the named keys, and SHALL NOT return the `Created` placement the hub projects for an item the source lacks (hub.md, membership propagation). That projection is what the whole-collection load offers the merge so it derives the append; it is no row the source holds, and every verb reading by key asks who holds the key. An upgrade settling a fetched identity against it reads the sibling's copy as this source's holding and mints `dup:<hint>#<handle>` for the resource the source has always held, where SYNC §6 mints only when this source binds the hint under another handle; a copy or a move into the collection mints the same way and pushes a second copy beside the append the offer stages.

#### Scenario: Two sources already holding one card
- GIVEN an item one source has hydrated, and a member of the same identity a second source lists
- WHEN the second source's page names it
- THEN it is linked under the identity itself and binds the shared item, and no key is minted

### Requirement: An unlinked upsert lands on the binding its handle holds
An `UpsertPlacement` carrying no `link_id` SHALL be resolved against the binding its `(collection, source, handle)` holds (`LINK_FOR_HANDLE`) and folded through the hub as that item. One for a handle no binding holds SHALL be refused (`PimdirError::Unnamed`): nothing reaches the store unnamed, every member a listing carries arriving with its meta (STORAGE §14, SYNC §10).

A placement carries no link id because its identity has not been read, not because it has none. A handle the store has already bound has one, and staging a second row for it makes `load` answer with two placements for one handle, which is read one layer up as two items: the upgrade fetches the handle twice and mints a `dup:` key for a copy that does not exist, binding one source handle to two items no sync can converge. The write is ordinary, not exotic: an earlier engine pulled a remote edit of a locally deleted item as an unnamed row of the handle the tombstone still binds (STORAGE §10).

Resolving before the fold is also what puts such an upsert under the identity floor: the rebind guard and the link set a batch folds into both read the placement's link id, and neither sees a placement that has none.

#### Scenario: A bound handle written unnamed stays one item
- GIVEN a handle bound to an item
- WHEN a write upserts a placement for that handle carrying no link id
- THEN it folds into the bound item and the handle answers with one placement

#### Scenario: A handle nothing holds is refused unnamed
- GIVEN a handle no binding holds
- WHEN a write upserts a placement for it carrying no link id
- THEN the write is refused with `PimdirError::Unnamed` and nothing is stored

### Requirement: A minted link id is an ordinary key
The store SHALL persist whatever `link_id` the engine assigns, SHALL NOT parse it, and SHALL NOT re-canonicalise one. A minted key (pimdir STORAGE §9, `dup:<hint>#<handle>`) is subject to every rule a bare key is: `seq` allocation, retention and revival, dedup by object hash, the reader's pages, and the queue.

The key is the store's, the hint is the format's, and the two are only equal by default. A store that treated a prefix as meaning something would make the engine's assignment reversible by accident and would change a `seq` a consumer has already shown.

#### Scenario: Two resources under one hint are two items
- GIVEN a collection holding an item keyed by a bare hint
- WHEN a write upserts a second item keyed `dup:<hint>#<handle>` for the same source under another handle
- THEN both items exist with their own `seq`, their own binding and their own object reference

### Requirement: A store never collects itself
No write SHALL reclaim. `write`, the queue drain, a cancelled action and both
purges maintain the refcounts exactly as before and delete nothing: an object
at refcount zero is unreferenced, not deleted, and its body stays. That is what
lets a consumer index a body in one batch and attach it in a later one, which
STORAGE §14 invites and a sweep at the end of every write silently broke,
taking the bytes with it.

`PimdirStore::collect_garbage` is the collector: it drops the object rows at
refcount zero and unlinks every blob file the index does not name, which is those
rows' own bodies and the orphans a crash left, reporting the rows, the files and
the bytes. It takes the store's staging lock exclusively and runs on an owning
handle, which already holds the owner lock; those two exclude every other
process. The owner lock excludes nothing inside its own process (STORAGE §8),
so the lock registry SHALL carry a per-store `RwLock` the process's writers
hold shared from blob staging through commit, `write` and the drain alike, and
the collector holds exclusively across its row deletion and its file walk.
Together they let it reclaim with no grace window, since no writer is between
a body and its row while it sweeps. It SHALL refuse with `PimdirError::InFlight`
while a verb of this process runs (`run` holds a running count on the lock
registry), never collecting between two chunks of an upgrade (STORAGE §5), and
SHALL run `RECOMPUTE_REFCOUNTS` before `DELETE_GARBAGE_OBJECTS`, trusting no
incremental count.

The collector owns a file by its position (STORAGE §3, §5): it SHALL unlink
only a file sitting at the shard path its name derives, and SHALL leave alone a
period-prefixed temporary, which belongs to a writer that has not committed,
and any other file under objects/, which is somebody else's. `PimdirBlobs::files`
lists on the same rule.

Unreferenced objects accumulate until someone collects. A consumer that wants
that bounded schedules the verb.

### Requirement: The store repairs what it can recompute
`recompute_refcounts` SHALL settle every object's count from the six columns that pin one (STORAGE §5, §7), `bindings.shared_object` among them, returning how many disagreed, and `clear_dangling_bindings` SHALL delete the bindings whose item is gone, returning how many. Both recover a fact the store already holds. `refcount_drift` compares the stored counts with the same six columns.

Every other dangling row stays reported and untouched: an item whose object row is missing is still the item, and a queue row whose body is missing is still an intent somebody expressed, so deleting either would destroy data rather than repair it.

### Requirement: A superseded handle licenses its own rebind
A write batch dropping a handle with `PimdirDropReason::Superseded` or `Rekeyed` SHALL let the binding holding it move to whatever handle the same batch upserts for that identity, by deleting the binding and inserting the new one (STORAGE §10, §12). A batch dropping a handle as `Deleted`, or not dropping it at all, SHALL keep the bound handle and **refuse the write**. A batch carrying a `Rekeyed` drop SHALL bump the collection's generation in the same transaction, the engine emitting no op for it.

The two are indistinguishable from the rows: a rebuilt spine and a source reporting one identity under a second handle produce the same before and after, and only the drop's reason separates them. Reading a rebuild as a collision refuses every write of a renumbered collection, under handles the server has just voided.

The licence SHALL be per handle rather than per batch, so a rebuild carrying a genuine second copy of an identity still refuses that one and the floor stays intact inside the one operation that legitimately repoints.

#### Scenario: A renumbered collection is not a duplicated one
- GIVEN a binding on a handle a rekey batch supersedes
- WHEN the batch drops it and upserts the item under a new handle
- THEN the binding follows the new handle and projects `Clean`

### Requirement: A body is written before its transaction opens
A write SHALL land the bodies its batch carries in the blob store before opening
the transaction that indexes them (STORAGE §14). A body is content-addressed and
immutable, so writing it early can only produce a file some later batch produces
identically, and the worst a crash between the two leaves is an orphan for the
collector; inside the transaction the same write holds SQLite's single writer
lock across a file write, two `fsync`s and a rename, serialising every other
writer behind an I/O path that touches no database page.

What keeps a collector out of the window this opens is the writer's lock (SPEC
§8), not the file's age. A caller that cannot stage ahead, the queue drain
building its ops inside the transaction that claims the row, MAY write inside
it: the blob write is idempotent, so the batch may re-offer a body at the cost of
one existence check.

### Requirement: The collector asks about the file in front of it
The collector SHALL resolve each blob file against the index with a point lookup
on the primary key (`OBJECT_EXISTS`), never by reading every hash into memory
first: a store of the size STORAGE §1 promises holds hundreds of thousands of them,
and the question is always about one file. `LIST_OBJECT_HASHES` remains for the
diagnosis that visits every row anyway.

### Requirement: A purge releases the pins the delete reported
`PURGE_ITEM` and `PURGE_RETAINED_BEFORE` SHALL return each removed row's
`object_hash` and `conflict_object`, and the caller SHALL settle them with
`RELEASE_PINS` in the same transaction. The pins have to be released by whoever
deletes the rows, and reading them beforehand visits every swept row twice for an
answer the delete already has.

### Requirement: Releasing the owner role and retaking it is one operation
The process-wide owner lock SHALL be released and reacquired under one critical
section: the registry owns the locked file description and closes it as the last
handle goes, rather than tracking a weak reference while each handle holds its
own.

A strong count reaches zero before the description it named is closed, so a
registry that let the two be observed apart would have the next handle open a
second description and `flock` itself out of its own store, reporting `Owned`
with no other process in it (STORAGE §8).

#### Scenario: Handing the role over inside one process
- GIVEN a store whose owner handles are taken and dropped concurrently
- WHEN a handle is taken as the last one is being released
- THEN it shares the role, and no acquisition reports the store owned

### Requirement: A conflict body is pinned
An object referenced by a binding's `conflict_object` SHALL be reachable, and
SHALL NOT be collected while the binding stays conflicted. The pin SHALL be
released when the conflict resolves, so a resolved conflict's body is collected
like any other unreferenced object.

A conflict is resolved by a person, and the interval between the run that found
it and the moment that person sits down is the whole point of persisting it. An
unpinned body does not survive that interval, and its loss is silent: the
resolver finds a revision naming a body that is gone, and can only fall back on
asking the remote, which is the dependency the column exists to remove.

#### Scenario: A conflict body outlives a collection
- GIVEN a conflicted binding whose diverging body is stored
- WHEN the collector runs
- THEN the body is still readable

#### Scenario: Resolving releases the pin
- GIVEN the same binding, then resolved
- WHEN the collector runs
- THEN the body is collected like any other unreferenced object

### Requirement: Conflicted bindings are listable
The store SHALL answer for the conflicted bindings of an account without paging
its collections, naming each one's collection, handle and link id, and the
hashes of its base, local and diverging bodies.

The flag is written and read back with its row today and never filtered on, so
the question "what is waiting for a decision" costs a full scan. A run reports
that count on every invocation and a listing command asks it directly, so it is
a question the store answers rather than one its callers assemble.

#### Scenario: Only the conflicted rows come back
- GIVEN a store holding conflicted and ordinary bindings across two collections
- WHEN its conflicted bindings are listed
- THEN every conflicted one is named with its three hashes, and no other row appears

### Requirement: A binding persists what it last agreed with the hub on
The `bindings` table SHALL carry `shared_object`, round-tripped through `PimdirBinding`: the hub's shared body this source last reconciled against, which is the base of the cross-source merge. It is meaningful on every binding, conflicted or not, and `NULL` only until the source has folded once, where the sync base stands in for it.

This is the second base, for the second axis, and the store MUST keep both. `base_object` is what the source last agreed with its own **remote** and only a sync moves it, which is what keeps a pending push derivable; `shared_object` is what it last agreed with the **hub** and every live absorb moves it. Read one as the other and a source disagrees with itself: its own unpushed edit leaves the sync base behind the shared body exactly as another source folding in does, so the next edit is filed as a cross-source conflict and dropped.

`shared_object` SHALL reference `objects(hash)` and be pinned as `base_object` is (STORAGE §5): counted in the write path's refcount diff, in `recompute_refcounts` and in the collector's reachability, released when the binding agrees with another body, and released by `release_bases_before` with the item's own body. It is the common ancestor a merge reads, so it outlives the item's own body until the binding reconciles against what replaced it.

#### Scenario: The agreement point survives a reopen
- GIVEN an ordinary unconflicted binding written by a sync
- WHEN it is read back after a restart
- THEN it carries the shared body the reconcile settled on

#### Scenario: A conflicted binding carries one too
- GIVEN a binding whose own remote diverged from it
- WHEN it is read back
- THEN its agreement point is there beside the conflict, the flag gating only the conflict pair

#### Scenario: A second offline edit across a restart is not a conflict
- GIVEN a store whose single source edited an item offline, with the push still pending
- WHEN the store is reopened and a second offline edit is absorbed
- THEN the hub adopts it and the item holds the second body

#### Scenario: The agreement point pins its body
- GIVEN a source whose agreement point is the only pointer at a body
- WHEN the collector runs, then the source agrees with another body and the collector runs again
- THEN the first run keeps the body readable, the second takes it (tests/shared_object_pin.rs)

### Requirement: A summary and its addresses are written with the item
A named placement carries its `PimdirSummary` (STORAGE Annex A), and the write SHALL persist it as the row of the kind's summary table and the `item_address` rows, in the item's transaction, only when it moved: the batch's load reads the summaries and addresses of the link ids it names, the diff compares them, and a summary that moved under unmoved item columns runs `stamp_item` so the change feed sees it (STORAGE §4.5). A summary of another variant deletes the old row; a placement carrying none deletes the stored row too.

### Requirement: The change feed is the triggers'
`items.changed` and `collections.changed` SHALL be stamped by the canonical triggers and never bound by a writer: `STAMP_ITEM` requests a stamp (`changed = -1`) the trigger draws, and a rename restamps the collection's items. A purge counts in `store_meta.purges`. The reader SHALL expose `change_cursor`, `items_changed_since` and `collections_changed_since` (STORAGE §4.5); `PimdirChangeCursor::changed` is the last stamp drawn, so a consumer resuming from it misses nothing and repeats nothing.

### Requirement: The refcount floor is a constraint
`objects.refcount` SHALL carry `CHECK (refcount >= 0)`, so a double release fails at the statement that caused it (STORAGE §7).

### Requirement: The queue derives summaries from bodies
A queued `add` or `update` SHALL carry no summary: the drain reads the body the producer wrote and derives the key, the summary and the sort key under Annex A for the collection's declared kind, parking an `add` that names no link id and derives none.

A body an `add` or `update` names that the blob store lacks, or whose file does not hash to its name, SHALL fail the row as a failure of the environment (`PimdirError::Io`, `NotFound`), never apply it: its attempts are bumped and it stays pending, then parks past `MAX_ATTEMPTS` (pimdir STORAGE §15.2). A file failing its name is unlinked by the owner under the staging lock taken exclusively, left for the next read when a producer holds it shared, so the next write of that body writes it anew (STORAGE §7).

#### Scenario: A body unlinked after its enqueue
- GIVEN a queued `add` whose blob was unlinked after the enqueue
- WHEN the owner drains `MAX_ATTEMPTS` times
- THEN each drain but the last counts it `retried`, the last parks it with the error naming the body, and no item was created (tests/queue.rs `a_queued_add_missing_its_body_is_retried_then_parked`, and `a_queued_update_missing_its_body_is_retried_then_parked` for an `update`)

#### Scenario: A body failing its name
- GIVEN a queued `add` whose blob had a byte flipped
- WHEN the owner drains, the producer stages the body again, and the owner drains again
- THEN `get` answers none, the first drain retries and unlinks the file, and the second applies the add (tests/store_rules.rs `a_body_failing_its_name_is_no_body_and_the_owner_unlinks_it`)

### Requirement: A load derives a tombstone's destination and reads its sync state
`load` SHALL set a `Tombstone` placement's origin to the collection `DESTINATION_FOR_LINK` names, the same source's base-less binding of the same link id elsewhere, under the tombstone's own handle, so the engine derives `Remove { to }` from it (SYNC §3); a `Created` placement's origin stays `ORIGIN_FOR_LINK`'s. Both are derived from the bindings and never stored, so they read the same after a reopen and under every scope. Every scope SHALL read the source's checkpoint with its coverage (`LOAD_CHECKPOINT`) and its round under way (`LOAD_ROUND`), and an `All` scope while a round is open the bindings it has not stamped in its scope (`LIST_UNSTAMPED_BINDINGS`, STORAGE §14).

#### Scenario: A staged move survives a reopen
- GIVEN a move staged as a target create and a source tombstone
- WHEN the store is reopened and the source loaded
- THEN the tombstone carries the target as its destination and the create the source as its origin

### Requirement: A handle names one item per source
`bindings_by_handle` is unique (STORAGE §4.3). `write` SHALL resolve every upserted handle with `LINK_FOR_HANDLE`, and a handle bound to a different link id than the upsert carries SHALL retire that binding first, in the same transaction, exactly as a `Deleted` drop of the handle would: `delete_binding`, the item retained when it was its last binding, purged when `held_elsewhere` says the identity moved (STORAGE §10, §11). A `hash:` key changing under one DAV resource is the case: the old item is retained, the new one is ordinary. The diff SHALL release every binding it removes, every item's, before it inserts any, so a handle moving between two items in one batch leaves before it arrives. The refusal of a binding resolved to another handle is unchanged.

#### Scenario: A UID-less card is edited on its server
- GIVEN a handle bound to a `hash:` key
- WHEN a write upserts the same handle under the key of the new bytes
- THEN the old item is retained with no binding, the new one is live under its own `seq`, and the handle projects one placement

### Requirement: The collection-id type is one
Every read and write taking a collection SHALL take `impl AsRef<str>`, `PimdirCollectionId` implementing `AsRef<str>`, so a caller passes a `&str`, a `String` or the id type alike and no two methods spell the parameter apart. The address reads (`address_placements`, `domain_placements`) SHALL skip a row under a role the format lacks, never map it to `from`. Every single-statement owner write (`ensure_collection`, `set_collection_account`, `set_collection_conflict`, `set_sort_key`, `rename_collection`, `fail_action`, `recompute_refcounts`, `clear_dangling_bindings`) SHALL map SQLITE_BUSY and SQLITE_LOCKED to `PimdirError::Busy` as a transaction does. `read_hub` SHALL propagate a failure to serialise its link ids, never read the whole collection in its place.

### Requirement: A refused delete is decided beside the collection's sources
`PimdirSourceStore::sync` SHALL hand `PimdirSync::beside_other_sources` the answer of `COLLECTION_SOURCES`, the union of the collection's `sources` rows and its bindings' sources, this handle's own left out (SYNC §5): a checkpoint counts, since a source whose remote dropped its last member holds no binding and still syncs the collection. There is no configured delete policy.

### Requirement: Conflicted items are listable
`list_item_conflicts(account)` SHALL run `LIST_CONFLICTED_ITEMS` (STORAGE §14.1) and answer each item's collection, link id, `seq`, shared body and diverging body, so a consumer finds what waits for a cross-source decision without paging its collections.

### Requirement: A lookup answers hash and size
`lookup_objects` SHALL answer `PimdirObject`s, hash and size, `LOOKUP_OBJECTS` joining `objects`, for the size witness the upgrade applies before linking an immutable body (SYNC §6).

### Requirement: The schema check covers the triggers
`check` SHALL verify every table and every trigger the canonical migrations declare, `PimdirError::Stale { missing }` naming the first one absent, since the change feed and the collector's counts are the triggers' and a store lacking one corrupts silently.

### Requirement: SQLite is the system's unless `vendored` says otherwise
The `client` feature SHALL take `rusqlite` with no features of its own, so the default build links the SQLite the machine provides through pkg-config and a store this crate writes is the one the system's `sqlite3` reads. A `vendored` feature, off by default, SHALL forward to `rusqlite/bundled` for a static target or a platform carrying no library, mirroring what `vendored` does for OpenSSL across the org. No other feature SHALL imply either choice.

### Requirement: A collection's name is a label, never an address
`collections.name` SHALL be writable independently of `collections.id`: `set_collection_name(collection, name)` moves the label and touches neither the id every foreign key cascades on, the declared kind, nor the account. An owner namespacing its ids SHALL record the bare name there, since the separator is its own convention and a reader cannot strip one it does not know. Nothing keys on the column, so moving it costs a label and never a re-sync; it is observable, so the collection takes a new `changed` stamp and a reader on the feed re-renders.

### Requirement: A producer's enqueue is gated on the declared capabilities
`PimdirProducer::enqueue` SHALL refuse with `PimdirError::Unsupported`, naming capability, source and detail, an action a declared source concerned does not support (pimdir STORAGE §15.6, Annex B.1), reading a calendar write's new and current resources from the blob store for scheduling, occurrence changes and online meetings; an undeclared source gates nothing. `PimdirProducer::check` SHALL run the same gate without enqueueing and answer the partial supports, whose detail a producer shows. An intent with no resolvable performer SHALL be `NoPerformer` or `Ambiguous`, checked once its account has declared anything.

#### Scenario: A scheduled event on a source that notifies nobody
- **GIVEN** a calendar source declaring `calendar.scheduling` `none`
- **WHEN** a producer enqueues an event naming an attendee with no `SCHEDULE-AGENT`
- **THEN** it is refused with `calendar.scheduling`, and the same event marked `SCHEDULE-AGENT=NONE` is queued

### Requirement: The drain parks what a declared source does not support
The drain SHALL run the gate again before applying an action and park one a declared source does not support, the capability named in the error, and SHALL apply `set-performer` as `set_performer` or `delete_performer` for the account of its anchor.

### Requirement: An intent's candidates are read at its anchor
The candidates of an intent SHALL be the account's sources whose row at the anchor collection, its own or else the source-wide one, has some support (`list_capability_sources`). With several, the performer is the latest `set-performer` still queued for the account and capability, else the recorded one, while it is a candidate.

### Requirement: A performed intent is replaced by the change it leaves
`PimdirStore::replace_action` SHALL enqueue the change, pin its body, then cancel the intent, record its receipt with no `seq` and release the intent's pin, in one transaction, enqueueing nothing when the intent is gone (STORAGE §15.5).

### Requirement: The capability tables reconcile on open
Opening a store written before capabilities SHALL create `capabilities`, `performers` and `receipts` from the canonical migration, and a reader of a store lacking them SHALL read every source as undeclared and keep no receipt.

### Requirement: A collection-create is anchored on its parent and needs a declared performer
The gate SHALL read a `collection-create` payload strictly (`PimdirCollectionCreate::from_payload`: `v: 1`, a non-blank `name` without control characters, an optional `parent`; pimdir Annex B.2), refuse one naming a `parent` other than the collection it is enqueued on, or anchored on a collection of no declared kind, with `PimdirActionError::Invalid`, and refuse it with `NoPerformer` on an account whose sources declare nothing, whose owner would skip it for ever. `PimdirProducer::enqueue_collection_create` SHALL anchor it on its parent, else on the collection given, resolve the performer as `performer` does and write it into the payload. `collection.create` SHALL be in the declaration list of every domain.

#### Scenario: A parent that refuses children
- **GIVEN** a mail source declaring `collection.create` `full` and `none` on Archive
- **WHEN** a producer asks for a collection under Archive, then at the top level anchored on INBOX
- **THEN** the first is `NoPerformer` and the second is queued naming the source

### Requirement: An occurrence is gated beside its intent
A `calendar-reply` or `calendar-cancel` naming `recurrence_id` SHALL be refused with `PimdirActionError::Invalid` unless the value is `YYYYMMDD` or `YYYYMMDDTHHMMSS[Z]`, with `Unsupported` unless its performer declares `calendar.reply.occurrence` or `calendar.cancel.occurrence` at the anchor, and with `Unsupported` on an account whose sources declare nothing, whose owner would answer or cancel the whole series. Without the field the intent is gated as before. `PimdirInvitation` SHALL build and read both payloads.

### Requirement: An applied row leaves a receipt a producer follows
The drain SHALL record a receipt (`record_receipt`) in the transaction applying a row, `seq` the item an `add` created (`seq_by_link` on the key it staged) and `None` for every other kind, and SHALL prune the receipts older than `RECEIPT_DAYS`, seven, once its pass is done, never before it reads the pending list. `action_status(id)`, on the producer and the reader, SHALL answer `Pending`, `Parked` with its error, `Applied` with the receipt (an acknowledged or replaced intent included), or `Unknown` for a row withdrawn by request or applied before its receipt was pruned; a store lacking the table answers `Unknown` once the row is gone.

#### Scenario: A draft saved offline
- **GIVEN** a producer that enqueued an `add` and kept the id
- **WHEN** the owner drains
- **THEN** `action_status(id)` is `Applied` with the `seq` of the new item, and a second `add` of the same key reads `Parked`

### Requirement: A collection keeps the role its source states
`PimdirStore::set_collection_role(collection, role)` SHALL set or clear `collections.role` on a declared collection (pimdir STORAGE §14), the old holder of the role in the account losing it in the same statement (`collections_role_moves`), both collections stamped in the feed. The column is an open vocabulary no constraint checks (§13): the owner SHALL refuse a role outside its collection's kind vocabulary (`collection::role_of_kind`) as `PimdirError::Role { collection, role }` (code `collection-role`), and a reader SHALL read a role it does not know, or one of another kind's, as `None`. `PimdirCollection::role` SHALL carry it on `list_collections` and `list_collections_by_account`; a reader of a store its owner has not reconciled SHALL read every role as `None` when the column is absent.

#### Scenario: A server moves its Sent folder
- **GIVEN** `imap/Sent` holding `sent`
- **WHEN** the owner sets `sent` on `imap/Sent Items`
- **THEN** `imap/Sent` holds no role, `imap/Sent Items` holds `sent`, and both are above the reader's cursor

### Requirement: The store holds coverage and rounds per source
`write` SHALL apply `OpenRound`, `SetRoundCursor`, `CloseRound` and `SetCoverage` with the canonical statements (`open_round`, `set_round_cursor`, `close_round`, `set_coverage`), the first in its batch after `ensure_collection`, `open_round` recording whether the round is a band round (`round_band`), which `load_round` answers as `PimdirRound::band`, and `Stamp` (`stamp_bindings`) after the batch's upserts, so a binding the batch inserted is stamped too (STORAGE §4.3, SYNC §5). The items a batch inserts SHALL draw their public ids in the order the batch upserts them, a page's members numbered in the order it names them. Open SHALL reconcile a store an earlier draft wrote, in one transaction: the coverage and round columns of `sources`, `round_band` among them, and `bindings.round` added by `ALTER TABLE`, each cut out of the canonical DDL, an open round of an earlier build reading as no band round, `sources_stamp_coverage` created, and the `probes` table dropped, its handles named again by the next round (STORAGE §6). A reader of a store not reconciled yet reads no coverage. A row an earlier draft wrote at level `0` is repaired to `1` by the reconciliation (`repair_levels`, STORAGE §6) and loads as `Meta`, the level being closed to 1 and 2 (STORAGE §13); the owner's read special-cases no other value.

#### Scenario: An earlier draft's store
- GIVEN a store holding a `probes` table, no coverage columns and items at level `0`
- WHEN its owner opens it
- THEN the columns and the trigger are added, `probes` is dropped, and the items load at `Meta`

### Requirement: A collection is listed with its coverage
`PimdirCollection::coverage` SHALL be the narrowest coverage of the collection's sources (the latest floor, the earliest ceiling, the oldest closing), `None` while one of them never closed a round (STORAGE §14.1); `list_coverage(collection)` SHALL answer it per source, with the scope and start of the round each has under way. A coverage moving restamps its collection (`sources_stamp_coverage`).

### Requirement: The owner collects below a date
`PimdirStore::collect_before(collection, before)` SHALL run `collect_before` and `recompute_refcounts` in one transaction and answer the public ids collected (STORAGE §11.3): every live mail item older than `before` that owes nothing, an item conflicted, pending, or holding unpushed flags staying, an undated one too. It is no delete: no tombstone, no push, the remote keeps every member a later widening names again. It SHALL refuse to run while a verb of the process is between two chunks (`PimdirError::InFlight`).

### Requirement: The owner releases bodies below a date
`PimdirStore::release_before(collections, until)` SHALL run `release_bases_before`, `release_before` and `recompute_refcounts` in one transaction and answer the public ids released (`PimdirReleaseReport`, STORAGE §11.4): every live mail item of the set whose `sort_key` is below `until` (`None` for no ceiling), the undated included, back to `Meta` with its summary, flags and bindings kept, a base naming the body released with it. An item needing its body keeps it: conflicted, holding a conflicted binding, a pending create or a local edit, with no binding, or not bound by a source of its collection. It is no delete and no claim: a later round fetches and pushes nothing for it. It SHALL refuse to run while a verb of the process is between two chunks (`PimdirError::InFlight`).

### Requirement: The mail reads count, page and search across collections
The reader SHALL answer, over a set of collections and under `PimdirMailFilter` (the read and attachment chips, `None` for either), `count_mail`, `count_mail_by_day` (per day of the `Date`, a SQLite modifier moving it to the reader's clock), `count_unread` (per collection), `list_mail_page_filtered` and `search_mail` (newest first on `PimdirMailCursor`, `(sort_key, seq, collection)`, with summaries and addresses), from the committed rows (STORAGE §14.1). All but `search_mail` SHALL take a `since` floor on the sort key, undated mail below it and `None` setting none. `sum_mail` SHALL answer, under the same set and chips over `[since, until)` (either `None` for open), a `PimdirMailSum`: the messages, the summed size of those knowing it, and the count of those that do not. `like_pattern` SHALL build `search_mail`'s pattern from the words searched, `%` around them and `%`, `_` and `\` escaped. A page over several collections SHALL walk `items_by_sort_global` and sort nothing, on a store without statistics (STORAGE §9.3); opening a store as its owner SHALL create that index when absent, a reader of a store lacking it paging by a scan and a sort.

### Requirement: Items refer to each other
`PimdirStore::add_reference(from, to, role, origin)` SHALL run `add_reference` and answer the reference when recorded, or when a `User` one took over an `Auto` one, `None` when it changed nothing: a duplicate, or an end the store holds no row of under its kind (STORAGE §14.2). A role outside the vocabulary and a self-reference SHALL be refused by the schema. `remove_reference` SHALL delete one whatever its origin. `PimdirReader::references_from` and `references_to` SHALL answer an end's references in the canonical order, none on a store lacking the table. A reference SHALL go with the last row of either end, by the canonical trigger; opening a store as its owner SHALL create `item_reference`, `item_reference_to` and `items_drop_references` when absent (STORAGE §6).

#### Scenario: Recorded once, read from either end
- tests/references.rs `a_reference_is_recorded_once_and_read_from_either_end`.

#### Scenario: Gone with the last row
- tests/references.rs `a_reference_goes_with_the_last_row_of_an_end`.

#### Scenario: An earlier store gains the table
- tests/references.rs `an_earlier_store_gains_the_references_on_open`, tests/references.rs `an_earlier_store_derives_its_tasks_organisation_on_open`.

### Requirement: Files, and attachments standing for a message's parts
A collection of kind `application/octet-stream` (`summary::file::KIND`) SHALL hold files, its rows summarised by `PimdirFileSummary` in `file_summary` (Annex A.7), listed A to Z by `list_summaries` (STORAGE §14.3). `PimdirStore::put_file(collection, link_id, summary)` SHALL write a file holding no body at `Meta`, or restate its summary and sort key when held, drawing the `seq` its key already has elsewhere, and SHALL refuse a collection of another kind (`PimdirError::NotFiles`). `summary::file::part_key` SHALL key an attachment `part:<message>#<section>`. `PimdirReader::list_attachments(account, message)` SHALL answer a message's files through its `attachment` references in the order recorded, the stand-in's summary and any placement's body, none on a store lacking the tables. A stand-in SHALL go once no reference names it, by the canonical trigger; opening a store as its owner SHALL create `file_summary` and `item_reference_collects_files` when absent (STORAGE §6).

`PimdirStore::put_item(collection, body)` SHALL write a body into a collection no source syncs (STORAGE §14.3), of kind `text/vcard` or `text/calendar`: its key, summary and sort key derived as a synced write derives them (Annex A), the item at `Full`, its summary, its automatic references (§14.2) and its pins in one transaction, a held key taking the new body with its public id and flags kept, answering the link id and the `seq`. It SHALL refuse a collection a source syncs (`collection_sources` answering any) as `PimdirError::Synced` and any other kind, undeclared included, as `PimdirError::NotUnbound`, writing nothing. `client::TIME_COLLECTION` SHALL be the local time collection's id, `U+0001` and `time`; `PimdirStore::ensure_time_collection` SHALL declare it, kind `text/calendar` under no account and named `time`, idempotently, and answer it. `PimdirStore::delete_unbound` SHALL delete such an item with its summary and release its pins.

#### Scenario: A stand-in is a Meta file
- tests/files.rs `a_stand_in_is_a_meta_file_listed_by_name`.

#### Scenario: A message lists its attachments and takes them with it
- tests/files.rs `a_message_lists_its_attachments_and_takes_them_with_it`.

#### Scenario: An earlier store gains the files
- tests/files.rs `an_earlier_store_gains_the_files_on_open`.

### Requirement: The writer records the automatic references
A write recording an item's summary SHALL run the canonical rule statements for it (STORAGE §14.2): `link_senders_of` for a mail, and `link_invitations_of` when it carries an invitation; `link_sources_to` for a mail; `link_mail_from` for a contact; `link_invitations_to` and `link_parents_to` for a calendar item, and for a task `link_parents_of` when it names a parent and `link_sources_of` when it names a `mid`. Each records origin `auto` from the item stating the reference to what it names, never twice and never over a person's reference, and never removes one: a link made at the write stays when its end later changes. `PimdirMailSummary::invitation` SHALL be derived with the body (Annex A.1) and kept over a write without it; opening a store as its owner SHALL add the column and `mail_summary_by_invitation` when absent.

#### Scenario: Whichever end lands first
- tests/references.rs `the_writer_records_the_automatic_references`.

#### Scenario: Linked at the write, never recomputed
- tests/references.rs `a_rule_links_at_the_write_and_never_recomputes`: the sender at `Meta`, the invitation once the body states it, both kept when the contact drops the address.

#### Scenario: A task refers to its project and its mail
- tests/references.rs `a_task_refers_to_its_project_and_its_mail`: the project held before the task, the mail synced after it, roles `parent` and `source` (`PimdirReferenceRole::Parent`, `Source`).

### Requirement: The audit's answers and the linking reads
`PimdirReader::sum_mail` SHALL count a message once across the set and take a `held` chip. `summary::file::part_key` SHALL answer `None` for a message under a writer-derived key, and `add_reference` SHALL answer the reference standing, new or not, `None` only for an end missing or derived. `PimdirStore::delete_unbound` SHALL delete an item no source binds and release its pins; `delete_collection` SHALL skip the recompute when `collection_holds_objects` answers false. `get_mail_row`, `describe_endpoint`, `search_contacts`, `search_calendar`, `search_files` and `list_attachments_by_account` SHALL answer their canonical statements, none on a store lacking a table they name. Opening a store as its owner SHALL rebuild `collections` when its role `CHECK` lacks `attachments`, and backfill `mail_summary.invitation` from the held bodies when it adds the column (STORAGE §6).

#### Scenario: The linking reads
- tests/linking.rs, tests/files.rs `an_account_lists_its_attachments_and_deletes_unbound_files`.

#### Scenario: An older store is reconciled
- tests/files.rs `an_earlier_role_constraint_is_rebuilt_on_open`, tests/linking.rs `an_earlier_store_backfills_the_invitation_on_open`.

### Requirement: Open reconciles against the canonical text
Opening a version 1 store as its owner SHALL compare every table, index and trigger with the one 0001_init.sql alone creates in an empty in-memory database, by its `sqlite_schema` text with comments dropped and whitespace collapsed (STORAGE §6). A store that differs SHALL be reconciled in one transaction, foreign keys off:

1. The data a new constraint would refuse is repaired first: `repair_shared_objects` and `recompute_refcounts` (once `bindings` holds `shared_object`, else after it is rebuilt), `repair_levels`, `repair_kinds`.
2. A table that differs is rebuilt from the canonical text (its indexes and triggers dropped, renamed aside with `legacy_alter_table` on, created, its shared columns copied, its `AUTOINCREMENT` counter carried, the old one dropped).
3. What is missing is created, what differs recreated, what the canonical schema lacks dropped.
4. The columns a rebuild added are backfilled with the canonical statements: `backfill_shared_object` then `repair_shared_objects`, `backfill_invitation` over the held bodies by `list_held_mail` then `link_invitations_of`, `backfill_task_organisation` over the held bodies by `list_held_tasks` then the four task rules with a `NULL` link id; then `recompute_refcounts` again, and no foreign key left dangling.

A current store SHALL be left untouched.

#### Scenario: An older shape reconciled
- tests/files.rs `an_earlier_role_constraint_is_rebuilt_on_open` (the rebuilt text equals a fresh store's), tests/scoped_sync.rs `an_earlier_draft_store_is_reconciled_on_open` (a level 0 row repaired to `Meta`), tests/linking.rs `an_earlier_store_backfills_the_invitation_on_open`, tests/references.rs `an_earlier_store_gains_the_references_on_open`.

### Requirement: A body entering the store stands for its attachments
A write SHALL record, in its transaction, the stand-ins of every `message/rfc822` body it brings into the store (STORAGE §14.3): a body a live, stored item under a key naming an identity gains, which `lookup_objects` did not find under that key in the account before the write. Each part `summary::mail::attachments` answers SHALL get a file at `Meta` holding no body in the account's attachments collection, keyed `part:`, with its `file_summary`, unless the collection holds it, and an `auto` `attachment` reference from the message. The collection SHALL be the account's file collection with the role `attachments`, else `<account>/\u0001attachments` (`\u0001attachments` with no account) declared with the file kind, the name `attachments` and the role. A body filed again, stored again or held before SHALL walk nothing: no backfill, no recompute.

#### Scenario: Recorded once, as the body enters
- tests/attachments.rs; sync vector 70 in tests/vectors_sync.rs.

### Requirement: A body gaining a row states the columns only the body gives
A write SHALL state, on the mail row of every live, stored item gaining a body (pimdir STORAGE Annex A.1), the columns only the body gives, over whatever the writer read without it. A body entering the store (no row of its key in the account held it before the write, `lookup_objects`) SHALL be walked once (`summary::mail::attachments::PimdirAttachmentWalk`), its mark stated and its invitation stated when it gives one, the same walk answering its stand-ins; a writer-derived key's body is walked for these too and stands for nothing. A body the account already holds SHALL be linked: the row's size is the object's, and its mark and invitation those a row holding that body states (`held_mail_summary`). A body no such row states a mark of (rows written before marks were read off bodies) SHALL be walked from its blob for the mark and the invitation of the new row alone: no stand-in recorded (§14.3) and the held rows left as they are, the writer's mark kept when the blob store lacks the body.

#### Scenario: A streamed body
- tests/attachments.rs `a_body_read_without_it_states_what_its_parts_give`: the header block's guess (an attachment, no invitation) written with the body reads as the body gives (none, the `UID`).

#### Scenario: A copy of a held body
- tests/attachments.rs `a_copy_of_a_held_body_takes_the_columns_its_body_gave`: a placement written with a held hash and its listing's summary takes the held row's mark and invitation and the body's size, and records no stand-in.

#### Scenario: A copy of a body held with no mark
- tests/attachments.rs `a_copy_of_a_body_held_with_no_mark_takes_the_columns_a_walk_gives`: held rows stating neither column and no stand-in, as an older writer left them; a copy guessing the opposite mark takes the walk's mark and invitation, records no stand-in, and the held rows still state neither.

### Requirement: Collections refusing tasks

`capability::X_VTODO` SHALL name `x-vtodo` (STORAGE §15.6). The gate SHALL refuse with `PimdirError::Unsupported` naming `x-vtodo` an `add` or `update` whose new resource is a VTODO, and a `move` or `copy` of one, into a collection where a source concerned states `x-vtodo` `none` there or source-wide; a source stating no row refuses nothing by it. `PimdirReader::accepts_vtodo(collection)` SHALL answer whether no source syncing the collection states `none` there, for a client choosing where tasks go.

#### Scenario: an events-only calendar
- GIVEN source caldav declaring `x-vtodo` `none` on Events
- WHEN a producer enqueues an add of a VTODO into Events
- THEN it is refused naming `x-vtodo` and caldav, a VEVENT is queued, `accepts_vtodo("Events")` is false and true elsewhere (tests/capabilities.rs)

### Requirement: Which message ids a mail replies to

`PimdirReader::replied(message_ids)` SHALL answer which of the given ids some live mail replies to (`list_replied`), mail under the `sent` or `drafts` role aside: each id trimmed and stripped of one pair of enclosing angle brackets, an empty one dropped, compared bare and exactly, answered once each in code point order; `collections.role` missing reads as `NULL`.

#### Scenario: a reply and the person's own
- GIVEN INBOX holding a mail replying to asked@example.org and Sent (role `sent`) one replying to mine@example.org
- WHEN a reader asks for `<asked@example.org>`, ` mine@example.org `, never@example.org, asked@example.org and an empty id
- THEN it answers asked@example.org alone (tests/replied.rs)

### Requirement: An owner open that reconciles nothing
`PimdirStore::open_current(dir)` SHALL open the existing store as its owner, taking the same lock as `open` (`PimdirError::Owned` at once when another process holds it), and SHALL create, migrate and reconcile nothing. It SHALL refuse, writing nothing, as `PimdirError::NotCurrent` with `PimdirNotCurrent::Absent` when no pimdir.db is under `dir`, `PimdirNotCurrent::Version { found }` when `user_version` is not this crate's, and `PimdirNotCurrent::Shape { object }` naming the first table, index or trigger whose text differs from the canonical schema or that one side lacks, which `open` would reconcile. A damaged or foreign store is refused as `open` refuses it (`Stale`, `VersionMismatch`, `Foreign`, `HashAlgo`).

#### Scenario: a store another revision reconciled
- GIVEN a store whose `task_summary` lacks the task organisation columns
- WHEN `open_current` opens it
- THEN it answers `NotCurrent(Shape { object: "task_summary" })` and the table is left as it was; `open` then reconciles it and `open_current` opens it (tests/open_current.rs)

### Requirement: An add superseding an item

`PimdirAction::Add` SHALL carry `supersedes: Option<PimdirSupersedes>`, `PimdirSupersedes { collection, seq }`, encoded as the payload's `supersedes: { "collection", "seq" }` (STORAGE §15.3) and decoded strictly (a malformed one parks). The drain SHALL stage the add, parking it for a live duplicate link id before anything else, then in the same transaction tombstone the superseded item as a `remove` would and record the pair (`supersede_item`); a superseded item absent applies the add alone, and one no source binds is retained. The gate SHALL run the add against its collection and a `remove` of the superseded item against the sources binding it, as producer and as the owner's backstop.

#### Scenario: a parked add leaves the source in place
- GIVEN task 1 live in Inbox and a queued add into Next carrying `supersedes: {Inbox, 1}`, Next holding a live item under the add's link id
- WHEN the owner drains
- THEN the row is parked, task 1 is still live in Inbox and no supersession is recorded (tests/engine/supersedes.rs)

### Requirement: A producer stages its own body
`PimdirProducer::stage_body(bytes)` SHALL hash a body under the store's algorithm and write it to the blob tree while the producer's staging lock is held, answering the `PimdirObject` `enqueue` takes; `PimdirProducer::blob_writer()` SHALL do the same for a streamed body. A collector running between the stage and the enqueue SHALL be refused (`PimdirError::Staging`), so the body reaches the row that pins it.

The producer SHALL hold the staging lock for its staging window alone (STORAGE §8), not its lifetime: taken at the first `stage_body`, `blob_writer` or one-step enqueue, released once a committed enqueue of this producer pinned every body it staged whole, and kept until the producer drops after a streamed one, whose hash the writer alone knows. An open producer that stages nothing, an always-open frontend, keeps no collector out. The lock is taken before the enqueue transaction opens, so waiting out a collector never holds the write lock it needs.

#### Scenario: An idle producer
- tests/staging.rs `an_idle_producer_keeps_no_collector_out` (failed before the change: the collector was refused beside an idle producer)

#### Scenario: A collector between a stage and its enqueue
- GIVEN a producer that staged a body and has not enqueued yet
- WHEN the owner runs the collector, then the producer enqueues an `add` naming the body
- THEN the collector is refused with `Staging`, and the drain applies the add over the body (tests/staging.rs `a_body_staged_before_its_enqueue_survives_a_collector`)

### Requirement: A create refuses a link id live or pending
`PimdirProducer::enqueue_add(collection, link_id, flags, object, supersedes)` SHALL enqueue an `add` naming `link_id`, answering the queue row id, and SHALL refuse it with `PimdirError::Duplicate`, nothing appended, when the collection holds a live item under that link id (`live_item_for_link`, the check the drain parks on) that no pending `remove` or `move` withdraws, or when a pending `add` in the collection names it. The check and the append SHALL be one transaction, so two producers cannot both pass it.

#### Scenario: A create racing the same key
- GIVEN a collection with a live item `mid:a` and a pending add `mid:b`
- WHEN a producer adds `mid:a`, then `mid:b`, then `mid:c`
- THEN the first two are `Duplicate` (`Live(seq)`, `Pending(row)`) and leave the queue as it was, the third answers its row id (tests/staging.rs `a_create_naming_a_live_or_pending_link_id_is_refused`)

### Requirement: The submit intent is typed
`intent::PimdirSubmit` SHALL build and read the `submit` intent payload (pimdir STORAGE Annex B.2): `v: 1`, the optional performing `source`, the message's `object` (the body the row pins), the envelope `from` (empty for the null path) and `rcpts` (at least one), the optional `subject` and the optional `copy` collection. `to_action` SHALL validate and answer the `PimdirAction::Unknown` carrying it, its `object_hash` set; `from_payload` SHALL read a payload strictly, refusing an unknown version, a missing field or an empty `rcpts` with `PimdirActionError`.

### Requirement: An error carries a stable code
`PimdirError::code()` SHALL answer a stable kebab-case code per failure, the contract a consumer maps to its own output rather than matching the message: `store-stale`, `store-newer` (a `Version`, raised above this crate's version alone, or a `NotCurrent` above it), `store-uncreated` (an `Uncreated` or an absent `NotCurrent`), `store-foreign`, `store-not-current`, `store-corrupt`, `store-hash-algo`, `store-busy`, `store-owned`, `store-staging`, `store-in-flight`, `sql`, `io`, `json`, `action-malformed`, `binding-rebind`, `placement-unnamed`, `scope-not-mail`, `collection-not-files`, `collection-synced`, `collection-not-unbound`, `item-duplicate`, `unsupported`, `no-performer`, `ambiguous-performer`, `cutoff-invalid`, and with `search` `query-refused`, `tag-refused`, `index-fts5-missing`, `query-unanswerable`, `index-busy`, `index-missing`, `index-outdated`, `index-query-only`. A code never changes meaning; a new failure gets a new code.

No message SHALL advise deleting the store: a resync restores neither the queue, unsent mail, retained items, references nor items kept on the device. `Stale` and `NotCurrent` say to run or upgrade the store's sync engine, `Version` above this crate's to upgrade this program; the CLI's report the same.

#### Scenario: A newer store
- GIVEN a store stamped above this crate's version
- WHEN an owner, a producer or a reader opens it
- THEN the error's code is `store-newer` and its message ends "upgrade this program", naming no reset (tests/queue.rs `a_store_stamped_with_a_higher_version_is_refused`); a store missing a core table is `store-stale`, its message ending "run or upgrade the store's sync engine" (tests/stale_store.rs)

### Requirement: The public API names no foreign error type
`PimdirError::Sql`, `Unanswerable` and `Json` SHALL wrap the opaque `PimdirSqlError` and `PimdirJsonError`, which display the cause and give it through `Error::source` without naming rusqlite or serde_json types, so a major of either is not one of this crate. The `From` conversions the crate's own `?` relies on are hidden from the documentation and are no contract.

### Requirement: The enums consumers match grow without a major
`PimdirError`, `PimdirSyncEvent`, `PimdirEnumerated` and `PimdirQueryErrorCode` SHALL be `#[non_exhaustive]`: a consumer matching one keeps a wildcard arm, and a variant the spec adds is a minor. Structs stay pub-fielded and exhaustive, built as literals.

### Requirement: A store this crate may not write is never written
A store above this crate's version, a foreign database, and a store whose algorithm is not the one declared SHALL be refused by the owner open before any byte of pimdir.db changes; the reader, the producer and `open_current` SHALL refuse the first two as well, a database declaring tables at `user_version` 0 being foreign (`PimdirError::Foreign`, `store-foreign`) for every role, never `store-uncreated` nor `store-not-current` (tests/store_version.rs `every_role_refuses_a_foreign_database_as_foreign`).

#### Scenario: A newer store in rollback-journal mode
- GIVEN a released store stamped one version above this crate's, in rollback-journal mode, holding a table this crate does not know
- WHEN the owner, a reader, a producer and `open_current` open it
- THEN each is refused (`Version`, `store-newer`) and pimdir.db is byte for byte as it was (tests/store_version.rs `a_store_above_this_version_is_refused_by_every_role_and_never_written`; also `a_foreign_database_is_refused_and_left_as_it_was`, `a_declared_algorithm_is_checked_before_anything_is_reconciled`)

### Requirement: A reader or a producer reads an older store as it stands
A reader or a producer meeting a store below this crate's version, or a version 1 store its owner has not reconciled yet, SHALL read it as its reconciliation would leave it before any backfill (STORAGE §6): a column it lacks as its canonical `DEFAULT` where 0001_init.sql declares it `NOT NULL` with one, else `NULL`, a table it lacks as empty, running no statement naming either. `schema::as_reconciled` runs a canonical statement with each table the store lacks, or holds without some column, read through a subquery answering the canonical columns, the statement's parameters and result columns kept; every statement a reader, a producer or the search index runs over the store goes through it, through `schema::StoreShape`, which runs it unchanged on a store holding every canonical table and column: the collection listings, `list_coverage`, `replied`, the task and summary reads, the capability and performer reads, the queue and receipt reads, the reference, attachment, file and endpoint reads, and the index's compiled query, feed and extraction. A store without the capability tables therefore lists its syncing sources undeclared, and one without `file_summary` lists its attachments without summaries, as its owner's reconciliation would. Owner-only paths (the drain, the write path, diagnostics) read a store their open reconciled or `open_current` found current. A source whose store lacks the coverage columns therefore never closed a round: `list_coverage` lists it with no coverage, and a search over its collection is filling, never complete.

#### Scenario: The released fixtures, read as they stand
- GIVEN a store io-pimdir 0.5.1, 0.7.0 or 0.8.0 wrote, with a source syncing INBOX
- WHEN a reader and a producer read it
- THEN `list_coverage("INBOX")` lists the source with no coverage and no round, `search_coverage` is `Bounded { filling: true }`, every public read answers, and pimdir.db is byte for byte as it was (tests/store_version.rs `a_store_released_by_*_reads_as_its_reconciliation_would_leave_it`; 0.5.1 failed before the change: no source listed)
- src/client/schema.rs `a_statement_reads_an_older_store_as_its_reconciliation_leaves_it` (a `NOT NULL` default, `categories` as `'[]'`, a missing table as no row), `a_current_store_runs_the_statement_unchanged`

#### Scenario: Capabilities on a store without the capability tables
- GIVEN the 0.5.1 fixture, unreconciled, and a copy its owner reconciled
- WHEN a reader and a producer read capabilities, performers and action statuses
- THEN both answer alike, the syncing sources undeclared (tests/store_version.rs `a_store_released_by_*_declares_as_its_reconciliation_would`; 0.5.1 failed before: no source listed)

### Requirement: A released store opens whole
A store io-pimdir 0.5.1, 0.7.0 or 0.8.0 wrote SHALL open as owner with every row of every table kept: collections (one of a media type outside today's closed set, recorded kindless), items (a retained one included), bindings, objects, receipts, references, and the queue, its parked row still parked and its pending rows applying at the next drain; the invitation and the task organisation are re-derived from the held bodies, the refcounts settle with no drift, and the next `open_current` finds it current.

#### Scenario: The released fixtures
- tests/fixtures/store-v0.5.1, store-v0.7.0 and store-v0.8.0, written by those releases with tests/fixtures/store_gen.rs, read by a reader and a producer before the owner opens them (tests/store_version.rs `a_store_released_by_*_opens_without_losing_a_row`)

### Requirement: A migration runs after the reconciliation and keeps every row
A migration above version 1 SHALL run after the reconciliation of a version 1 store, so a script assuming 0001's canonical shape applies; it SHALL move both stamps to its number and keep every row of every table. A store at version 2 or above is never reconciled to 0001.

#### Scenario: A synthetic 0002
- GIVEN a version 1 store missing an index an earlier draft lacked, and a 0002 that drops that index, adds a column and a table
- WHEN the runner opens it with both migrations
- THEN both stamps read 2, every table keeps its row count, a second open drops nothing it added, and this crate (version 1) refuses it as `Version { found: 2 }` (src/client/schema.rs `migration::a_migration_runs_after_the_reconciliation_and_keeps_every_row`)

### Requirement: The store records the derivation revision
The store SHALL record in `store_meta.derivation` the Annex A revision its summaries, addresses and sort keys were derived under (STORAGE §6): `init_store_meta` writes this crate's (`schema::DERIVATION`, revision 1), a reconciliation gives an existing store the column's default 1, and an owner at a later revision re-derives what changed then records it with `set_derivation`, which never lowers it. Raising `DERIVATION` owes that re-derivation.

### Requirement: A stated identity beginning like a derived key is escaped
The key a body states (a `Message-ID`, a vCard or iCalendar `UID`) SHALL be filed under `esc:` and the hint when the hint begins with one of the reserved prefixes `alt:`, `hash:`, `dup:`, `part:`, `file:` or `esc:` (STORAGE §9, `PimdirLinkId::RESERVED`, `PimdirLinkId::from_hint`), so it never reads as a key the writer derived. A connector naming the identity to its server strips the one leading `esc:`.

#### Scenario: A card whose UID reads like a hash key
- the summaries vector case card-reserved-uid.vcf (tests/summaries.rs)

### Requirement: An edit names the body it replaced
`PimdirAction::Update` SHALL carry `if_match: Option<PimdirHash>`, encoded as the payload's optional `if_match` (STORAGE §15.3): the body the producer read and replaced. The drain SHALL park the row, naming both bodies, when the item holds another body, and apply a row stating none over whatever the item holds.

#### Scenario: Two producers edit one read
- tests/store_rules.rs `an_update_whose_read_body_was_replaced_parks`

### Requirement: A write staged with its body stages nothing when refused
`PimdirProducer::enqueue_with_body(collection, body, |hash| action)` SHALL hash the body, build the action from its hash, run the gate of §15.6 on the bytes in hand and, for an `add`, the duplicate check of `enqueue_add` on its link id or on the key the drain would derive from the body, then write the body and append the row in one transaction, answering the row id. A refused write SHALL leave objects/ as it was. `stage_body` with `enqueue` or `enqueue_add` keep working, a refused write there leaving its body for the collector.

`PimdirProducer::enqueue_with_body_checked` SHALL be the same step answering `PimdirEnqueued { id, partial }`, `partial` the partial supports the gate passed the action with, read with the body in hand, so a frontend warns without running `check` a second time (tests/capabilities.rs `a_checked_enqueue_answers_the_partial_supports_it_passed_with`).

#### Scenario: A duplicate and an unsupported create
- tests/staging.rs `a_create_refused_for_its_link_id_stages_nothing`, tests/capabilities.rs `a_refused_create_stages_nothing`

### Requirement: An action on an item no source binds is the store's to settle
The drain SHALL never skip an action on an existing item no source binds, which would leave the row pending for ever (STORAGE §15.2 reserves skipping for a kind, a payload version or an intent the owner does not run). A `remove` SHALL retain the item, a delete nobody pushes. In a collection no source syncs, an item kept on the device (`put_item`, STORAGE §14.3), a `set-flags` SHALL set its flags and an `update` SHALL write its new body as `put_item` writes one, its flags kept and its summary derived again, on an `update`'s usual terms: one whose `if_match` names another body than the item's parks naming both (STORAGE §15.3), checked before the item is found unbound; a body deriving no summary, or another key than the item's, parks. A `move` or a `copy` of such an item, and anything on an unbound item of a synced collection, SHALL park naming the item. Each applied row leaves its receipt and releases its pin as any other.

#### Scenario: A device-only event edited, flagged, moved and removed
- tests/unbound_items.rs `a_queued_action_on_an_item_no_source_binds_applies_or_parks` (failed before the change: the drain reported both rows skipped)

#### Scenario: A device-only event edited over a replaced body
- tests/unbound_items.rs `an_unbound_edit_over_a_replaced_body_parks` (passed before this change: the fence was already checked in `stage_action`)

### Requirement: The drain reports the bodies a policy kept aside
`PimdirDrainReport::overruled` SHALL list, as `PimdirDrainOverruled { id, collection, link_id }`, every body a collection's conflict policy did not adopt while an applied row was written (SYNC §9), kept as a retained item under `link_id`: nothing else reports a body set aside during a drain. `PimdirDrainReport` is therefore not `Copy`.

#### Scenario: A drained edit under prefer-incoming
- tests/queue.rs `a_body_a_policy_overrules_during_a_drain_is_reported` (failed before the change: the retained item was kept and nothing reported it)

### Requirement: The journal mode is chosen when the store is created
The owner open SHALL set `PRAGMA journal_mode = WAL` when it creates a store, as STORAGE §4.1 recommends for a local one, and no handle SHALL set the journal mode of an existing store: a store its operator switched to a rollback journal for a network filesystem (STORAGE §8) stays on it through every owner, producer and reader open. README states the switch.

#### Scenario: A store on a rollback journal
- tests/store_rules.rs `a_store_on_a_rollback_journal_stays_on_it` (failed before the change: the producer's open set WAL again)

### Requirement: A purge cutoff is written as the stamps are
`purge_retained_before(cutoff)` SHALL refuse a cutoff not written as the store's stamps are, `%Y-%m-%dT%H:%M:%fZ` (STORAGE §11.2, the comparison being textual), as `PimdirError::Cutoff` (`cutoff-invalid`), purging nothing: a word (`now`) or an instant without milliseconds would purge what was never asked.

#### Scenario: A word and an instant without milliseconds
- tests/retention.rs `a_purge_cutoff_not_written_as_the_stamps_are_is_refused` (failed before the change: `now` purged the trash)

### Requirement: pimdir.db holds the canonical schema alone
The owner's reconciliation of a version 1 store SHALL drop every table, column, index and trigger 0001_init.sql lacks, whoever added it (STORAGE §6), and this crate SHALL keep no state of its own in pimdir.db: the search index's lives in index.db (SEARCH §2), the locks in their own files.

#### Scenario: Another implementation's schema
- GIVEN a current store to which another implementation added a table, a column, an index and a trigger
- WHEN its owner opens it
- THEN all four are gone, every canonical row is kept, and `open_current` finds it current (tests/store_version.rs `the_reconciliation_drops_what_the_canonical_schema_lacks`)

### Requirement: A producer proves it can write without writing
`PimdirProducer::probe` SHALL take the staging lock shared without waiting and release it, and take the database's write lock and roll it back, so an app's check proves the store writable by its producer, the staging lock included, before its first write, writing nothing into the store (objects/ untouched). A collector holding the staging lock SHALL be reported at once as `PimdirError::Busy` (`store-busy`), never waited out. A producer whose staging window is open already holds the lock and probes the database alone.

#### Scenario: A collector holds the staging lock
- GIVEN a store whose objects.lock another process holds exclusively
- WHEN a producer probes, THEN it answers `Busy` at once; once the lock is released it answers Ok, and objects/ holds no file (tests/staging.rs `a_probe_takes_the_staging_lock_and_writes_nothing`)
