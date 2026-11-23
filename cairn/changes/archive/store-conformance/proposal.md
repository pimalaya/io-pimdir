---
cairn: change
id: store-conformance
status: landed
created: 2026-09-04
---

# Store conformance with the revised standard

## Why

The standard was revised on 2026-09-04 (pimdir/cairn/log/2026-09-04-*.md) and re-vendored here: a Tombstone placement derives its destination through `destination_for_link`, a Handles load reads its probes through `load_probes_by_handle`, `bindings_by_handle` is unique and a handle names one item per source, a drain parks a store failure and skips a foreign remove, the `hash:` key is pinned by a vector, and the migration runner, the collector and the blob write have normative text the std client did not meet. A review of the client beside it found the FNV prime wrong, the collector unserialised against the process's own writers, a reader failing every overlaid read on one malformed pending row, and an API taking two spellings of a collection id.

## What

Every divergence the review listed, in the std client alone: the load's destination and probes, the handle retirement in the write, the drain's three outcomes, the overlay's tolerance, the blob durability and the collector's lock and walk rule, the migration runner and the open errors, the collection-id type, the busy mapping of single-statement writes, the summary decoders, and the cairn spec cleaned of what no longer exists.
