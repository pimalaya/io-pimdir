---
cairn: log
change: source-discovery
landed: 2026-08-01
---

# Source discovery read

Added `PimdirStore::distinct_sources()` (SQL `LIST_SOURCES` =
`SELECT DISTINCT source FROM bindings`), so a client can discover which replica
source(s) the store was synced as and attribute its writes accordingly. A store
synced as a single source (the local-sync case) returns exactly one, letting a
Himalaya pimdir backend auto-select it without configuration. Kind-agnostic,
read-only. Suite green, fmt clean.

Spec updated: `store` (ADDED: a client can discover the store's sources).
