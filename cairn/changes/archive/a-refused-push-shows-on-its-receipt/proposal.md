---
cairn: change
id: a-refused-push-shows-on-its-receipt
status: landed
created: 2026-11-15
---

# A refused push shows on its receipt

## Why

Follows pimdir's draft of the same id (pimdir cairn/changes/a-refused-push-shows-on-its-receipt), which waits for the owner's answers. Live, 2026-11-15: a push the remote refuses for good (an append into a Graph folder other than Drafts, a floating time Google refuses) leaves the queue row that caused it `applied`; the refusal shows only in that run's `PimdirSyncReport` (`rejected`, `rejected_creates`), unlinked to the row and without a reason. 0.13 cannot park an applied row, and should not: the row's effect is in the store, a retry would apply it again and a cancel would leave the item (pimdir's proposal, "The ask, and why not as asked").

## What the crate would do, once pimdir decides

- `PimdirPushOutcome::Refused { reason }` beside `Rejected` (whose doc already says a stale base); the enum is exhaustive and `Copy` today, so a breaking release, best made `#[non_exhaustive]` with it.
- `PimdirWriteOp::RefusePlacement { collection, handle, key, reason }` written with the outcome, cleared by the next accepted push; the reason through `security::scrub`.
- The sync engine holding a change whose key the binding names refused, and reporting `PimdirSyncEvent::Refused` every run it holds one.
- `record_receipt` binding the item's `seq` for every action on one; `PimdirReader::receipt_refusals(id)` and `refusals(collection)`; the CLI's `queue show` printing the refused state.
- Tests failing first: a connector refusing an add, the row's receipt reading the refusal, the next run not pushing it, an edit pushing it again, a remove withdrawing it (with a-pending-create-is-withdrawn-whatever-the-rights).

Nothing is written yet.

Decided by the owner on 2026-11-15, each recommendation of pimdir's change of the same id taken: D1 `Refused { reason }`, D2 binding columns, D3 held by key, D4 every receipt names its item, D5 no re-parking.
