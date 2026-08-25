//! The canonical pimdir SQL, inlined verbatim from the spec so the crate is
//! self-contained. Kept in sync with `pimdir/migrations/` and
//! `pimdir/queries/`; the spec is the source of truth.
//!
//! A store keeps one shared **item** per logical thing (its truth: flags, body,
//! summary), and one **binding** per source that syncs it (that source's last
//! agreed base). A single-source store is the degenerate case of one binding per
//! item; a two-source store (two servers, or a server and a phone) keeps two.

/// Schema version 1 (`migrations/0001_init.sql`), the whole draft schema
/// including the action queue and collection generations. Applied to a fresh
/// database; the caller sets `PRAGMA user_version = 1` on success.
pub const MIGRATION_0001: &str = r#"
CREATE TABLE store_meta (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    format     TEXT    NOT NULL DEFAULT 'pimdir',
    version    INTEGER NOT NULL,
    hash_algo  TEXT    NOT NULL,
    created_at TEXT    NOT NULL,
    -- Store-global monotonic counter handing out the next item `seq`; only ever
    -- increases, so a public id is never reused across the whole store.
    next_seq   INTEGER NOT NULL DEFAULT 1
) STRICT;

-- `account` is the multi-account axis (SPEC.md §9.2): NULL in a single-account
-- store, an opaque owner-chosen id when one store holds several. It groups; it
-- neither keys nor partitions. No identifier is scoped by it: an identity or a
-- body occurring in two accounts is a fact the store reports
-- (LIST_LINK_PLACEMENTS, LIST_OBJECT_PLACEMENTS) and an interface interprets.
CREATE TABLE collections (
    id          TEXT PRIMARY KEY,
    account     TEXT,
    kind        TEXT NOT NULL,
    name        TEXT NOT NULL,
    parent      TEXT REFERENCES collections(id) ON UPDATE CASCADE ON DELETE SET NULL,
    color       TEXT,
    description TEXT,
    sort_order  INTEGER,
    -- Cross-source content-conflict policy: 'manual' | 'prefer-incoming' | 'prefer-existing'.
    conflict    TEXT NOT NULL DEFAULT 'manual',
    -- Collection generation: bumped by the owner whenever it rebuilds the
    -- collection's handle space (a backend identity reset), so a reader can derive
    -- epoch-dependent protocol values (an IMAP UIDVALIDITY) from the store alone
    -- (SPEC.md §12).
    generation  INTEGER NOT NULL DEFAULT 1
) STRICT;

-- "Every collection of this account", the merged view's filter axis. Partial: a
-- single-account store writes no account and pays for no index.
CREATE INDEX collections_by_account ON collections(account) WHERE account IS NOT NULL;

-- One row per source that syncs a collection (a server, a phone). A
-- single-source collection has one row here.
CREATE TABLE sources (
    collection TEXT NOT NULL REFERENCES collections(id) ON UPDATE CASCADE ON DELETE CASCADE,
    source     TEXT NOT NULL,
    checkpoint BLOB,
    PRIMARY KEY (collection, source)
) STRICT;

CREATE TABLE objects (
    hash     TEXT PRIMARY KEY,
    size     INTEGER NOT NULL,
    refcount INTEGER NOT NULL DEFAULT 0
) STRICT;

-- The shared truth of one logical item, keyed by its cross-source link id.
-- `deleted` lingers after a source removes it, until every source has dropped
-- it too (the cross-source delete memory). Once no source holds it, the row is
-- RETAINED rather than deleted: a store never loses an item, purge does.
CREATE TABLE items (
    collection      TEXT NOT NULL REFERENCES collections(id) ON UPDATE CASCADE ON DELETE CASCADE,
    link_id         TEXT NOT NULL,
    -- The message's public id: store-global, one per link_id (shared by its
    -- placements across mailboxes), never reused. A client shows it and resolves
    -- it back to `link_id`.
    seq             INTEGER NOT NULL,
    flags           TEXT,
    object_hash     TEXT REFERENCES objects(hash),
    meta            TEXT,
    -- The kind's ordering key, written beside `meta`; '' means unknown.
    sort_key        TEXT NOT NULL DEFAULT '',
    level           INTEGER NOT NULL,
    deleted         INTEGER NOT NULL DEFAULT 0,
    -- RFC 3339 instant the last binding vanished; non-NULL means retained
    -- (soft-deleted). One column carries both the flag and the purge clock.
    retained_at     TEXT,
    -- The source whose removal retired the item, diagnostic only.
    retained_by     TEXT,
    conflicted      INTEGER NOT NULL DEFAULT 0,
    conflict_object TEXT REFERENCES objects(hash),
    PRIMARY KEY (collection, link_id)
) STRICT;

-- One source's binding of an item: its handle there, the base last synced with
-- it (the 3-way-merge baseline), and whether that source's own sync is stuck on
-- an unresolved content conflict.
CREATE TABLE bindings (
    collection    TEXT NOT NULL,
    link_id       TEXT NOT NULL,
    source        TEXT NOT NULL,
    handle        TEXT NOT NULL,
    base_flags    TEXT,
    base_object   TEXT REFERENCES objects(hash),
    base_revision TEXT,
    -- Whether a base exists at all, which its three value columns cannot say: a
    -- source reporting no revision, no body and markers nobody has read still
    -- agreed, and that agreement is what tells a pending push from a settled
    -- one. Inferring presence from the three loses exactly that shape.
    base_present  INTEGER NOT NULL DEFAULT 0,
    -- This source and its OWN remote diverged. Distinct from
    -- items.conflicted, which is the cross-source divergence.
    conflicted        INTEGER NOT NULL DEFAULT 0,
    conflict_revision TEXT,
    -- The OTHER handles this source holds this identity under, as a JSON array,
    -- or NULL: the identity-axis twin of `conflicted`. A source may hold one
    -- link id twice, and a binding pins one handle, so the second has nowhere
    -- to live; recording it keeps the write from silently repointing the
    -- binding, and makes the freeze survive a restart.
    ambiguous_handles TEXT,
    PRIMARY KEY (collection, link_id, source),
    FOREIGN KEY (collection, link_id) REFERENCES items(collection, link_id) ON UPDATE CASCADE ON DELETE CASCADE
) STRICT;

-- The action queue (SPEC.md §15): mutations requested by processes that are not
-- the store owner, applied by the owner in append order.
CREATE TABLE queue (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,  -- global append order
    created_at  TEXT    NOT NULL,                   -- RFC 3339 timestamp
    producer    TEXT    NOT NULL,                   -- enqueuing process, diagnostic only
    collection  TEXT    NOT NULL REFERENCES collections(id) ON UPDATE CASCADE ON DELETE CASCADE,
    action      TEXT    NOT NULL,                   -- 'add' | 'set-flags' | 'remove' | 'move' | 'copy' | 'update', or an owner-defined intent
    payload     TEXT    NOT NULL,                   -- versioned JSON, shape per action (SPEC.md §15)
    object_hash TEXT    REFERENCES objects(hash),   -- pins the payload's body against GC, or NULL
    attempts    INTEGER NOT NULL DEFAULT 0,         -- apply attempts so far
    error       TEXT                                -- last failure; non-NULL means parked
) STRICT;

-- The owner drains a collection's pending actions in append order.
CREATE INDEX queue_by_collection ON queue(collection, id);

CREATE INDEX items_by_object ON items(object_hash);
CREATE INDEX bindings_by_object ON bindings(base_object);
-- A message's public id is shared by its placements, so it is unique per
-- (collection, seq) — the key a client resolves.
CREATE UNIQUE INDEX items_by_seq ON items(collection, seq);
-- Indexes the cross-collection "does this message already have a seq?" lookup.
CREATE INDEX items_by_link ON items(link_id);
-- Partial: the trash view and the purge sweep scan the retained set without
-- ever touching the live rows, which are the overwhelming majority.
CREATE INDEX items_retained ON items(collection, retained_at) WHERE retained_at IS NOT NULL;
-- Orders a collection by the kind's own sort key, with `seq` as the tiebreaker
-- that makes a keyset page over a non-unique key well defined.
CREATE INDEX items_by_sort ON items(collection, sort_key, seq);
-- `seq` is the store-global public id (spec §9.1), displayed and accepted back
-- without naming its collection; resolving one against items_by_seq means
-- scanning that whole index, since it leads with the collection.
CREATE INDEX items_by_seq_global ON items(seq);
-- The sweep of unreferenced objects. Partial, so it holds only what is about to
-- be collected and is empty at rest: without it both the list and the delete
-- scan the whole objects table, on every write transaction.
CREATE INDEX objects_garbage ON objects(refcount) WHERE refcount <= 0;
-- The other two pointers at an object, so a refcount recomputation reaches every
-- reference by index rather than by scanning items and queue once per object.
CREATE INDEX items_by_conflict_object ON items(conflict_object);
CREATE INDEX queue_by_object ON queue(object_hash);
-- Resolves one source handle back to the link id it is bound to, which is what
-- a batch dropping a placement needs: a drop names a handle and the shared item
-- is keyed by link id. Without it that resolution is a scan of every item.
CREATE INDEX bindings_by_handle ON bindings(collection, source, handle);
"#;

/// The current schema version.
pub const VERSION: i64 = 1;

/// Creates a collection row if it does not exist yet, leaving an existing one
/// untouched (the kind is declared separately by `SET_COLLECTION_KIND`).
pub const ENSURE_COLLECTION: &str = "\
INSERT INTO collections(id, account, kind, name) VALUES(:collection, :account, '', :collection) \
ON CONFLICT(id) DO NOTHING";

/// Declares (or re-declares) a collection's kind, creating the row if the
/// collection is not known yet. Updates the kind alone, so a collection never
/// changes account as a side effect of a sync declaring its media type.
pub const SET_COLLECTION_KIND: &str = "\
INSERT INTO collections(id, account, kind, name) VALUES(:collection, :account, :kind, :collection) \
ON CONFLICT(id) DO UPDATE SET kind = excluded.kind";

/// Regroups a collection under another account, or out of one with `NULL`. Safe
/// at any time: the account partitions no identifier (spec §9.2), so the move
/// leaves seqs, link ids and objects alone.
pub const SET_COLLECTION_ACCOUNT: &str =
    "UPDATE collections SET account = :account WHERE id = :collection";

/// Gives a collection a new id, carrying its whole contents with it: every
/// foreign key onto `collections(id)` is `ON UPDATE CASCADE`, so the items,
/// bindings, sources, queue rows and child collections follow in the same
/// statement (spec §14).
///
/// The only safe way to change an id. Deleting and recreating the collection
/// instead destroys the cache: the `ON DELETE CASCADE` takes every item and
/// binding with it, so a rename silently becomes a full re-download and drops
/// any staged local change not yet pushed.
pub const RENAME_COLLECTION: &str = "UPDATE collections SET id = :new_id WHERE id = :collection";

/// Reads a collection's owning account.
pub const LOAD_ACCOUNT: &str = "SELECT account FROM collections WHERE id = :collection";

/// Reads a collection's declared kind.
pub const LOAD_KIND: &str = "SELECT kind FROM collections WHERE id = :collection";

/// Stores a collection's conflict policy.
pub const SET_CONFLICT: &str = "UPDATE collections SET conflict = :conflict WHERE id = :collection";

/// Reads a collection's conflict policy.
pub const LOAD_CONFLICT: &str = "SELECT conflict FROM collections WHERE id = :collection";

/// Loads a whole collection for the sync seam: every item, tombstones
/// included, unpaginated and unordered.
///
/// Retained (soft-deleted) rows are excluded. That is what makes retention safe
/// under io-replica's contract: the merge reconciles only what `load` returns,
/// so a hidden row is never re-derived, on a delta or a full resync.
///
/// `sort_key` rides along so the round trip preserves it: the engine now
/// carries the key on a placement, so a load that dropped it would hand every
/// save an unknown key and the update below would write that back, erasing on
/// every sync what the last one derived (spec §9.3).
pub const LOAD_ITEMS: &str = "\
SELECT link_id, flags, object_hash, meta, sort_key, level, deleted, conflicted, conflict_object \
FROM items WHERE collection = :collection AND retained_at IS NULL";

/// The same rows, narrowed to the link ids one write batch touches (spec §14).
///
/// A write folds its batch into the hub and persists the difference, and that
/// difference only ever names rows the batch named: reading the rest costs a
/// full pass over the collection to compute nothing. It is the whole cost of a
/// small write, and it grows with the mailbox rather than with the batch.
pub const LOAD_ITEMS_BY_LINK: &str = "\
SELECT link_id, flags, object_hash, meta, sort_key, level, deleted, conflicted, conflict_object \
FROM items WHERE collection = :collection AND retained_at IS NULL \
  AND link_id IN (SELECT value FROM json_each(:links))";

// Client read surface (kind-agnostic, indexed getters over the same store the
// sync seam writes). Distinct from `LOAD_ITEMS`: paginated, live-only, ordered.

/// Lists every collection with its display metadata and generation, ordered by
/// `sort_order` then id, the ones carrying no sort order coming last.
pub const LIST_COLLECTIONS: &str = "\
SELECT id, account, kind, name, parent, color, description, sort_order, generation \
FROM collections ORDER BY sort_order IS NULL, sort_order, id";

/// One account's collections, the filter axis of a merged view. `IS` so binding
/// `NULL` selects the collections of a single-account store.
pub const LIST_COLLECTIONS_BY_ACCOUNT: &str = "\
SELECT id, account, kind, name, parent, color, description, sort_order, generation \
FROM collections WHERE account IS :account ORDER BY sort_order IS NULL, sort_order, id";

/// The accounts owning at least one collection. A store knows an account only
/// through its collections (spec §9.2), so this is not a configured roster.
pub const LIST_ACCOUNTS: &str = "\
SELECT DISTINCT account FROM collections WHERE account IS NOT NULL ORDER BY account";

/// A keyset page of a collection's live items in **link-id order**. `:after` is
/// the exclusive lower bound on `link_id` (the empty string starts from the
/// beginning, since a `link_id` is never empty); rides the `items` primary key,
/// no extra index.
///
/// Link-id order means nothing to a reader: this is the page for a sweep that
/// must see every item exactly once (an export, a re-projection). A reader
/// presenting a list wants one of the two ordered pages below.
pub const LIST_ITEMS_PAGE: &str = "\
SELECT seq, link_id, flags, object_hash, meta, sort_key, level FROM items \
WHERE collection = :collection AND deleted = 0 AND link_id > :after \
ORDER BY link_id LIMIT :limit";

/// A keyset page of a collection's live items in the kind's own **ascending**
/// order (spec §9.3): A to Z for contacts, earliest first for mail and
/// calendars.
///
/// The cursor is the pair `(:after_key, :after_seq)`, because a sort key is not
/// unique: two messages share a timestamp, two contacts share a name. `seq`
/// breaks the tie, and being unique per collection it makes the page total. The
/// empty string with seq 0 starts from the beginning, since no real key sorts
/// before an unknown one ascending.
pub const LIST_ITEMS_PAGE_ASC: &str = "\
SELECT seq, link_id, flags, object_hash, meta, sort_key, level FROM items \
WHERE collection = :collection AND deleted = 0 \
AND (sort_key, seq) > (:after_key, :after_seq) \
ORDER BY sort_key, seq LIMIT :limit";

/// The same page **descending**: newest first for mail and calendars, Z to A for
/// contacts.
///
/// The first page binds a NULL cursor rather than a key above every other one:
/// a sort key is arbitrary text a writer derives, so no value is reserved and
/// "the largest key the store can hold" is not expressible. A sentinel would
/// hide everything sorting above it from every descending page, for good, while
/// the count still reported it. The comparison stays a keyset one, so the index
/// still serves it.
pub const LIST_ITEMS_PAGE_DESC: &str = "\
SELECT seq, link_id, flags, object_hash, meta, sort_key, level FROM items \
WHERE collection = :collection AND deleted = 0 \
AND (:after_key IS NULL OR (sort_key, seq) < (:after_key, :after_seq)) \
ORDER BY sort_key DESC, seq DESC LIMIT :limit";

/// Restates one item's ordering key, for a re-projection that derives sort keys
/// for items already stored: a store written before its kind had a convention,
/// one whose convention changed, or a consumer whose sync engine does not carry
/// the key inline yet (spec §9.3). Not part of the ordinary write path.
pub const SET_SORT_KEY: &str = "\
UPDATE items SET sort_key = :sort_key \
WHERE collection = :collection AND link_id = :link_id";

/// Fetches one live item by its public id (`seq`) — the client-facing key.
pub const GET_ITEM: &str = "\
SELECT seq, link_id, flags, object_hash, meta, sort_key, level FROM items \
WHERE collection = :collection AND seq = :seq AND deleted = 0";

/// Resolves an item's public id (`seq`) from its internal `link_id` — the inverse
/// of `GET_ITEM`, for a consumer that just staged an add and wants the new id.
pub const SEQ_BY_LINK: &str =
    "SELECT seq FROM items WHERE collection = :collection AND link_id = :link_id";

/// Counts a collection's live items (tombstones excluded).
pub const COUNT_ITEMS: &str =
    "SELECT count(*) FROM items WHERE collection = :collection AND deleted = 0";

/// Every live placement of one identity, with the collection and account it
/// sits in (spec §9.2). The store reports where a link id occurs and takes no
/// position on whether the placements are one thing: a mail view lists them, a
/// contact view may offer to merge them, off the same rows.
pub const LIST_LINK_PLACEMENTS: &str = "\
SELECT i.collection, c.account, i.seq, i.object_hash, i.flags, i.level \
FROM items i JOIN collections c ON c.id = i.collection \
WHERE i.link_id = :link_id AND i.deleted = 0 AND i.retained_at IS NULL \
ORDER BY c.account IS NULL, c.account, i.collection";

/// The same on the dedup axis, by body rather than identity, so it pairs
/// placements two servers gave different link ids.
pub const LIST_OBJECT_PLACEMENTS: &str = "\
SELECT i.collection, c.account, i.seq, i.link_id, i.flags, i.level \
FROM items i JOIN collections c ON c.id = i.collection \
WHERE i.object_hash = :hash AND i.deleted = 0 AND i.retained_at IS NULL \
ORDER BY c.account IS NULL, c.account, i.collection";

/// The distinct source names the store has synced (across all collections), so a
/// client can discover which source to attribute its writes to.
pub const LIST_SOURCES: &str = "SELECT DISTINCT source FROM bindings ORDER BY source";

/// Loads every per-source binding of a collection: the stored base (handle,
/// flags, object, revision) each sync merges against.
pub const LOAD_BINDINGS: &str = "\
SELECT link_id, source, handle, base_flags, base_object, base_revision, base_present, \
conflicted, conflict_revision, ambiguous_handles \
FROM bindings WHERE collection = :collection";

/// The same rows, narrowed to the link ids one write batch touches: the binding
/// half of [`LOAD_ITEMS_BY_LINK`].
pub const LOAD_BINDINGS_BY_LINK: &str = "\
SELECT link_id, source, handle, base_flags, base_object, base_revision, base_present, \
conflicted, conflict_revision, ambiguous_handles \
FROM bindings WHERE collection = :collection \
  AND link_id IN (SELECT value FROM json_each(:links))";

/// Whether a collection holds a live (non-retained, non-deleted) item under a
/// link id: the collision check a queued `add` runs before staging.
///
/// A point read on the items primary key, because it runs once per drained
/// action: answering it by loading the collection makes a drain of N actions
/// cost N passes over the mailbox.
pub const LIVE_ITEM_FOR_LINK: &str = "\
SELECT seq FROM items \
WHERE collection = :collection AND link_id = :link_id \
  AND deleted = 0 AND retained_at IS NULL";

/// One source's handle for an item, which its binding's primary key answers
/// directly: the lookup a queued action needs to name the placement it edits.
pub const HANDLE_FOR_LINK: &str = "\
SELECT handle FROM bindings \
WHERE collection = :collection AND link_id = :link_id AND source = :source";

/// The link id one source's handle is bound to, for a batch that drops a
/// placement: a drop names a handle, and the hub is keyed by link id.
///
/// Served by the `bindings_by_handle` index, so resolving it is a seek rather
/// than the scan over every item a whole-collection load would answer it with.
pub const LINK_FOR_HANDLE: &str = "\
SELECT link_id FROM bindings \
WHERE collection = :collection AND source = :source AND handle = :handle";

/// Reads one source's sync checkpoint for a collection.
pub const LOAD_CHECKPOINT: &str =
    "SELECT checkpoint FROM sources WHERE collection = :collection AND source = :source";

/// The message's existing public id, if any placement of this `link_id` already
/// has one (in any collection), so all placements of a message share one id.
pub const SEQ_FOR_LINK_ANY: &str = "SELECT seq FROM items WHERE link_id = :link_id LIMIT 1";

/// Hands out (and advances) the store-global next public id via `RETURNING`. The
/// counter only ever increases, so a `seq` is never reused. Run only when the
/// message has no id yet.
pub const BUMP_NEXT_SEQ: &str =
    "UPDATE store_meta SET next_seq = next_seq + 1 WHERE id = 1 RETURNING next_seq - 1";

/// Inserts one item row (the new-placement path; `UPDATE_ITEM` handles an
/// existing one).
pub const INSERT_ITEM: &str = "\
INSERT INTO items(collection, link_id, seq, flags, object_hash, meta, sort_key, level, deleted, conflicted, conflict_object) \
VALUES(:collection, :link_id, :seq, :flags, :object_hash, :meta, :sort_key, :level, :deleted, :conflicted, :conflict_object)";

/// Updates one existing item's columns in place (the diffed-save path; the
/// primary key `(collection, link_id)` is unchanged).
pub const UPDATE_ITEM: &str = "\
UPDATE items SET flags = :flags, object_hash = :object_hash, meta = :meta, sort_key = :sort_key, \
level = :level, deleted = :deleted, conflicted = :conflicted, conflict_object = :conflict_object \
WHERE collection = :collection AND link_id = :link_id";

// Retention (spec §11): the last binding vanishing retires the row instead of
// deleting it, a reappearing link id revives it, and purge is the only true
// delete.

/// Retires one item: it stands exactly where a hard-deleting store would have
/// issued its delete. The row keeps its `object_hash`, so the body keeps its
/// reference and its blob survives the sweep. SQLite stamps the instant itself,
/// so no clock is plumbed through the crate to reach this statement; a purge's
/// *cutoff* is by contrast the caller's parameter, which keeps the tests
/// deterministic.
pub const RETAIN_ITEM: &str = "\
UPDATE items SET deleted = 1, \
retained_at = strftime('%Y-%m-%dT%H:%M:%fZ','now'), retained_by = :source \
WHERE collection = :collection AND link_id = :link_id";

/// Deletes every binding of one item, for the retire path: the row survives, but
/// no source holds it, so no base does either (a delete would have cascaded).
/// A retained row carrying no binding at all is the persisted form of "the
/// removal has finished propagating" (spec §11).
pub const DELETE_ITEM_BINDINGS: &str =
    "DELETE FROM bindings WHERE collection = :collection AND link_id = :link_id";

/// The retained row holding a link id, if any: its public id and the objects it
/// pins, which revive releases and purge reclaims.
pub const RETAINED_ITEM: &str = "\
SELECT seq, object_hash, conflict_object FROM items \
WHERE collection = :collection AND link_id = :link_id AND retained_at IS NOT NULL";

/// Revives a retained row: the link id is back (a source-side resurrection, or a
/// client `add`), so it stops being retained instead of conflicting on the
/// primary key. The caller adopts the new content with `UPDATE_ITEM` in the same
/// transaction. The row keeps its `seq`, so a restored item keeps the public id
/// it always had.
pub const REVIVE_ITEM: &str = "\
UPDATE items SET deleted = 0, retained_at = NULL, retained_by = NULL \
WHERE collection = :collection AND link_id = :link_id";

/// A keyset page of a collection's retained items, joined to the body size the
/// row still pins (`NULL` when unhydrated): the trash listing beside
/// `LIST_ITEMS_PAGE`, and the only read that returns them.
///
/// `:after` is the exclusive lower bound on the public `seq` (0 starts from the
/// beginning), an equivalent substitution for the reference statement's
/// `link_id` cursor (spec §7): a caller pages the trash by the same small
/// integer it purges and restores by.
pub const LIST_RETAINED_PAGE: &str = "\
SELECT i.seq, i.link_id, i.flags, i.object_hash, i.meta, i.sort_key, i.level, \
i.retained_at, i.retained_by, o.size \
FROM items i LEFT JOIN objects o ON o.hash = i.object_hash \
WHERE i.collection = :collection AND i.retained_at IS NOT NULL AND i.seq > :after \
ORDER BY i.seq LIMIT :limit";

/// Every index the schema grew after version 1 was first published, as one
/// idempotent batch: a store written by an earlier draft has the tables but not
/// these, and an index is not something a reader can do without.
///
/// Run on open rather than only when a column is missing, because most of these
/// index columns that were always there: what changed is that a statement now
/// needs them. A store that kept the old plans would keep scanning where the
/// schema says it seeks, silently and for good.
pub const ENSURE_INDEXES: &str = "\
CREATE INDEX IF NOT EXISTS items_retained ON items(collection, retained_at) \
WHERE retained_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS collections_by_account ON collections(account) \
WHERE account IS NOT NULL;
CREATE INDEX IF NOT EXISTS items_by_sort ON items(collection, sort_key, seq);
CREATE INDEX IF NOT EXISTS items_by_seq_global ON items(seq);
CREATE INDEX IF NOT EXISTS objects_garbage ON objects(refcount) WHERE refcount <= 0;
CREATE INDEX IF NOT EXISTS items_by_conflict_object ON items(conflict_object);
CREATE INDEX IF NOT EXISTS queue_by_object ON queue(object_hash);
CREATE INDEX IF NOT EXISTS bindings_by_handle ON bindings(collection, source, handle);";

/// Counts a collection's retained items, the counterpart of `COUNT_ITEMS`;
/// rides the `items_retained` partial index.
pub const COUNT_RETAINED: &str =
    "SELECT count(*) FROM items WHERE collection = :collection AND retained_at IS NOT NULL";

/// The store-wide size of the bodies retention is holding, each distinct object
/// counted once (two retained placements of one message share it). An upper
/// bound on what a purge reclaims: an object a live item also points at keeps a
/// reference and survives the sweep.
pub const RETAINED_BYTES: &str = "\
SELECT coalesce(sum(o.size), 0) FROM objects o WHERE o.hash IN \
(SELECT object_hash FROM items WHERE retained_at IS NOT NULL AND object_hash IS NOT NULL)";

/// The objects one retained item pins, addressed by its public id: what the
/// targeted purge releases before deleting the row. A live item matches nothing,
/// so a purge can never reach one.
pub const RETAINED_ITEM_BY_SEQ: &str = "\
SELECT object_hash, conflict_object FROM items \
WHERE collection = :collection AND seq = :seq AND retained_at IS NOT NULL";

/// Purges one retained item by its public id: the only true delete. Its bindings
/// cascade, and the body it released is unlinked by the ordinary refcount sweep.
/// Guarded on `retained_at`, so a purge can never take a live item.
pub const PURGE_ITEM: &str = "\
DELETE FROM items WHERE collection = :collection AND seq = :seq AND retained_at IS NOT NULL";

/// The objects the time-based sweep is about to release, with the rows'
/// collections and link ids. Strictly before the cutoff, so an item retained
/// exactly at that instant is kept.
pub const RETAINED_BEFORE: &str = "\
SELECT collection, link_id, object_hash, conflict_object FROM items \
WHERE retained_at IS NOT NULL AND retained_at < :cutoff";

/// The time-based sweep: every item retired before `:cutoff` (RFC 3339),
/// store-wide, since how long to keep is the owner's policy rather than a
/// collection's. The cutoff is the caller's parameter, not the store's clock, so
/// the boundary is deterministic even though the stamp is SQLite's.
pub const PURGE_RETAINED_BEFORE: &str =
    "DELETE FROM items WHERE retained_at IS NOT NULL AND retained_at < :cutoff";

/// Inserts one item's binding for one source (the new-binding path;
/// `UPDATE_BINDING` handles an existing one).
pub const INSERT_BINDING: &str = "\
INSERT INTO bindings(collection, link_id, source, handle, base_flags, base_object, \
base_revision, base_present, conflicted, conflict_revision, ambiguous_handles) \
VALUES(:collection, :link_id, :source, :handle, :base_flags, :base_object, \
:base_revision, :base_present, :conflicted, :conflict_revision, :ambiguous_handles)";

/// Updates one existing binding's columns in place (its primary key
/// `(collection, link_id, source)` is unchanged).
///
/// `handle` is deliberately not among them. A binding pins one handle, and
/// repointing it to a different one is how the fact that a source holds an
/// identity twice was destroyed, silently, at the write: no later rule could
/// then act on it, because the evidence was already gone. A second copy is
/// recorded in `ambiguous_handles` instead, which freezes the item until the
/// source holds the identity once again. Rebinding a handle legitimately, after
/// a handle-space change, goes through the rebuild that drops the old spine and
/// inserts the new one, never through this statement.
pub const UPDATE_BINDING: &str = "\
UPDATE bindings SET base_flags = :base_flags, \
base_object = :base_object, base_revision = :base_revision, base_present = :base_present, \
conflicted = :conflicted, conflict_revision = :conflict_revision, \
ambiguous_handles = :ambiguous_handles \
WHERE collection = :collection AND link_id = :link_id AND source = :source";

/// Deletes one source's binding of an item.
pub const DELETE_BINDING: &str = "DELETE FROM bindings WHERE collection = :collection AND link_id = :link_id AND source = :source";

/// Adjusts one object's refcount by a signed delta (the incremental-refcount
/// path); the hash's primary key makes this an indexed point update.
pub const ADJUST_REFCOUNT: &str =
    "UPDATE objects SET refcount = refcount + :delta WHERE hash = :hash";

/// Releases one reference from each of the given hashes (a JSON array), the
/// set-based form of [`ADJUST_REFCOUNT`] at `-1`.
///
/// A hash listed twice releases twice, which is what makes it the same
/// operation as the loop it replaces: a retained item pins its body and its
/// conflict body separately, and a purge releases both.
pub const RELEASE_PINS: &str = "\
UPDATE objects SET refcount = refcount - \
  (SELECT count(*) FROM json_each(:hashes) WHERE value = objects.hash) \
WHERE hash IN (SELECT value FROM json_each(:hashes))";

/// Writes one source's sync checkpoint for a collection, replacing the
/// previous one.
pub const UPSERT_CHECKPOINT: &str = "\
INSERT INTO sources(collection, source, checkpoint) VALUES(:collection, :source, :checkpoint) \
ON CONFLICT(collection, source) DO UPDATE SET checkpoint = excluded.checkpoint";

/// Indexes an object by its content hash at refcount 0; re-storing a known
/// hash only refreshes its size, since the count belongs to
/// `ADJUST_REFCOUNT`.
pub const STORE_OBJECT: &str = "\
INSERT INTO objects(hash, size, refcount) VALUES(:hash, :size, 0) \
ON CONFLICT(hash) DO UPDATE SET size = excluded.size";

/// Resolves the object hash currently bound to each of the given link ids
/// (passed as a JSON array), skipping the ones carrying no body.
///
/// Scoped to one account, which is the axis a link id is trustworthy on. Across
/// collections it is exactly what this read exists for: one message filed in two
/// mailboxes is one body, downloaded once. Across accounts it is not a fact at
/// all, because two unrelated servers may mint the same vCard `UID` (spec §9.2),
/// and answering with the other account's body hands one account's content to
/// the other's sync, which then believes the item is hydrated. A single-account
/// store writes no account, so the filter is a no-op there and the dedup is
/// whole-store, as it should be.
pub const LOOKUP_OBJECTS: &str = "\
SELECT i.link_id, i.object_hash FROM items i \
JOIN collections c ON c.id = i.collection \
WHERE i.object_hash IS NOT NULL \
  AND i.link_id IN (SELECT value FROM json_each(:links)) \
  AND c.account IS :account";

/// Lists the objects no placement references any more: the blobs the write
/// transaction is about to collect.
pub const LIST_GARBAGE_OBJECTS: &str = "SELECT hash FROM objects WHERE refcount = 0";

/// Every hash the index holds, for the collector to diff the blob tree against:
/// a file this does not name is a body nothing references.
pub const LIST_OBJECT_HASHES: &str = "SELECT hash FROM objects";

/// Drops the unreferenced object rows inside the collector's transaction; their
/// blobs are unlinked after the commit, so a crash leaves at worst an orphan
/// blob.
pub const DELETE_GARBAGE_OBJECTS: &str = "DELETE FROM objects WHERE refcount <= 0";

/// Recomputes every object's refcount from the four columns that pin one (spec
/// §7): an item's body, an item's conflict copy, a source's stored base and a
/// pending queue action's body.
///
/// The repair, not the write path: writes maintain the count incrementally with
/// `ADJUST_REFCOUNT`, which is O(changes) where this is O(items+bindings+queue).
/// The pointers are gathered into one stream and counted in a single grouped
/// pass, so the cost is linear in them rather than in their product with the
/// object table. The left join is what settles an object no pointer names any
/// more: it counts zero rather than going unvisited. A row already holding its
/// true count is left alone, so the statement writes only the drift it found,
/// and reports how many rows that was.
pub const RECOMPUTE_REFCOUNTS: &str = "\
UPDATE objects SET refcount = counted.n \
FROM ( \
  SELECT o.hash AS hash, count(r.hash) AS n FROM objects o \
  LEFT JOIN ( \
    SELECT object_hash AS hash FROM items WHERE object_hash IS NOT NULL \
    UNION ALL SELECT conflict_object FROM items WHERE conflict_object IS NOT NULL \
    UNION ALL SELECT base_object FROM bindings WHERE base_object IS NOT NULL \
    UNION ALL SELECT object_hash FROM queue WHERE object_hash IS NOT NULL \
  ) r ON r.hash = o.hash \
  GROUP BY o.hash \
) AS counted \
WHERE counted.hash = objects.hash AND objects.refcount != counted.n";

/// Deletes the bindings whose item is gone, the one dangling row a repair can
/// clear without guessing: a binding with no item is unreachable, where an item
/// with no object row still holds the item.
pub const DELETE_DANGLING_BINDINGS: &str = "\
DELETE FROM bindings WHERE NOT EXISTS ( \
  SELECT 1 FROM items i \
  WHERE i.collection = bindings.collection AND i.link_id = bindings.link_id)";

// The action queue (spec §15, `queries/queue.sql`): the write door for every
// process that is not the store owner. A producer appends; the owner applies
// pending actions in append order and deletes each in the same transaction as
// its effects.

/// A producer's append. Runs after `ENSURE_COLLECTION`, in one transaction with
/// the `STORE_OBJECT` upsert when the payload references a body (spec §15.1).
pub const ENQUEUE_ACTION: &str = "\
INSERT INTO queue(created_at, producer, collection, action, payload, object_hash) \
VALUES(:created_at, :producer, :collection, :action, :payload, :object_hash)";

/// The collections with pending work, for the owner's drain loop.
pub const LIST_QUEUED_COLLECTIONS: &str =
    "SELECT DISTINCT collection FROM queue WHERE error IS NULL";

/// The owner's drain: a collection's pending (non-parked) actions, in append
/// order. A reader runs the same statement to overlay pending actions on its
/// item projection (read-your-writes, spec §15.4).
pub const LOAD_PENDING_ACTIONS: &str = "\
SELECT id, created_at, producer, action, payload, object_hash, attempts \
FROM queue WHERE collection = :collection AND error IS NULL ORDER BY id";

/// Deletes the row an owner is about to apply, and reports whether it was still
/// there.
///
/// It runs **first** in the applying transaction, not last: the pending rows are
/// read outside any transaction, so a second owner reading the same list would
/// otherwise apply every action a second time, and `add` and `copy` are not
/// idempotent. Claiming the row before doing its work makes exactly-once a
/// property of the statement rather than a convention about who runs the drain.
pub const CLAIM_ACTION: &str = "DELETE FROM queue WHERE id = :id RETURNING id";

/// One queue row's spent attempts and pinned body, for a caller acting on a row
/// by id: cancelling it, acknowledging an intent it performed out of band, or
/// recording a failure.
pub const LOAD_ACTION_ROW: &str = "SELECT attempts, object_hash FROM queue WHERE id = :id";

/// One queue row removed by request rather than by application, pending or
/// parked (spec §15.5): a queued item withdrawn, or a performed intent
/// acknowledged by the process that could carry it out. The same delete as
/// `DELETE_ACTION`, named apart because the trigger is a request, not an apply.
/// It releases the row's `object_hash` pin, so it runs in one transaction with
/// the refcount settle.
pub const CANCEL_ACTION: &str = "DELETE FROM queue WHERE id = :id";

/// A permanently failing action: recorded and skipped, visible to operators and
/// frontends instead of blocking the collection's queue forever.
pub const PARK_ACTION: &str =
    "UPDATE queue SET attempts = :attempts, error = :error WHERE id = :id";

/// Records a failed apply attempt without parking (the retry path; equivalent
/// substitution of the reference `park_action` with a `NULL` error).
pub const BUMP_ATTEMPTS: &str = "UPDATE queue SET attempts = attempts + 1 WHERE id = :id";

/// The parked actions, for status surfaces and operator repair.
pub const LOAD_PARKED_ACTIONS: &str = "\
SELECT id, created_at, producer, collection, action, payload, attempts, error \
FROM queue WHERE error IS NOT NULL ORDER BY id";

/// The owner's handle-space reset marker (spec §12): run in the same
/// transaction as the rebuild it records.
pub const BUMP_GENERATION: &str = "\
UPDATE collections SET generation = generation + 1 WHERE id = :collection \
RETURNING generation";

/// A collection's handle-space epoch, so a reader derives epoch-dependent
/// protocol values (an IMAP UIDVALIDITY) from the store alone.
pub const LOAD_GENERATION: &str = "SELECT generation FROM collections WHERE id = :collection";

/// Every statement in this module, paired with its constant name.
///
/// The way a consumer without the `client` feature reaches the canonical SQL:
/// it holds its own SQLite driver (an Android app runs the platform's), so it
/// needs the statements by name rather than a Rust accessor per statement.
/// [`MIGRATION_0001`] is included, since creating the database is as much a
/// consumer's job as querying it; [`VERSION`] is not, being an integer.
///
/// Hand-written, and guarded: the test below derives the expected set from this
/// module's own source, so a statement added without being indexed fails the
/// suite instead of shipping a silent gap.
pub const ALL: &[(&str, &str)] = &[
    ("MIGRATION_0001", MIGRATION_0001),
    ("ENSURE_COLLECTION", ENSURE_COLLECTION),
    ("SET_COLLECTION_KIND", SET_COLLECTION_KIND),
    ("SET_COLLECTION_ACCOUNT", SET_COLLECTION_ACCOUNT),
    ("RENAME_COLLECTION", RENAME_COLLECTION),
    ("LOAD_ACCOUNT", LOAD_ACCOUNT),
    ("LOAD_KIND", LOAD_KIND),
    ("SET_CONFLICT", SET_CONFLICT),
    ("LOAD_CONFLICT", LOAD_CONFLICT),
    ("LOAD_ITEMS", LOAD_ITEMS),
    ("LOAD_ITEMS_BY_LINK", LOAD_ITEMS_BY_LINK),
    ("LIST_COLLECTIONS", LIST_COLLECTIONS),
    ("LIST_COLLECTIONS_BY_ACCOUNT", LIST_COLLECTIONS_BY_ACCOUNT),
    ("LIST_ACCOUNTS", LIST_ACCOUNTS),
    ("LIST_ITEMS_PAGE", LIST_ITEMS_PAGE),
    ("LIST_ITEMS_PAGE_ASC", LIST_ITEMS_PAGE_ASC),
    ("LIST_ITEMS_PAGE_DESC", LIST_ITEMS_PAGE_DESC),
    ("SET_SORT_KEY", SET_SORT_KEY),
    ("GET_ITEM", GET_ITEM),
    ("SEQ_BY_LINK", SEQ_BY_LINK),
    ("COUNT_ITEMS", COUNT_ITEMS),
    ("LIST_LINK_PLACEMENTS", LIST_LINK_PLACEMENTS),
    ("LIST_OBJECT_PLACEMENTS", LIST_OBJECT_PLACEMENTS),
    ("LIST_SOURCES", LIST_SOURCES),
    ("LOAD_BINDINGS", LOAD_BINDINGS),
    ("LOAD_BINDINGS_BY_LINK", LOAD_BINDINGS_BY_LINK),
    ("LIVE_ITEM_FOR_LINK", LIVE_ITEM_FOR_LINK),
    ("HANDLE_FOR_LINK", HANDLE_FOR_LINK),
    ("LINK_FOR_HANDLE", LINK_FOR_HANDLE),
    ("LOAD_CHECKPOINT", LOAD_CHECKPOINT),
    ("SEQ_FOR_LINK_ANY", SEQ_FOR_LINK_ANY),
    ("BUMP_NEXT_SEQ", BUMP_NEXT_SEQ),
    ("INSERT_ITEM", INSERT_ITEM),
    ("UPDATE_ITEM", UPDATE_ITEM),
    ("RETAIN_ITEM", RETAIN_ITEM),
    ("DELETE_ITEM_BINDINGS", DELETE_ITEM_BINDINGS),
    ("RETAINED_ITEM", RETAINED_ITEM),
    ("REVIVE_ITEM", REVIVE_ITEM),
    ("LIST_RETAINED_PAGE", LIST_RETAINED_PAGE),
    ("ENSURE_INDEXES", ENSURE_INDEXES),
    ("COUNT_RETAINED", COUNT_RETAINED),
    ("RETAINED_BYTES", RETAINED_BYTES),
    ("RETAINED_ITEM_BY_SEQ", RETAINED_ITEM_BY_SEQ),
    ("PURGE_ITEM", PURGE_ITEM),
    ("RETAINED_BEFORE", RETAINED_BEFORE),
    ("PURGE_RETAINED_BEFORE", PURGE_RETAINED_BEFORE),
    ("INSERT_BINDING", INSERT_BINDING),
    ("UPDATE_BINDING", UPDATE_BINDING),
    ("DELETE_BINDING", DELETE_BINDING),
    ("ADJUST_REFCOUNT", ADJUST_REFCOUNT),
    ("RELEASE_PINS", RELEASE_PINS),
    ("UPSERT_CHECKPOINT", UPSERT_CHECKPOINT),
    ("STORE_OBJECT", STORE_OBJECT),
    ("LOOKUP_OBJECTS", LOOKUP_OBJECTS),
    ("LIST_GARBAGE_OBJECTS", LIST_GARBAGE_OBJECTS),
    ("LIST_OBJECT_HASHES", LIST_OBJECT_HASHES),
    ("DELETE_GARBAGE_OBJECTS", DELETE_GARBAGE_OBJECTS),
    ("RECOMPUTE_REFCOUNTS", RECOMPUTE_REFCOUNTS),
    ("DELETE_DANGLING_BINDINGS", DELETE_DANGLING_BINDINGS),
    ("ENQUEUE_ACTION", ENQUEUE_ACTION),
    ("LIST_QUEUED_COLLECTIONS", LIST_QUEUED_COLLECTIONS),
    ("LOAD_PENDING_ACTIONS", LOAD_PENDING_ACTIONS),
    ("CLAIM_ACTION", CLAIM_ACTION),
    ("LOAD_ACTION_ROW", LOAD_ACTION_ROW),
    ("CANCEL_ACTION", CANCEL_ACTION),
    ("PARK_ACTION", PARK_ACTION),
    ("BUMP_ATTEMPTS", BUMP_ATTEMPTS),
    ("LOAD_PARKED_ACTIONS", LOAD_PARKED_ACTIONS),
    ("BUMP_GENERATION", BUMP_GENERATION),
    ("LOAD_GENERATION", LOAD_GENERATION),
];

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::ALL;

    /// Every `pub const` this module declares, read from its own source.
    fn declared() -> Vec<&'static str> {
        include_str!("sql.rs")
            .lines()
            .filter_map(|line| line.strip_prefix("pub const "))
            .filter_map(|rest| rest.split(':').next())
            .map(str::trim)
            .collect()
    }

    #[test]
    fn the_index_covers_every_statement() {
        // NOTE: VERSION is an integer, not SQL, and ALL is itself a const in
        // the same shape; neither belongs in an index of statements.
        let expected: Vec<_> = declared()
            .into_iter()
            .filter(|name| *name != "VERSION" && *name != "ALL")
            .collect();

        assert!(!expected.is_empty(), "source scan found no constants");

        for name in &expected {
            assert!(
                ALL.iter().any(|(indexed, _)| indexed == name),
                "{name} is declared but missing from sql::ALL"
            );
        }
        assert_eq!(
            ALL.len(),
            expected.len(),
            "sql::ALL has entries the module does not declare"
        );
    }

    #[test]
    fn the_index_follows_the_declaration_order() {
        // Coverage alone would not catch an entry pairing one name with another
        // constant. Order does not prove the pairing either, but it keeps the
        // index a line-for-line mirror of the module, which is what makes a
        // wrong pairing visible on review rather than buried in an arbitrary
        // sequence. Two statements may legitimately share text, so comparing
        // texts proves nothing.
        let declared: Vec<_> = declared()
            .into_iter()
            .filter(|name| *name != "VERSION" && *name != "ALL")
            .collect();
        let indexed: Vec<_> = ALL.iter().map(|(name, _)| *name).collect();

        assert_eq!(
            indexed, declared,
            "sql::ALL drifted from the declaration order"
        );
    }

    #[test]
    fn no_statement_is_empty() {
        for (name, sql) in ALL {
            assert!(!sql.trim().is_empty(), "{name} is empty");
        }
    }
}
