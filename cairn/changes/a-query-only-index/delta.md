---
cairn: delta
change: a-query-only-index
---

## ADDED Requirements

### Requirement: A query client opens the index read-only and builds nothing
`PimdirIndex::open_query`: read-only, no lock, never creates, rebuilds nor refreshes; `index-missing`, `index-outdated`, `index-query-only`.

### Requirement: A collection term names an id
`collection:` and `folder:` match collection ids alone.

### Requirement: A value of digits names a seq and a link id
`id:` and `linked:` digits name the `seq` and every item of that link id; `linked:` reads a role only when it is one.

### Requirement: A rename past a cut pass is reconciled
The next completed pass reconciles a rename a cut pass folded past.

## MODIFIED Requirements

### Requirement: The index is derived and rebuilt, never migrated
Build 2 (`index_build.reconciled`); an index another implementation built is rebuilt, so stored chunk offsets are this extraction's.

### Requirement: A refresh folds the feed in bounded, resumable batches
The reconciliation compares collection stamps with `index_build.reconciled`, which no batch advances; a query handle refuses to refresh.

### Requirement: One-shot answers refresh first and say what is behind
Every answer states `coverage.local.behind`.
