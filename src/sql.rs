//! The canonical pimdir SQL, inlined verbatim from the spec so the crate is
//! self-contained. Kept in sync with `pimdir/migrations/0001_init.sql` and
//! `pimdir/queries/`; the spec is the source of truth.
//!
//! A store keeps one shared **item** per logical thing (its truth: flags, body,
//! summary), and one **binding** per source that syncs it (that source's last
//! agreed base). A single-source store is the degenerate case of one binding per
//! item; a two-source store (two servers, or a server and a phone) keeps two.

/// Schema version 1 (`migrations/0001_init.sql`). Applied to a fresh database;
/// the caller sets `PRAGMA user_version = 1` on success.
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
    conflict    TEXT NOT NULL DEFAULT 'manual'
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

pub const ENSURE_COLLECTION: &str = "\
INSERT INTO collections(id, kind, name) VALUES(:collection, '', :collection) \
ON CONFLICT(id) DO NOTHING";

pub const SET_COLLECTION_KIND: &str = "\
INSERT INTO collections(id, kind, name) VALUES(:collection, :kind, :collection) \
ON CONFLICT(id) DO UPDATE SET kind = excluded.kind";

pub const LOAD_KIND: &str = "SELECT kind FROM collections WHERE id = :collection";

pub const SET_CONFLICT: &str = "UPDATE collections SET conflict = :conflict WHERE id = :collection";

pub const LOAD_CONFLICT: &str = "SELECT conflict FROM collections WHERE id = :collection";

pub const LOAD_ITEMS: &str = "\
SELECT link_id, flags, object_hash, meta, level, deleted, conflicted, conflict_object \
FROM items WHERE collection = :collection";

// Client read surface (kind-agnostic, indexed getters over the same store the
// sync seam writes). Distinct from `LOAD_ITEMS`: paginated, live-only, ordered.

pub const LIST_COLLECTIONS: &str = "\
SELECT id, kind, name, parent, color, description, sort_order \
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

pub const COUNT_ITEMS: &str =
    "SELECT count(*) FROM items WHERE collection = :collection AND deleted = 0";

/// The distinct source names the store has synced (across all collections), so a
/// client can discover which source to attribute its writes to.
pub const LIST_SOURCES: &str = "SELECT DISTINCT source FROM bindings ORDER BY source";

pub const LOAD_BINDINGS: &str = "\
SELECT link_id, source, handle, base_flags, base_object, base_revision \
FROM bindings WHERE collection = :collection";

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

pub const UPSERT_CHECKPOINT: &str = "\
INSERT INTO sources(collection, source, checkpoint) VALUES(:collection, :source, :checkpoint) \
ON CONFLICT(collection, source) DO UPDATE SET checkpoint = excluded.checkpoint";

pub const STORE_OBJECT: &str = "\
INSERT INTO objects(hash, size, refcount) VALUES(:hash, :size, 0) \
ON CONFLICT(hash) DO UPDATE SET size = excluded.size";

pub const LOOKUP_OBJECTS: &str = "\
SELECT link_id, object_hash FROM items \
WHERE object_hash IS NOT NULL \
  AND link_id IN (SELECT value FROM json_each(:links))";

pub const LIST_GARBAGE_OBJECTS: &str = "SELECT hash FROM objects WHERE refcount = 0";

pub const DELETE_GARBAGE_OBJECTS: &str = "DELETE FROM objects WHERE refcount = 0";
