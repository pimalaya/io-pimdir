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

CREATE TABLE collections (
    id          TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,
    name        TEXT NOT NULL,
    parent      TEXT REFERENCES collections(id) ON DELETE SET NULL,
    color       TEXT,
    description TEXT,
    sort_order  INTEGER,
    -- Cross-source content-conflict policy: 'manual' | 'prefer-incoming' | 'prefer-existing'.
    conflict    TEXT NOT NULL DEFAULT 'manual',
    -- Collection generation: bumped by the owner whenever it rebuilds the
    -- collection's handle space (a backend identity reset), so a reader can derive
    -- epoch-dependent protocol values (an IMAP UIDVALIDITY) from the store alone
    -- (SPEC.md §15).
    generation  INTEGER NOT NULL DEFAULT 1
) STRICT;

-- One row per source that syncs a collection (a server, a phone). A
-- single-source collection has one row here.
CREATE TABLE sources (
    collection TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
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
-- it too (the cross-source delete memory).
CREATE TABLE items (
    collection      TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    link_id         TEXT NOT NULL,
    -- The message's public id: store-global, one per link_id (shared by its
    -- placements across mailboxes), never reused. A client shows it and resolves
    -- it back to `link_id`.
    seq             INTEGER NOT NULL,
    flags           TEXT,
    object_hash     TEXT REFERENCES objects(hash),
    meta            TEXT,
    level           INTEGER NOT NULL,
    deleted         INTEGER NOT NULL DEFAULT 0,
    conflicted      INTEGER NOT NULL DEFAULT 0,
    conflict_object TEXT REFERENCES objects(hash),
    PRIMARY KEY (collection, link_id)
) STRICT;

-- One source's binding of an item: its handle there and the base last synced
-- with it (the 3-way-merge baseline).
CREATE TABLE bindings (
    collection    TEXT NOT NULL,
    link_id       TEXT NOT NULL,
    source        TEXT NOT NULL,
    handle        TEXT NOT NULL,
    base_flags    TEXT,
    base_object   TEXT REFERENCES objects(hash),
    base_revision TEXT,
    PRIMARY KEY (collection, link_id, source),
    FOREIGN KEY (collection, link_id) REFERENCES items(collection, link_id) ON DELETE CASCADE
) STRICT;

-- The action queue (SPEC.md §14): mutations requested by processes that are not
-- the store owner, applied by the owner in append order.
CREATE TABLE queue (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,  -- global append order
    created_at  TEXT    NOT NULL,                   -- RFC 3339 timestamp
    producer    TEXT    NOT NULL,                   -- enqueuing process, diagnostic only
    collection  TEXT    NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    action      TEXT    NOT NULL,                   -- 'add' | 'set-flags' | 'remove' | 'move' | 'copy' | 'update'
    payload     TEXT    NOT NULL,                   -- versioned JSON, shape per action (SPEC.md §14)
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
"#;

/// The current schema version.
pub const VERSION: i64 = 1;

/// Creates a collection row if it does not exist yet, leaving an existing one
/// untouched (the kind is declared separately by `SET_COLLECTION_KIND`).
pub const ENSURE_COLLECTION: &str = "\
INSERT INTO collections(id, kind, name) VALUES(:collection, '', :collection) \
ON CONFLICT(id) DO NOTHING";

/// Declares (or re-declares) a collection's kind, creating the row if the
/// collection is not known yet.
pub const SET_COLLECTION_KIND: &str = "\
INSERT INTO collections(id, kind, name) VALUES(:collection, :kind, :collection) \
ON CONFLICT(id) DO UPDATE SET kind = excluded.kind";

/// Reads a collection's declared kind.
pub const LOAD_KIND: &str = "SELECT kind FROM collections WHERE id = :collection";

/// Stores a collection's conflict policy.
pub const SET_CONFLICT: &str = "UPDATE collections SET conflict = :conflict WHERE id = :collection";

/// Reads a collection's conflict policy.
pub const LOAD_CONFLICT: &str = "SELECT conflict FROM collections WHERE id = :collection";

/// Loads a whole collection for the sync seam: every item, tombstones
/// included, unpaginated and unordered.
pub const LOAD_ITEMS: &str = "\
SELECT link_id, flags, object_hash, meta, level, deleted, conflicted, conflict_object \
FROM items WHERE collection = :collection";

// Client read surface (kind-agnostic, indexed getters over the same store the
// sync seam writes). Distinct from `LOAD_ITEMS`: paginated, live-only, ordered.

/// Lists every collection with its display metadata and generation, ordered by
/// `sort_order` then id, the ones carrying no sort order coming last.
pub const LIST_COLLECTIONS: &str = "\
SELECT id, kind, name, parent, color, description, sort_order, generation \
FROM collections ORDER BY sort_order IS NULL, sort_order, id";

/// A keyset page of a collection's live items. `:after` is the exclusive lower
/// bound on `link_id` (the empty string starts from the beginning, since a
/// `link_id` is never empty); rides the `items` primary key, no extra index.
pub const LIST_ITEMS_PAGE: &str = "\
SELECT seq, link_id, flags, object_hash, meta, level FROM items \
WHERE collection = :collection AND deleted = 0 AND link_id > :after \
ORDER BY link_id LIMIT :limit";

/// Fetches one live item by its public id (`seq`) — the client-facing key.
pub const GET_ITEM: &str = "\
SELECT seq, link_id, flags, object_hash, meta, level FROM items \
WHERE collection = :collection AND seq = :seq AND deleted = 0";

/// Resolves an item's public id (`seq`) from its internal `link_id` — the inverse
/// of `GET_ITEM`, for a consumer that just staged an add and wants the new id.
pub const SEQ_BY_LINK: &str =
    "SELECT seq FROM items WHERE collection = :collection AND link_id = :link_id";

/// Counts a collection's live items (tombstones excluded).
pub const COUNT_ITEMS: &str =
    "SELECT count(*) FROM items WHERE collection = :collection AND deleted = 0";

/// The distinct source names the store has synced (across all collections), so a
/// client can discover which source to attribute its writes to.
pub const LIST_SOURCES: &str = "SELECT DISTINCT source FROM bindings ORDER BY source";

/// Loads every per-source binding of a collection: the stored base (handle,
/// flags, object, revision) each sync merges against.
pub const LOAD_BINDINGS: &str = "\
SELECT link_id, source, handle, base_flags, base_object, base_revision \
FROM bindings WHERE collection = :collection";

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
INSERT INTO items(collection, link_id, seq, flags, object_hash, meta, level, deleted, conflicted, conflict_object) \
VALUES(:collection, :link_id, :seq, :flags, :object_hash, :meta, :level, :deleted, :conflicted, :conflict_object)";

/// Updates one existing item's columns in place (the diffed-save path; the
/// primary key `(collection, link_id)` is unchanged).
pub const UPDATE_ITEM: &str = "\
UPDATE items SET flags = :flags, object_hash = :object_hash, meta = :meta, \
level = :level, deleted = :deleted, conflicted = :conflicted, conflict_object = :conflict_object \
WHERE collection = :collection AND link_id = :link_id";

/// Deletes one item; its bindings cascade (`PRAGMA foreign_keys = ON`).
pub const DELETE_ITEM: &str =
    "DELETE FROM items WHERE collection = :collection AND link_id = :link_id";

/// Inserts one item's binding for one source (the new-binding path;
/// `UPDATE_BINDING` handles an existing one).
pub const INSERT_BINDING: &str = "\
INSERT INTO bindings(collection, link_id, source, handle, base_flags, base_object, base_revision) \
VALUES(:collection, :link_id, :source, :handle, :base_flags, :base_object, :base_revision)";

/// Updates one existing binding's columns in place (its primary key
/// `(collection, link_id, source)` is unchanged).
pub const UPDATE_BINDING: &str = "\
UPDATE bindings SET handle = :handle, base_flags = :base_flags, \
base_object = :base_object, base_revision = :base_revision \
WHERE collection = :collection AND link_id = :link_id AND source = :source";

/// Deletes one source's binding of an item.
pub const DELETE_BINDING: &str = "DELETE FROM bindings WHERE collection = :collection AND link_id = :link_id AND source = :source";

/// Adjusts one object's refcount by a signed delta (the incremental-refcount
/// path); the hash's primary key makes this an indexed point update.
pub const ADJUST_REFCOUNT: &str =
    "UPDATE objects SET refcount = refcount + :delta WHERE hash = :hash";

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
pub const LOOKUP_OBJECTS: &str = "\
SELECT link_id, object_hash FROM items \
WHERE object_hash IS NOT NULL \
  AND link_id IN (SELECT value FROM json_each(:links))";

/// Lists the objects no placement references any more: the blobs the write
/// transaction is about to collect.
pub const LIST_GARBAGE_OBJECTS: &str = "SELECT hash FROM objects WHERE refcount = 0";

/// Drops the unreferenced object rows inside the write transaction; their
/// blobs are unlinked after the commit, so a crash leaves at worst an orphan
/// blob.
pub const DELETE_GARBAGE_OBJECTS: &str = "DELETE FROM objects WHERE refcount = 0";

// The action queue (spec §14, `queries/queue.sql`): the write door for every
// process that is not the store owner. A producer appends; the owner applies
// pending actions in append order and deletes each in the same transaction as
// its effects.

/// A producer's append. Runs after `ENSURE_COLLECTION`, in one transaction with
/// the `STORE_OBJECT` upsert when the payload references a body (spec §14.1).
pub const ENQUEUE_ACTION: &str = "\
INSERT INTO queue(created_at, producer, collection, action, payload, object_hash) \
VALUES(:created_at, :producer, :collection, :action, :payload, :object_hash)";

/// The collections with pending work, for the owner's drain loop.
pub const LIST_QUEUED_COLLECTIONS: &str =
    "SELECT DISTINCT collection FROM queue WHERE error IS NULL";

/// The owner's drain: a collection's pending (non-parked) actions, in append
/// order. A reader runs the same statement to overlay pending actions on its
/// item projection (read-your-writes, spec §14.4).
pub const LOAD_PENDING_ACTIONS: &str = "\
SELECT id, created_at, producer, action, payload, object_hash, attempts \
FROM queue WHERE collection = :collection AND error IS NULL ORDER BY id";

/// An applied action: deleted in the same transaction as its item and binding
/// writes, so applying is exactly-once.
pub const DELETE_ACTION: &str = "DELETE FROM queue WHERE id = :id";

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

/// The owner's handle-space reset marker (spec §15): run in the same
/// transaction as the rebuild it records.
pub const BUMP_GENERATION: &str = "\
UPDATE collections SET generation = generation + 1 WHERE id = :collection \
RETURNING generation";

/// A collection's handle-space epoch, so a reader derives epoch-dependent
/// protocol values (an IMAP UIDVALIDITY) from the store alone.
pub const LOAD_GENERATION: &str = "SELECT generation FROM collections WHERE id = :collection";
