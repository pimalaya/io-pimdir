---
cairn: change
change: source-discovery
---

# Delta

## ADDED Requirements

### Requirement: A client can discover the store's sources
The store SHALL expose the distinct source names it has synced against (across all
collections) via `distinct_sources`, so a client can attribute its writes to a
source without configuration — a store synced as a single source returns exactly
one. This is a kind-agnostic read; it never mutates.

## MODIFIED Requirements

## REMOVED Requirements
