---
cairn: change
id: client-read-api
status: landed
created: 2026-08-01
---

# Client read API (indexed, paginated reads)

## Why

Today every statement in the store is **engine-oriented**: `load_hub` reads a
whole collection (`LOAD_ITEMS` = load-all), `write` deletes-all-then-reinserts.
That services io-replica's sync seam, but a **client** (Himalaya projecting an
envelope list, a desktop/mobile UI) cannot use the store as a local backend: it
has no way to page a collection, fetch one item, or count without loading every
item into memory and rebuilding the hub.

This is the missing half of "pimdir as the generic local backend"
(LOCAL_STORE_PLAN §4, action plan M1): reads are direct, indexed getters over the
same store the sync engine writes — no second copy, no format bridge. It is the
prerequisite for a Himalaya `pimdir` backend (action plan M4).

The rule (LOCAL_STORE_PLAN §4.2) holds: **reads are direct getters here; writes
stay disciplined through io-replica's mutate seam** (unchanged by this change).

## What

A read-only query surface on `PimdirStore`, kind-agnostic (raw `meta`, string
flags, opaque object hash), keyset-paginated, tombstone-excluding, and
**availability-aware** (each item carries its `level` so a caller knows a body is
absent without probing the blob):

- `list_collections()` → `[PimdirCollection { id, kind, name, parent, color,
  description, sort_order }]`.
- `list_items(collection, after, limit)` → a page of `PimdirItem { link_id,
  flags, meta, object, level }`, keyset on `link_id` (`link_id > after`), live
  items only (`deleted = 0`), ordered by `link_id`.
- `get_item(collection, link_id)` → `Option<PimdirItem>` (live only).
- `count_items(collection)` → live item count.

Blob reads already exist (`PimdirBlobs::get`/`reader`); this change adds only the
index queries.

## Scope / non-goals

- **No schema change.** Keyset pages ride the existing `items` PK
  `(collection, link_id)`; no new index needed.
- **No write path change.** Writes remain io-replica `ReplicaWriteOp`s through
  `write`; this change adds no setter.
- **No meta interpretation.** `meta` is returned as the raw stored string; the
  per-domain schema (action plan M3) and the Himalaya projection (M4) are
  separate changes.
- **No search/filter beyond keyset pagination.** `LIKE`/FTS is later (M4).
