---
cairn: log
date: 2026-08-07
change: soft-delete-retention
---

# The store retains instead of deleting, and skips what it cannot apply

Until now `save_hub_diff` hard-deleted an item the moment its last binding
vanished, and the refcount sweep unlinked the body behind it. For a backup, that
made a remote expunge terminal: the store was the only holder of the copy, and
nothing could bring it back. io-replica had already published the contract that
fixes this (`cairn/spec/storage.md`, 2026-07-28): every removal reaches storage
as a `DropPlacement` the storage MAY retain, and hiding retained rows from `load`
is safe because the merge reconciles only what `load` returns. This implements
that contract; io-replica itself is untouched.

`items` gained `retained_at` and `retained_by` plus the partial `items_retained`
index, folded into **version 1** (the spec is a draft, `sql::VERSION` stays `1`),
structurally identical to `pimdir/migrations/0001_init.sql`, with both columns
and the index added to `reconcile_draft_shape` so an earlier-draft store heals on
open. The retiring update sets `deleted = 1` alongside the stamp, which is what
keeps the existing live-only reads (`LIST_ITEMS_PAGE`, `GET_ITEM`, `COUNT_ITEMS`)
hiding retained rows with no change of their own, and matches the format spec's
definition of the retained state (`deleted = 1` **and** no bindings).

The mechanism is three edits and a pin. `LOAD_ITEMS` filters `retained_at IS
NULL`. `save_hub_diff`'s "gone in `new`" branch runs `RETAIN_ITEM` (SQLite stamps
the instant, so the crate stays clock-free and the tests stay deterministic: what
a caller parameterises is the purge *cutoff*) and deletes the item's now
source-less bindings. `insert_item` revives a retained row rather than colliding
on its primary key, keeping its `seq`. And because the hub diff releases an
item's object references as it leaves the hub while the row survives pointing at
them, retiring compensates `+1` on `object_hash` and `conflict_object`: a
retained row pins its bodies exactly as a queue row pins a queued body, and
revive and purge release that pin. Without it the sweep would have hit a foreign
key on a body a retained row still referenced.

On top: `list_retained` / `count_retained` / `retained_bytes` (the trash view),
`purge` and `purge_retained_before` (the only true deletes, both reporting the
bytes actually unlinked, both letting the existing collector do the unlinking).
The paging cursor is the public `seq` rather than the reference statement's
`link_id`, an equivalent substitution under format-spec §8: a caller pages the
trash by the same integer it restores and purges by. Restore needs no new action
kind, as the plan predicted: a queued `Add` over the values the row still holds
lands through the revive branch, and the duplicate-link-id guard exempts retained
rows for free, since it tests the hub and the hub no longer holds them.

The queue half is smaller but changes a contract. An unrecognised kind used to
park, which claims the row is permanently unappliable; it now decodes as
`PimdirAction::Unknown { kind, payload, object_hash }` (payload verbatim, body
still pinned) and the drain **skips** it, leaving it pending and carrying on with
the rest of the collection. That is what lets one queue carry store mutations any
owner applies beside a capability-bound intent (a mail submission) only neverest
can perform. `drop_action` gives that owner the other half: one verb for
cancelling a queued action and for acknowledging an intent performed out of band,
releasing the row's pin in the same transaction. `fail_action` exposes the two
existing failure statements coherently. Malformed payloads still park, so
`PimdirActionError::UnknownKind` is gone (nothing could construct it any more).

Tests: a new `tests/retention.rs` (8), including a quiescence test that drives a
**real** `ReplicaClient` against a fake source and asserts a delta sync and a
full resync both come back empty with no push attempted, mirroring
`io-replica/tests/soft_delete.rs`. Three existing tests asserted the old
hard-delete and were rewritten against the new behaviour (the blob now survives
the drop and goes on the purge). 46 tests green, clippy clean.

Capabilities moved: **store**.

Blocked, and not this change's to fix: `Cargo.toml` still resolves `io-replica =
"0.2"` from crates.io, which predates the binding-conflict fields, so the crate
does not build without patching io-replica to its local path. Everything here was
verified with that patch applied on the command line.
