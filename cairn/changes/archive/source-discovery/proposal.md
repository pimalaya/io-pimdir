---
cairn: change
id: source-discovery
status: landed
created: 2026-08-01
---

# Source discovery read

## Why

A client using the store as a local cache must attribute its writes to a replica
source (so the sync pushes them). In the local-sync case the store is synced as a
single source, so the client can discover it rather than being configured — but
there was no read for "which sources has this store synced".

## What

`PimdirStore::distinct_sources()` returns the distinct source names across the
store's bindings (SQL `LIST_SOURCES`), kind-agnostic. A store synced as one source
returns exactly one, letting a client (a Himalaya pimdir backend) auto-select it.

## Scope / non-goals

- Read-only; no write or schema change.
