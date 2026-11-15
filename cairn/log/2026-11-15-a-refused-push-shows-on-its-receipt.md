---
cairn: log
change: a-refused-push-shows-on-its-receipt
landed: 2026-11-15
---

# A refused push shows on its receipt

Follows pimdir's change of the same id, decided by the owner on 2026-11-15 (D1 `Refused { reason }`, D2 binding columns, D3 held by key, D4 every receipt names its item, D5 no re-parking), in the working tree of both repositories, uncommitted. spec/migrations, spec/queries, spec/vectors and spec/payloads are re-vendored whole from pimdir's working tree; spec/REVISION still names e4a05b1, so tests/spec_drift.rs `the_vendored_copy_is_the_spec_byte_for_byte` fails until the owner commits pimdir and the revision moves.

## What landed

- `PimdirPushOutcome::Refused { reason }`, the enum `#[non_exhaustive]` and no longer `Copy`; `Rejected` documented as a stale base, or what a connector that cannot tell answers.
- `PimdirWriteOp::RefusePlacement { collection, handle, key, reason }` and `PimdirWriteOp::ClearRefusal { collection, handle }`: the client runs `refuse_binding`, the reason through `security::scrub`, and `clear_refusal`, after the batch's placement writes and stamps; the hub ignores both.
- The engine: `PimdirLoaded::refused` (`list_refusals`, the source's rows, on every selection) keyed by handle; a change derived under a refused key is held, its stash dropped, counted in `PimdirSyncReport::refused` and reported `PimdirSyncEvent::Refused { handle, reason }` with the next write; a `Refused` outcome writes `RefusePlacement` with the pushed change's key and reports the same; an `Accepted` outcome on a refused handle writes `ClearRefusal` before the settle.
- Receipts: the drain binds the `seq` of the item every action acted on (`set-flags`, `update`, `remove` when the item is live before the apply, `move` and `copy` their target's create); `PimdirActionStatus::Applied::refusals` (`PimdirReceiptRefusal`), `PimdirReader::receipt_refusals(id)` and `PimdirReader::refusals(collection)` (`PimdirRefusalRow`); `prune_receipts` keeps a refused item's receipt (the vendored statement); `pimdir queue status` prints a `Refused by …` line per refusal and `refusals` in its JSON.
- tests/vectors_sync.rs reads format 7: seeds a binding's refusal through `refuse_binding`, compares `refusal` and `refused_key`, feeds `Refused` outcomes and reads `Refused` events.

## Checks

- tests/vectors_sync.rs over the vendored cases 162 to 167, run with the new runner on the old engine (`Refused` fed as `Rejected`, the one outcome it had): 162 failed ("events differ", no `Refused`), 163 ("pushes differ", the held `Add` pushed again), 165 ("bindings rows differ", the refusal still standing after the accepted `Update`), 166 ("pushes differ", the held `Remove` pushed again); 164 and 167 passed. All 167 cases pass now, tests/pending_vectors.txt unchanged.
- tests/receipts.rs `every_receipt_names_the_item_its_row_acted_on` (failed on the old drain's receipt logic with "a set-flags names the item it changed", left `None`, right `Some(1)`) and `a_refusal_shows_on_the_receipt_and_keeps_it_until_cleared` (a refusal read from the receipt, scrubbed, the receipt kept past a week until cleared); `a_parked_row_reads_with_its_error_and_a_cancelled_one_nowhere` still holds that a remove of an absent item names none. tests/cli_json.rs `an_absent_value_prints_null` expects the empty `refusals`.
- Capabilities moved: seam ("A connector tells a refusal from a stale base", "A refusal is the binding's", added), sync ("Push-outcome discipline" and "The report says what a run left pending", modified; "A refused change is held by its key", added), store ("An applied row leaves a receipt a producer follows", modified), cli (`queue status`).

## Left open

- A create the store offers a source (propagated from another) has no binding: its refusal is reported and recorded nowhere, and pushed again every run (pimdir's log, "Left open").
- neverest and the Android app answer `Refused` from their connectors next.
