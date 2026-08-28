//! The canonical pimdir SQL, inlined verbatim from the spec so the crate
//! is self-contained. Kept in sync with `pimdir/migrations/` and
//! `pimdir/queries/`, where the source of truth is.
//!
//! A store keeps one shared item per logical thing (its flags, body and
//! summary) and one binding per source that syncs it (that source's last
//! agreed base). A single-source store is the degenerate case of one
//! binding per item; a two-source store keeps two.

/// The current schema version.
pub const VERSION: i64 = 1;

/// Indexes an earlier draft created under the same name over different
/// columns, as `(name, the columns it must hold now)`.
///
/// [`ENSURE_INDEXES`] cannot repair one: `CREATE INDEX IF NOT EXISTS`
/// keys on the name, so it leaves the old shape in place and the store
/// keeps planning the read the schema no longer says. Such an index is
/// dropped on open when its columns disagree, then recreated.
///
/// Checked rather than dropped unconditionally, since rebuilding a large
/// store's index on every open is the cost this exists to avoid.
pub const RESHAPED_INDEXES: &[(&str, &[&str])] = &[
    // NOTE: was (collection, retained_at) while `list_retained_page`
    // still paged by `link_id`. A store keeping the old one sorts every
    // retained row of the collection to return one page.
    ("items_retained", &["collection", "seq"]),
];

/// Declares the module's statements and the [`ALL`] index in one
/// expansion, so a new statement is added in one place rather than three.
macro_rules! statements {
    ($($(#[$doc:meta])* $name:ident = $sql:expr;)*) => {
        $($(#[$doc])* pub const $name: &str = $sql;)*

        /// Every statement in this module, paired with its constant name.
        ///
        /// How a consumer without the `client` feature reaches the
        /// canonical SQL: it holds its own SQLite driver, so it needs the
        /// statements by name rather than a Rust accessor each.
        /// [`MIGRATION_0001`] is included, creating the database being as
        /// much a consumer's job as querying it; [`VERSION`] is not,
        /// being an integer.
        pub const ALL: &[(&str, &str)] = &[$((stringify!($name), $name)),*];
    };
}

// NOTE: the statements below are not indented into the macro invocation:
// half of them are raw strings holding the spec's SQL verbatim, and
// indenting would rewrite that text.
statements! {
/// Schema version 1 (`migrations/0001_init.sql`), the whole draft schema
/// including the action queue and collection generations. Applied to a fresh
/// database; the caller sets `PRAGMA user_version = 1` on success.
MIGRATION_0001 = r#"
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

-- One source's binding of an item: its handle there, the two bases it agreed
-- from (the one last synced with the source, which is the 3-way-merge baseline,
-- and the shared body it last reconciled against), and whether that source's
-- own sync is stuck on an unresolved content conflict.
CREATE TABLE bindings (
    collection    TEXT NOT NULL,
    link_id       TEXT NOT NULL,
    source        TEXT NOT NULL,
    -- The item's backend id on this source (IMAP UID, DAV href). Bound once: a
    -- write resolving this binding to another handle is refused, and the one
    -- licensed rebind is the handle-space rebuild (SPEC.md §10, §12).
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
    -- The diverging remote body at that revision, so a resolver reads the
    -- three sides (base, local, remote) from the store and needs no
    -- credentials. Pinned like any other reference while the binding stays
    -- conflicted, and released when it resolves.
    conflict_object   TEXT REFERENCES objects(hash),
    -- The shared body this source last reconciled against, the base of the
    -- cross-source merge. base_object answers to the source's own remote and
    -- only a sync moves it, so a body this source folded in and has not pushed
    -- yet leaves it behind; read as the shared base it would have the source
    -- disagree with itself. Meaningful on every binding, conflicted or not.
    -- It names an object and pins none, hence no REFERENCES, no index and no
    -- refcount: the value is only ever compared for equality, never read as
    -- bytes, and a content hash compares the same after the body it named has
    -- been swept.
    shared_object     TEXT,
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
-- Retained (soft-deleted) items: every retained read rides this one index, and
-- none of them touches the live rows, which are the overwhelming majority. It
-- leads with `seq` because the trash listing pages on the public id
-- (LIST_RETAINED_PAGE, spec §14.1), and ordering by anything this index does not
-- lead with sorts every retained row in the collection to return one page.
-- COUNT_RETAINED rides the collection prefix, and the store-wide purge scans the
-- index whole, which is O(retained) because the index is partial.
CREATE INDEX items_retained ON items(collection, seq) WHERE retained_at IS NOT NULL;
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
-- The other three pointers at an object, so a refcount recomputation reaches
-- every reference by index rather than by scanning items, bindings and queue
-- once per object.
CREATE INDEX items_by_conflict_object ON items(conflict_object);
CREATE INDEX bindings_by_conflict_object ON bindings(conflict_object);
CREATE INDEX queue_by_object ON queue(object_hash);
-- The bindings waiting for a decision. Partial, so it holds only what is
-- outstanding and is empty at rest: a run reports that count on every
-- invocation, and a listing command asks the same question directly, both of
-- which would otherwise scan every binding in the store.
CREATE INDEX bindings_conflicted ON bindings(collection, link_id, source) WHERE conflicted = 1;
-- Resolves one source handle back to the link id it is bound to, which is what
-- a batch dropping a placement needs: a drop names a handle and the shared item
-- is keyed by link id. Without it that resolution is a scan of every item.
CREATE INDEX bindings_by_handle ON bindings(collection, source, handle);
"#;


/// Creates a collection row if it does not exist yet, leaving an existing one
/// untouched (the kind is declared separately by `SET_COLLECTION_KIND`).
ENSURE_COLLECTION = "\
INSERT INTO collections(id, account, kind, name) VALUES(:collection, :account, '', :collection) \
ON CONFLICT(id) DO NOTHING";

/// Declares (or re-declares) a collection's kind, creating the row if the
/// collection is not known yet. Updates the kind alone, so a collection never
/// changes account as a side effect of a sync declaring its media type.
SET_COLLECTION_KIND = "\
INSERT INTO collections(id, account, kind, name) VALUES(:collection, :account, :kind, :collection) \
ON CONFLICT(id) DO UPDATE SET kind = excluded.kind";

/// Regroups a collection under another account, or out of one with `NULL`. Safe
/// at any time: the account partitions no identifier (spec §9.2), so the move
/// leaves seqs, link ids and objects alone.
SET_COLLECTION_ACCOUNT =
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
RENAME_COLLECTION = "UPDATE collections SET id = :new_id WHERE id = :collection";

/// Reads a collection's owning account.
LOAD_ACCOUNT = "SELECT account FROM collections WHERE id = :collection";

/// Reads a collection's declared kind.
LOAD_KIND = "SELECT kind FROM collections WHERE id = :collection";

/// Stores a collection's conflict policy.
SET_CONFLICT = "UPDATE collections SET conflict = :conflict WHERE id = :collection";

/// Reads a collection's conflict policy.
LOAD_CONFLICT = "SELECT conflict FROM collections WHERE id = :collection";

/// Loads a whole collection for the sync seam: every item, tombstones
/// included, unpaginated and unordered.
///
/// Retained (soft-deleted) rows are excluded, which is what makes
/// retention safe under io-replica's contract: the merge reconciles only
/// what `load` returns, so a hidden row is never re-derived.
///
/// `sort_key` rides along so the round trip preserves it: the engine
/// carries the key on a placement, so a load that dropped it would hand
/// every save an unknown key and erase on every sync what the last one
/// derived (spec §9.3).
LOAD_ITEMS = "\
SELECT link_id, flags, object_hash, meta, sort_key, level, deleted, conflicted, conflict_object \
FROM items WHERE collection = :collection AND retained_at IS NULL";

/// The same rows, narrowed to the link ids one write batch touches (spec §14).
///
/// A write folds its batch into the hub and persists the difference, and
/// that difference only names rows the batch named: reading the rest
/// costs a full pass over the collection to compute nothing, growing
/// with the mailbox rather than with the batch.
LOAD_ITEMS_BY_LINK = "\
SELECT link_id, flags, object_hash, meta, sort_key, level, deleted, conflicted, conflict_object \
FROM items WHERE collection = :collection AND retained_at IS NULL \
  AND link_id IN (SELECT value FROM json_each(:links))";

// Client read surface (kind-agnostic, indexed getters over the same store the
// sync seam writes). Distinct from `LOAD_ITEMS`: paginated, live-only, ordered.

/// Lists every collection with its display metadata and generation, ordered by
/// `sort_order` then id, the ones carrying no sort order coming last.
LIST_COLLECTIONS = "\
SELECT id, account, kind, name, parent, color, description, sort_order, generation \
FROM collections ORDER BY sort_order IS NULL, sort_order, id";

/// One account's collections, the filter axis of a merged view. `IS` so binding
/// `NULL` selects the collections of a single-account store.
LIST_COLLECTIONS_BY_ACCOUNT = "\
SELECT id, account, kind, name, parent, color, description, sort_order, generation \
FROM collections WHERE account IS :account ORDER BY sort_order IS NULL, sort_order, id";

/// The accounts owning at least one collection. A store knows an account only
/// through its collections (spec §9.2), so this is not a configured roster.
LIST_ACCOUNTS = "\
SELECT DISTINCT account FROM collections WHERE account IS NOT NULL ORDER BY account";

/// A keyset page of a collection's live items in link-id order. `:after`
/// is the exclusive lower bound on `link_id`, the empty string starting
/// from the beginning since a `link_id` is never empty; rides the `items`
/// primary key, with no extra index.
///
/// Link-id order means nothing to a reader: this is the page for a sweep
/// that must see every item exactly once. A reader presenting a list
/// wants one of the two ordered pages below.
LIST_ITEMS_PAGE = "\
SELECT seq, link_id, flags, object_hash, meta, sort_key, level FROM items \
WHERE collection = :collection AND deleted = 0 AND link_id > :after \
ORDER BY link_id LIMIT :limit";

/// A keyset page of a collection's live items in the kind's own
/// ascending order (spec §9.3): A to Z for contacts, earliest first for
/// mail and calendars.
///
/// The cursor is the pair `(:after_key, :after_seq)`, because a sort key
/// is not unique: two messages share a timestamp, two contacts a name.
/// `seq` breaks the tie and, being unique per collection, makes the page
/// total. The empty string with seq 0 starts from the beginning, since
/// no real key sorts before an unknown one ascending.
LIST_ITEMS_PAGE_ASC = "\
SELECT seq, link_id, flags, object_hash, meta, sort_key, level FROM items \
WHERE collection = :collection AND deleted = 0 \
AND (sort_key, seq) > (:after_key, :after_seq) \
ORDER BY sort_key, seq LIMIT :limit";

/// The same page descending: newest first for mail and calendars, Z to A
/// for contacts.
///
/// The first page binds a NULL cursor rather than a key above every other
/// one: a sort key is arbitrary text a writer derives, so no value is
/// reserved and "the largest key the store can hold" is not expressible.
/// A sentinel would hide everything sorting above it from every
/// descending page, for good. The comparison stays a keyset one, so the
/// index still serves it.
LIST_ITEMS_PAGE_DESC = "\
SELECT seq, link_id, flags, object_hash, meta, sort_key, level FROM items \
WHERE collection = :collection AND deleted = 0 \
AND (:after_key IS NULL OR (sort_key, seq) < (:after_key, :after_seq)) \
ORDER BY sort_key DESC, seq DESC LIMIT :limit";

/// Restates one item's ordering key, for a re-projection over items
/// already stored: a store written before its kind had a convention, one
/// whose convention changed, or a consumer whose sync engine does not
/// carry the key inline (spec §9.3). Not the ordinary write path.
SET_SORT_KEY = "\
UPDATE items SET sort_key = :sort_key \
WHERE collection = :collection AND link_id = :link_id";

/// Fetches one live item by its public id (`seq`), the client-facing key.
GET_ITEM = "\
SELECT seq, link_id, flags, object_hash, meta, sort_key, level FROM items \
WHERE collection = :collection AND seq = :seq AND deleted = 0";

/// Resolves an item's public id (`seq`) from its internal `link_id`, the
/// inverse of `GET_ITEM`, for a consumer that just staged an add.
SEQ_BY_LINK =
    "SELECT seq FROM items WHERE collection = :collection AND link_id = :link_id";

/// Counts a collection's live items (tombstones excluded).
COUNT_ITEMS =
    "SELECT count(*) FROM items WHERE collection = :collection AND deleted = 0";

/// Every live placement of one identity, with the collection and account
/// it sits in (spec §9.2). The store reports where a link id occurs and
/// takes no position on whether the placements are one thing: a mail view
/// lists them, a contact view may offer to merge them, off these rows.
LIST_LINK_PLACEMENTS = "\
SELECT i.collection, c.account, i.seq, i.object_hash, i.flags, i.level \
FROM items i JOIN collections c ON c.id = i.collection \
WHERE i.link_id = :link_id AND i.deleted = 0 AND i.retained_at IS NULL \
ORDER BY c.account IS NULL, c.account, i.collection";

/// The same on the dedup axis, by body rather than identity, so it pairs
/// placements two servers gave different link ids.
LIST_OBJECT_PLACEMENTS = "\
SELECT i.collection, c.account, i.seq, i.link_id, i.flags, i.level \
FROM items i JOIN collections c ON c.id = i.collection \
WHERE i.object_hash = :hash AND i.deleted = 0 AND i.retained_at IS NULL \
ORDER BY c.account IS NULL, c.account, i.collection";

/// The distinct source names the store has synced, across all
/// collections, so a client discovers which source to attribute writes
/// to.
LIST_SOURCES = "SELECT DISTINCT source FROM bindings ORDER BY source";

/// Loads every per-source binding of a collection: the stored base (handle,
/// flags, object, revision) each sync merges against.
LOAD_BINDINGS = "\
SELECT link_id, source, handle, base_flags, base_object, base_revision, base_present, \
conflicted, conflict_revision, conflict_object, shared_object \
FROM bindings WHERE collection = :collection";

/// The same rows, narrowed to the link ids one write batch touches: the binding
/// half of [`LOAD_ITEMS_BY_LINK`].
LOAD_BINDINGS_BY_LINK = "\
SELECT link_id, source, handle, base_flags, base_object, base_revision, base_present, \
conflicted, conflict_revision, conflict_object, shared_object \
FROM bindings WHERE collection = :collection \
  AND link_id IN (SELECT value FROM json_each(:links))";

/// The bindings waiting for a decision, across an account's collections:
/// what each one is, and the three bodies a resolver merges.
///
/// The base is the last state the two sides agreed on, the item's own
/// `object_hash` is the local side, and `conflict_object` is the remote
/// one at `conflict_revision`. All three come off the one row, so a
/// resolver holding no credentials reads the whole divergence from the
/// store.
///
/// Scoped to one account with `IS`, so binding `NULL` lists a
/// single-account store whole. Rides the partial index
/// `bindings_conflicted`, which holds only the outstanding rows: the
/// question is asked at the end of every run, and answering it by paging
/// each collection costs a pass over the whole store to report a number
/// that is usually zero.
LIST_CONFLICTED_BINDINGS = "\
SELECT b.collection, b.link_id, b.source, b.handle, b.conflict_revision, \
b.base_object, i.object_hash, b.conflict_object \
FROM bindings b \
JOIN items i ON i.collection = b.collection AND i.link_id = b.link_id \
JOIN collections c ON c.id = b.collection \
WHERE b.conflicted = 1 AND c.account IS :account \
ORDER BY b.collection, b.link_id, b.source";

/// Whether a collection holds a live item under a link id: the collision
/// check a queued `add` runs before staging.
///
/// A point read on the items primary key, because it runs once per
/// drained action: answering it by loading the collection would make a
/// drain of N actions cost N passes over the mailbox.
LIVE_ITEM_FOR_LINK = "\
SELECT seq FROM items \
WHERE collection = :collection AND link_id = :link_id \
  AND deleted = 0 AND retained_at IS NULL";

/// One source's handle for an item, which its binding's primary key answers
/// directly: the lookup a queued action needs to name the placement it edits.
HANDLE_FOR_LINK = "\
SELECT handle FROM bindings \
WHERE collection = :collection AND link_id = :link_id AND source = :source";

/// The link id one source's handle is bound to, for a batch that drops a
/// placement: a drop names a handle, and the hub is keyed by link id.
///
/// Served by the `bindings_by_handle` index, so resolving it is a seek
/// rather than a scan over every item.
LINK_FOR_HANDLE = "\
SELECT link_id FROM bindings \
WHERE collection = :collection AND source = :source AND handle = :handle";

/// Reads one source's sync checkpoint for a collection.
LOAD_CHECKPOINT =
    "SELECT checkpoint FROM sources WHERE collection = :collection AND source = :source";

/// The message's existing public id, if any placement of this `link_id` already
/// has one (in any collection), so all placements of a message share one id.
SEQ_FOR_LINK_ANY = "SELECT seq FROM items WHERE link_id = :link_id LIMIT 1";

/// Hands out, and advances, the store-global next public id via
/// `RETURNING`. The counter only ever increases, so a `seq` is never
/// reused. Run only when the message has no id yet.
BUMP_NEXT_SEQ =
    "UPDATE store_meta SET next_seq = next_seq + 1 WHERE id = 1 RETURNING next_seq - 1";

/// Inserts one item row (the new-placement path; `UPDATE_ITEM` handles an
/// existing one).
INSERT_ITEM = "\
INSERT INTO items(collection, link_id, seq, flags, object_hash, meta, sort_key, level, deleted, conflicted, conflict_object) \
VALUES(:collection, :link_id, :seq, :flags, :object_hash, :meta, :sort_key, :level, :deleted, :conflicted, :conflict_object)";

/// Updates one existing item's columns in place (the diffed-save path; the
/// primary key `(collection, link_id)` is unchanged).
UPDATE_ITEM = "\
UPDATE items SET flags = :flags, object_hash = :object_hash, meta = :meta, sort_key = :sort_key, \
level = :level, deleted = :deleted, conflicted = :conflicted, conflict_object = :conflict_object \
WHERE collection = :collection AND link_id = :link_id";

// Retention (spec §11): the last binding vanishing retires the row instead of
// deleting it, a reappearing link id revives it, and purge is the only true
// delete.

/// Retires one item: it stands exactly where a hard-deleting store would
/// have issued its delete. The row keeps its `object_hash`, so the body
/// keeps its reference and its blob survives the sweep. SQLite stamps the
/// instant itself, so no clock is plumbed through the crate; a purge's
/// cutoff is by contrast the caller's parameter.
RETAIN_ITEM = "\
UPDATE items SET deleted = 1, \
retained_at = strftime('%Y-%m-%dT%H:%M:%fZ','now'), retained_by = :source \
WHERE collection = :collection AND link_id = :link_id";

/// Deletes every binding of one item, for the retire path: the row
/// survives, but no source holds it, so no base does either. A retained
/// row carrying no binding is the persisted form of "the removal has
/// finished propagating" (spec §11).
DELETE_ITEM_BINDINGS =
    "DELETE FROM bindings WHERE collection = :collection AND link_id = :link_id";

/// The retained row holding a link id, if any: its public id and the objects it
/// pins, which revive releases and purge reclaims.
RETAINED_ITEM = "\
SELECT seq, object_hash, conflict_object FROM items \
WHERE collection = :collection AND link_id = :link_id AND retained_at IS NOT NULL";

/// Revives a retained row: the link id is back, from a source-side
/// resurrection or a client `add`, so it stops being retained instead of
/// conflicting on the primary key. The caller adopts the new content with
/// `UPDATE_ITEM` in the same transaction, and the row keeps its `seq`.
REVIVE_ITEM = "\
UPDATE items SET deleted = 0, retained_at = NULL, retained_by = NULL \
WHERE collection = :collection AND link_id = :link_id";

/// A keyset page of a collection's retained items, joined to the body size the
/// row still pins (`NULL` when unhydrated): the trash listing beside
/// `LIST_ITEMS_PAGE`, and the only read that returns them.
///
/// `:after` is the exclusive lower bound on the public `seq`, 0 starting
/// from the beginning: a real sentinel rather than an invented one, since
/// `seq` is handed out from 1. A caller pages the trash by the same small
/// integer it purges and restores by.
LIST_RETAINED_PAGE = "\
SELECT i.seq, i.link_id, i.flags, i.object_hash, i.meta, i.sort_key, i.level, \
i.retained_at, i.retained_by, o.size \
FROM items i LEFT JOIN objects o ON o.hash = i.object_hash \
WHERE i.collection = :collection AND i.retained_at IS NOT NULL AND i.seq > :after \
ORDER BY i.seq LIMIT :limit";

/// Every index the schema grew after version 1 was first published, as one
/// idempotent batch: a store written by an earlier draft has the tables but not
/// these, and an index is not something a reader can do without.
///
/// Run on open rather than only when a column is missing, because most of
/// these index columns that were always there: what changed is that a
/// statement now needs them. A store keeping the old plans would scan
/// where the schema says it seeks.
///
/// `IF NOT EXISTS` keys on the name, so an index whose columns changed is
/// not replaced by this batch and has to be dropped first (see
/// [`RESHAPED_INDEXES`]).
ENSURE_INDEXES = "\
CREATE INDEX IF NOT EXISTS items_retained ON items(collection, seq) \
WHERE retained_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS collections_by_account ON collections(account) \
WHERE account IS NOT NULL;
CREATE INDEX IF NOT EXISTS items_by_sort ON items(collection, sort_key, seq);
CREATE INDEX IF NOT EXISTS items_by_seq_global ON items(seq);
CREATE INDEX IF NOT EXISTS objects_garbage ON objects(refcount) WHERE refcount <= 0;
CREATE INDEX IF NOT EXISTS items_by_conflict_object ON items(conflict_object);
CREATE INDEX IF NOT EXISTS bindings_by_conflict_object ON bindings(conflict_object);
CREATE INDEX IF NOT EXISTS bindings_conflicted ON bindings(collection, link_id, source) \
WHERE conflicted = 1;
CREATE INDEX IF NOT EXISTS queue_by_object ON queue(object_hash);
CREATE INDEX IF NOT EXISTS bindings_by_handle ON bindings(collection, source, handle);";

/// Counts a collection's retained items, the counterpart of `COUNT_ITEMS`;
/// rides the `items_retained` partial index.
COUNT_RETAINED =
    "SELECT count(*) FROM items WHERE collection = :collection AND retained_at IS NOT NULL";

/// The store-wide size of the bodies retention is holding, each distinct
/// object counted once. An upper bound on what a purge reclaims: an
/// object a live item also points at keeps a reference and survives.
RETAINED_BYTES = "\
SELECT coalesce(sum(o.size), 0) FROM objects o WHERE o.hash IN \
(SELECT object_hash FROM items WHERE retained_at IS NOT NULL AND object_hash IS NOT NULL)";

/// Purges one retained item by its public id: the only true delete. Its
/// bindings cascade, and the body it released is unlinked by the
/// collector once nothing else references it. Guarded on `retained_at`,
/// so a purge can never take a live item.
///
/// Returns the two hashes the row pinned, so the caller settles them with
/// [`RELEASE_PINS`] in the same transaction rather than visiting the row
/// twice.
PURGE_ITEM = "\
DELETE FROM items WHERE collection = :collection AND seq = :seq AND retained_at IS NOT NULL \
RETURNING object_hash, conflict_object";

/// The time-based sweep: every item retired strictly before `:cutoff`
/// (RFC 3339), so one retained exactly at that instant is kept.
/// Store-wide, since how long to keep is the owner's policy. The cutoff
/// is the caller's parameter, not the store's clock, so the boundary is
/// deterministic even though the stamps are SQLite's.
///
/// Returns each purged row's two pinned hashes, on the same terms as
/// [`PURGE_ITEM`]: this is where visiting the rows twice costs most,
/// being the sweep that takes fifty thousand at once.
PURGE_RETAINED_BEFORE = "\
DELETE FROM items WHERE retained_at IS NOT NULL AND retained_at < :cutoff \
RETURNING object_hash, conflict_object";

/// Inserts one item's binding for one source (the new-binding path;
/// `UPDATE_BINDING` handles an existing one).
INSERT_BINDING = "\
INSERT INTO bindings(collection, link_id, source, handle, base_flags, base_object, \
base_revision, base_present, conflicted, conflict_revision, conflict_object, \
shared_object) \
VALUES(:collection, :link_id, :source, :handle, :base_flags, :base_object, \
:base_revision, :base_present, :conflicted, :conflict_revision, :conflict_object, \
:shared_object)";

/// Updates one existing binding's columns in place (its primary key
/// `(collection, link_id, source)` is unchanged).
///
/// `handle` is deliberately not among them, and cannot be. A binding
/// pins one handle, and repointing it would destroy the evidence that a
/// source holds an identity twice, before any later rule could act on it.
/// A write resolving this binding to another handle is refused instead
/// (spec §10), the second copy having a key and an item of its own (spec
/// §9). A legitimate rebind, after a handle-space change, goes through
/// the rebuild that drops the old spine and inserts the new one.
UPDATE_BINDING = "\
UPDATE bindings SET base_flags = :base_flags, \
base_object = :base_object, base_revision = :base_revision, base_present = :base_present, \
conflicted = :conflicted, conflict_revision = :conflict_revision, \
conflict_object = :conflict_object, shared_object = :shared_object \
WHERE collection = :collection AND link_id = :link_id AND source = :source";

/// Gives every binding written before `shared_object` existed the item's
/// own body as its agreement point, once the column has been added
/// (spec §6, the `draft` allowance).
///
/// Left empty the column reads as "this source has never folded", which
/// falls back to the sync base, and a binding whose push is pending sits
/// behind the shared body by definition: the first absorb after the
/// upgrade would then measure the cross-source axis from the base again
/// and file the source's own next edit as a divergence. An existing
/// store's sources agree with the body they hold, so that body is what
/// the rows already imply.
///
/// Guarded on `IS NULL`, which is every row of a column just added and
/// no row of one already backfilled, so running it twice is a no-op.
BACKFILL_SHARED_OBJECT = "\
UPDATE bindings SET shared_object = \
(SELECT object_hash FROM items \
 WHERE items.collection = bindings.collection AND items.link_id = bindings.link_id) \
WHERE shared_object IS NULL";

/// Deletes one source's binding of an item.
DELETE_BINDING = "DELETE FROM bindings WHERE collection = :collection AND link_id = :link_id AND source = :source";

/// Adjusts one object's refcount by a signed delta; the hash's primary
/// key makes this an indexed point update.
ADJUST_REFCOUNT =
    "UPDATE objects SET refcount = refcount + :delta WHERE hash = :hash";

/// Releases one reference from each of the given hashes (a JSON array), the
/// set-based form of [`ADJUST_REFCOUNT`] at `-1`.
///
/// A hash listed twice releases twice, which is what makes it the same
/// operation as the loop it replaces: a retained item pins its body and
/// its conflict body separately, and a purge releases both.
RELEASE_PINS = "\
UPDATE objects SET refcount = refcount - \
  (SELECT count(*) FROM json_each(:hashes) WHERE value = objects.hash) \
WHERE hash IN (SELECT value FROM json_each(:hashes))";

/// Writes one source's sync checkpoint for a collection, replacing the
/// previous one.
UPSERT_CHECKPOINT = "\
INSERT INTO sources(collection, source, checkpoint) VALUES(:collection, :source, :checkpoint) \
ON CONFLICT(collection, source) DO UPDATE SET checkpoint = excluded.checkpoint";

/// Indexes an object by its content hash at refcount 0; re-storing a known
/// hash only refreshes its size, since the count belongs to
/// `ADJUST_REFCOUNT`.
STORE_OBJECT = "\
INSERT INTO objects(hash, size, refcount) VALUES(:hash, :size, 0) \
ON CONFLICT(hash) DO UPDATE SET size = excluded.size";

/// Resolves the object hash currently bound to each of the given link ids
/// (passed as a JSON array), skipping the ones carrying no body.
///
/// Scoped to one account, the axis a link id is trustworthy on. Across
/// collections it is what this read exists for: one message filed in two
/// mailboxes is one body, downloaded once. Across accounts it is not a
/// fact at all, two unrelated servers being free to mint the same vCard
/// `UID` (spec §9.2), and answering with the other account's body hands
/// one account's content to the other's sync. A single-account store
/// writes no account, so the filter is a no-op and the dedup whole-store.
LOOKUP_OBJECTS = "\
SELECT i.link_id, i.object_hash FROM items i \
JOIN collections c ON c.id = i.collection \
WHERE i.object_hash IS NOT NULL \
  AND i.link_id IN (SELECT value FROM json_each(:links)) \
  AND c.account IS :account";

/// Lists the objects nothing references any more: what the collector
/// takes, and never a write's business, since the batch that attaches a
/// body may not be the one that indexed it (spec §5).
///
/// `<= 0` rather than `= 0`, matching the partial index
/// `objects_garbage` exactly so neither statement scans the table. Under
/// the refcount floor (spec §7) the two select the same rows; the wider
/// one is for a read-only reader, whose store may predate the constraint
/// and still carry a negative count.
LIST_GARBAGE_OBJECTS = "SELECT hash FROM objects WHERE refcount <= 0";

/// Whether the index still holds a body: the collector's question about the one
/// file in front of it, asked on the primary key (spec §5).
OBJECT_EXISTS = "SELECT 1 FROM objects WHERE hash = :hash";

/// Every hash the index holds. For the diagnosis that has to visit every
/// row anyway, never for the collector, which asks about the file in
/// front of it with [`OBJECT_EXISTS`] rather than holding the whole index
/// in memory.
LIST_OBJECT_HASHES = "SELECT hash FROM objects";

/// Drops the unreferenced object rows inside the collector's transaction; their
/// blobs are unlinked after the commit, so a crash leaves at worst an orphan
/// blob.
DELETE_GARBAGE_OBJECTS = "DELETE FROM objects WHERE refcount <= 0";

/// Recomputes every object's refcount from the five columns that pin one (spec
/// §7): an item's body, an item's conflict copy, a source's stored base, a
/// binding's diverging remote body and a pending queue action's body.
///
/// The repair, not the write path: writes maintain the count
/// incrementally with `ADJUST_REFCOUNT`, which is O(changes) where this
/// is O(items+bindings+queue). The pointers are gathered into one stream
/// and counted in a single grouped pass, so the cost is linear in them
/// rather than in their product with the object table. The left join
/// settles an object no pointer names any more, counting zero rather
/// than going unvisited, and a row already holding its true count is
/// left alone.
RECOMPUTE_REFCOUNTS = "\
UPDATE objects SET refcount = counted.n \
FROM ( \
  SELECT o.hash AS hash, count(r.hash) AS n FROM objects o \
  LEFT JOIN ( \
    SELECT object_hash AS hash FROM items WHERE object_hash IS NOT NULL \
    UNION ALL SELECT conflict_object FROM items WHERE conflict_object IS NOT NULL \
    UNION ALL SELECT base_object FROM bindings WHERE base_object IS NOT NULL \
    UNION ALL SELECT conflict_object FROM bindings WHERE conflict_object IS NOT NULL \
    UNION ALL SELECT object_hash FROM queue WHERE object_hash IS NOT NULL \
  ) r ON r.hash = o.hash \
  GROUP BY o.hash \
) AS counted \
WHERE counted.hash = objects.hash AND objects.refcount != counted.n";

/// Deletes the bindings whose item is gone, the one dangling row a repair can
/// clear without guessing: a binding with no item is unreachable, where an item
/// with no object row still holds the item.
DELETE_DANGLING_BINDINGS = "\
DELETE FROM bindings WHERE NOT EXISTS ( \
  SELECT 1 FROM items i \
  WHERE i.collection = bindings.collection AND i.link_id = bindings.link_id)";

// The action queue (spec §15, `queries/queue.sql`): the write door for every
// process that is not the store owner. A producer appends; the owner applies
// pending actions in append order and deletes each in the same transaction as
// its effects.

/// A producer's append. Runs after `ENSURE_COLLECTION`, in one
/// transaction with the `STORE_OBJECT` upsert when the payload references
/// a body (spec §15.1).
ENQUEUE_ACTION = "\
INSERT INTO queue(created_at, producer, collection, action, payload, object_hash) \
VALUES(:created_at, :producer, :collection, :action, :payload, :object_hash)";

/// The collections with pending work, for the owner's drain loop.
LIST_QUEUED_COLLECTIONS =
    "SELECT DISTINCT collection FROM queue WHERE error IS NULL";

/// The owner's drain: a collection's pending (non-parked) actions, in append
/// order. A reader runs the same statement to overlay pending actions on its
/// item projection (read-your-writes, spec §15.4).
LOAD_PENDING_ACTIONS = "\
SELECT id, created_at, producer, action, payload, object_hash, attempts \
FROM queue WHERE collection = :collection AND error IS NULL ORDER BY id";

/// Deletes the row an owner is about to apply, and reports whether it was still
/// there.
///
/// It runs first in the applying transaction, not last: the pending rows
/// are read outside any transaction, so a second owner reading the same
/// list would otherwise apply every action twice, and `add` and `copy`
/// are not idempotent. Claiming the row first makes exactly-once a
/// property of the statement rather than a convention about who drains.
CLAIM_ACTION = "DELETE FROM queue WHERE id = :id RETURNING id";

/// One queue row's spent attempts and pinned body, for a caller acting on a row
/// by id: cancelling it, acknowledging an intent it performed out of band, or
/// recording a failure.
LOAD_ACTION_ROW = "SELECT attempts, object_hash FROM queue WHERE id = :id";

/// One queue row removed by request rather than by application, pending
/// or parked (spec §15.5): a queued item withdrawn, or a performed intent
/// acknowledged by the process that carried it out. The same delete as
/// `DELETE_ACTION`, named apart because the trigger is a request. It
/// releases the row's `object_hash` pin, so it runs in one transaction
/// with the refcount settle.
CANCEL_ACTION = "DELETE FROM queue WHERE id = :id";

/// A permanently failing action: recorded and skipped, visible to
/// operators instead of blocking the collection's queue.
PARK_ACTION =
    "UPDATE queue SET attempts = :attempts, error = :error WHERE id = :id";

/// Records a failed apply attempt without parking: the retry path, the
/// reference `park_action` with a `NULL` error.
BUMP_ATTEMPTS = "UPDATE queue SET attempts = attempts + 1 WHERE id = :id";

/// The parked actions, for status surfaces and operator repair.
LOAD_PARKED_ACTIONS = "\
SELECT id, created_at, producer, collection, action, payload, attempts, error \
FROM queue WHERE error IS NOT NULL ORDER BY id";

/// The owner's handle-space reset marker (spec §12): run in the same
/// transaction as the rebuild it records.
BUMP_GENERATION = "\
UPDATE collections SET generation = generation + 1 WHERE id = :collection \
RETURNING generation";

/// A collection's handle-space epoch, so a reader derives epoch-dependent
/// protocol values (an IMAP UIDVALIDITY) from the store alone.
LOAD_GENERATION = "SELECT generation FROM collections WHERE id = :collection";

// NOTE: diagnostics (spec §7), what a consistency check asks about the
// index rather than through it. Not canonical statements, the spec
// stating the invariants rather than the queries that observe them, but
// inlined so a consumer running its own driver can check what it wrote.

/// How many objects are indexed and what they weigh.
OBJECT_STATS = "SELECT count(*), coalesce(sum(size), 0) FROM objects";

/// The bytes held by objects at least one live item binds. An object a
/// live and a retained item share counts here, since purging the
/// retained one frees nothing.
LIVE_BYTES = "\
SELECT coalesce(sum(size), 0) FROM objects WHERE hash IN \
(SELECT object_hash FROM items WHERE object_hash IS NOT NULL AND retained_at IS NULL)";

/// One object's stored size.
OBJECT_SIZE = "SELECT size FROM objects WHERE hash = :hash";

/// What a purge with this cutoff would retire, and what its bodies weigh:
/// the preview a confirmation prints, `PURGE_RETAINED_BEFORE` being the
/// act.
COUNT_RETAINED_BEFORE = "\
SELECT count(*), coalesce(sum(o.size), 0) FROM items i \
LEFT JOIN objects o ON o.hash = i.object_hash \
WHERE i.retained_at IS NOT NULL AND i.retained_at < :cutoff";

/// The objects whose stored refcount disagrees with the five pointer columns
/// that justify it: the read `RECOMPUTE_REFCOUNTS` settles.
REFCOUNT_DRIFT = "\
WITH refs(hash) AS ( \
  SELECT object_hash FROM items WHERE object_hash IS NOT NULL \
  UNION ALL SELECT conflict_object FROM items WHERE conflict_object IS NOT NULL \
  UNION ALL SELECT base_object FROM bindings WHERE base_object IS NOT NULL \
  UNION ALL SELECT conflict_object FROM bindings WHERE conflict_object IS NOT NULL \
  UNION ALL SELECT object_hash FROM queue WHERE object_hash IS NOT NULL \
), counted(hash, n) AS (SELECT hash, count(*) FROM refs GROUP BY hash) \
SELECT o.hash, o.refcount, coalesce(c.n, 0) FROM objects o \
LEFT JOIN counted c ON c.hash = o.hash \
WHERE o.refcount != coalesce(c.n, 0) ORDER BY o.hash";

/// Every source's binding of one item: the handle it is bound to, the base
/// the last sync agreed on, and the conflict it is stuck on. The read behind
/// `item show`, which names one item and can afford to say everything about it.
ITEM_BINDINGS = "\
SELECT link_id, source, handle, base_flags, base_object, base_revision, base_present, \
conflicted, conflict_revision, conflict_object, shared_object \
FROM bindings WHERE collection = :collection AND link_id = :link_id \
ORDER BY source";

/// How many minted keys (spec §9, `dup:<hint>#<handle>`) each collection
/// holds: the second copy of an identity a source hands over twice,
/// filed as an item of its own.
///
/// Informational, and the only read that looks at the shape of a key at
/// all. It counts them and nothing more: no hint and no handle is read
/// back out of one, since a minted key is opaque and a store that
/// resolved a prefix would make the engine's assignment reversible by
/// accident. `GLOB` rather than `LIKE`, which is case-insensitive over
/// ASCII and would count a hint of its own spelling.
MINTED_KEYS = "\
SELECT collection, count(*) FROM items \
WHERE link_id GLOB 'dup:*' AND deleted = 0 AND retained_at IS NULL \
GROUP BY collection ORDER BY collection";

/// The bindings whose item is gone: the one dangling row a repair can clear,
/// since nothing can read it (`DELETE_DANGLING_BINDINGS`).
DANGLING_BINDINGS = "\
SELECT b.collection, b.link_id, b.source FROM bindings b \
WHERE NOT EXISTS (SELECT 1 FROM items i \
  WHERE i.collection = b.collection AND i.link_id = b.link_id) \
ORDER BY b.collection, b.link_id, b.source";

/// The items whose body is not indexed. Reported, never repaired: the
/// item is still the item.
DANGLING_ITEM_OBJECTS = "\
SELECT collection, link_id, object_hash FROM items \
WHERE object_hash IS NOT NULL AND object_hash NOT IN (SELECT hash FROM objects) \
ORDER BY collection, link_id";

/// The queue rows whose body is not indexed. Reported, never repaired:
/// the row is still an intent somebody expressed.
DANGLING_QUEUE_OBJECTS = "\
SELECT id, collection, object_hash FROM queue \
WHERE object_hash IS NOT NULL AND object_hash NOT IN (SELECT hash FROM objects) \
ORDER BY id";
}

#[cfg(test)]
mod tests {
    use super::ALL;

    #[test]
    fn no_statement_is_empty() {
        for (name, sql) in ALL {
            assert!(!sql.trim().is_empty(), "{name} is empty");
        }
    }
}
