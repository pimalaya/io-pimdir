---
cairn: change
id: a-move-retains-nothing
status: landed
created: 2026-09-04
---

# A move retains nothing

## Why

Retention is unconditional (STORAGE §11), so a confirmed move left the source item in the source collection's trash: the inbox trash filled with every message archived, and a restore would have re-added a message the archive already held.

## What

When the drop of an item's last binding leaves the identity held live by another collection of the same account, the row is retired then purged in the same transaction. Nothing is lost: the holder pins the body, and the purge counts for the change feed. A delete with no holder elsewhere retains as before.
