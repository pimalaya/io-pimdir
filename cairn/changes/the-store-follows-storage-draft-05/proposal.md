---
cairn: change
id: the-store-follows-storage-draft-05
status: landed
created: 2026-10-30
---

# The store follows storage-draft-05

## Why

pimdir landed storage-draft-05 and search fixes on 2026-10-30 (its cairn/log/2026-10-30-*.md). Beside the version contract (the-store-version-contract), they change what a write, a read and a drain do: the agreed body is pinned (a-merge-keeps-every-body), a collection holding queued actions is not deleted, a derived-key prefix in a hint is escaped, an unknown payload `v` is skipped, an `update` may name the body it replaced (should-fix-before-the-freeze), a body is checked against its name when read (a-read-body-is-verified), the role vocabulary is open (every-vocabulary-is-open-or-closed), and `has:attachment` reads a calendar item's `attachment` field (SEARCH §6, §8). Two apps also hit an orphan blob: a create refused by the gate or the duplicate check left the body it had staged unpinned. The release audit's S1 (a blob temporary shared between writers) is store-side too.

## What

The store side of each rule, each with a test failing first; the engine side (the merge reading the pinned body, `SetFlags` add and remove sets, completeness, the mass-absence guard, the `prefer-*` loser, a corrupt body never pushed) stays with the next wave and its changes (the-agreed-body-stays-pinned, a-queued-edit-names-what-it-changes, sync-audit-2026-10-30). Capabilities: store and search.
