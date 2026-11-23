---
cairn: delta
change: a-delivered-create-waits-for-its-arrival
---

## ADDED Requirements

### Requirement: A delivered create waits for its arrival
Folded into [sync](../../spec/sync.md) under this name.

## MODIFIED Requirements

### Requirement: An add accepted without a handle waits for its page
The create is delivered rather than left pending; folded into [sync](../../spec/sync.md).

### Requirement: The offline mutation vocabulary
A `Remove` or a `Move` of a delivered create tombstones it; folded into [mutate](../../spec/mutate.md).

## REMOVED Requirements
