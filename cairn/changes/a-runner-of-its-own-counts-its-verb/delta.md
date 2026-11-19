---
cairn: delta
change: a-runner-of-its-own-counts-its-verb
---

## ADDED Requirements

### Requirement: A store never collects itself (the public guard)
`PimdirSourceStore::running` SHALL raise the running count the collector, `evict_before` and `release_before` read, until the `PimdirRunning` it answers drops; a runner of its own SHALL hold one from its first resume to its last.

## MODIFIED Requirements

### Requirement: A store never collects itself
The count is raised by `PimdirSourceStore::running` (`run` calling it), no longer named as `PimdirLock::running`.

## REMOVED Requirements
