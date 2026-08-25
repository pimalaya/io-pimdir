---
cairn: log
change: store-algorithm-audit
date: 2026-08-25
---

# The cold-eye audit's store findings landed

A first-time reading of the crate against the format (`store-algorithm-audit`) produced a triage list of correctness bugs, shape problems and compaction. The correctness half and the two shape findings that dominate a real store landed; the compaction and the findings that need a format decision did not, and are recorded below.

Every fix went in test-first: the regression test was written against the old code, watched fail, and only then made to pass.

## What landed

- **A write carrying a new sort key silently discarded it** (capability `store`). The row diff compared every column `UPDATE_ITEM` writes except `sort_key`, so a key that changed and nothing else reported the row unchanged and no statement was issued. A key is derived rather than given, and a connector fixing its derivation, a tzdb update moving a zoned start, or the second source of a two-source sync all restate one. The suite covered only the other half of the invariant, that a write carrying no key must leave the stored one alone, which is why it stayed green. The requirement is now stated as a rule about the diff rather than about one column.

- **A descending page hid every item sorting above its first cursor** (capability `store`). "No cursor" was a key no real one was expected to outrank, but a sort key is arbitrary text a writer derives, so no value is reserved: two of the same character outranked the sentinel, and such an item was invisible to every descending page for good while the count still reported it. The statement says `NULL` now, with the same keyset comparison and the same index.

- **Two owners draining one collection applied every action twice** (capability `store`). The pending rows are read outside any transaction and the row was deleted at the end of the applying transaction, so a second owner holding the same list re-applied all of it; `add` and `copy` are not idempotent, and the operator CLI opens a second owner handle routinely. `CLAIM_ACTION` runs first and skips the row when it deletes nothing. The canonical statement moved with it (pimdir `claim_action`).

- **A blob rename was never made durable** (capability `store`). The bytes were synced and the directory entry carrying the name was not, while the database commit is: a power loss could leave a committed row pointing at a body that never arrived.

- **A flag set the store could not decode read as known-empty** (capability `store`), an authoritative "no markers" the merge took as one side's opinion, clearing what the other side reported and persisting the result. It reads as unknown now.

- **`created_at` held epoch milliseconds** where the column is declared RFC 3339, and the empty string when the clock predated the epoch. SQLite writes it now, which also removes the crate's only clock read.

- **A write reads only the rows its batch names** (capability `store`). Folding a batch into a collection loaded, cloned and diffed the whole collection, so one flag on one message cost the size of the mailbox. Measured before and after with a release-mode probe: 3.5 ms / 13 ms / 59 ms at 1k / 4k / 16k items, cleanly linear, against a flat 145 / 161 / 175 µs. §1 of the format promises hundreds of thousands of items and the write path did not meet it. The probe is not kept as a test: a timing assertion would be flaky where the query plan is what actually changed.

- **The residual is keyed rather than listed.** A first sync probes a whole collection before linking any of it, so it grew to the collection size while every insertion, drop and lookup searched it linearly.

- **New indexes**, mirrored into the canonical schema: `objects_garbage` (partial, matching the sweep's new `<= 0` predicate), `items_by_seq_global`, `bindings_by_handle`, `items_by_conflict_object`, `queue_by_object`.

## Not landed, and why

- **`lookup_objects` is not collection-scoped**, so two accounts holding the same vCard `UID` hand each other's bodies across, which §9.2 of the format names as a hazard and does not defend against. The signature is io-replica's, and its yield carries no collection either, so this belongs to a change that moves both.

- **A base of unknown flags, no revision and no object round-trips to no base**, so an agreed placement reads as never-agreed and re-pushes for ever. The hole is the format's (§13 infers presence from three nullable columns) and the fix is a column; whether io-replica currently produces that shape on a linked placement is still open, and decides whether this is live or latent.

- **An object indexed with no referrer in the same batch is swept at the end of it.** The format invites the pattern and permits the sweep, so this is a decision about the format, not a repair here.

- **`refcount` still has no `CHECK (refcount >= 0)`**: adding a constraint to an existing table is a rebuild, which the draft-shape reconciliation does not do.

- **The compaction list is untouched**: the eleven copies of the row-collecting loop, the hand-written statement index and its two `include_str!` guards, the three near-identical row structs, the operator CLI's second read-only connection, the three ways to record a queue failure. Around 460 lines, no behaviour change, and worth doing as its own change against the regression tests this one added rather than mixed into them.

- **The drain still answers two point questions with whole-collection loads** (the `Add` duplicate check and the handle lookup), and `distinct_sources` still scans `bindings`. Both are the same shape as the write finding above and want the same treatment.

## Verification

- 76 tests green (16 lib, 60 integration), `cargo clippy --all-targets --all-features` clean, `cargo fmt`.
- The spec-fidelity suite compares the inlined DDL against `pimdir/migrations/0001_init.sql` through SQLite's own pragmas, and every canonical statement name against the constants, so both the schema and the statement changes are checked against the format on both axes.

Capabilities moved: `store`.
