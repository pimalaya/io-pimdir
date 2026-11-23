---
cairn: delta
change: a-purge-of-chosen-items
---

## ADDED Requirements

### Requirement: A purge of chosen items
`purge_items(items)` SHALL purge the retained rows a list of collection and public id names in one transaction, each as `purge` does, skipping a live or missing one, reporting how many went, and giving pages back as the other purges do.

## MODIFIED Requirements

## REMOVED Requirements
