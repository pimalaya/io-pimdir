---
cairn: log
change: a-rename-follows-a-submits-copy
landed: 2026-11-24
---

# A rename follows a submit's copy

Found by the live night (neverest N4, cairn/changes/the-live-night-findings F2): a send queued with its copy to file in a folder, the folder renamed by an earlier run, was sent, then filing the copy (an `add` into the payload's `copy` replacing the intent, STORAGE §15.5) failed "FOREIGN KEY constraint failed" and the intent stayed unconfirmed. neverest fixed the rename in the same run (8d44f88); one an earlier run made was io-pimdir's: `rename_collection` followed `queue.target_collection` but no payload, as STORAGE §14 said.

The rule moves in pimdir (STORAGE §14, storage-draft-17 edited in place): a `submit`'s `copy`, the one collection a payload names that no column holds, follows the rename in the same transaction, pending or parked, since a send cannot be repeated. A column (`target_collection`) for it was weighed and left: the performers read the payload as it stands (neverest's `SubmitIntent`), and a column only new rows would carry leaves every queued send as it is.

## What landed

- spec/queries/storage/owner/list_queued_submits.sql and set_queued_payload.sql, with their lines in spec/queries/signatures.json.
- src/client.rs: `rename_collection` reads every queued `submit` (`list_queued_submits`), decodes it (`PimdirSubmit::from_payload`, no JSON function in SQL), and writes back the ones whose `copy` names the old id with the new one (`set_queued_payload`), in the transaction of the rename. A payload that does not decode is left for its performer to park.
- src/intent.rs: `PimdirSubmit::to_payload`, which `to_action` now wraps.
- tests/receipts.rs `a_submit_queued_before_a_rename_files_its_copy_in_the_renamed_collection`: failed before at the performer's `replace_action` with `Sql(... "FOREIGN KEY constraint failed")`.

## Spec

[store](../spec/store.md): *A queued action's target is a column a rename follows* adds the submit's `copy` and the scenario *A send pending across a rename an earlier run made*.
