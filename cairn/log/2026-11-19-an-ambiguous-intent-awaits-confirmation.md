---
cairn: log
change: an-ambiguous-intent-awaits-confirmation
landed: 2026-11-19
---

# An ambiguous intent awaits confirmation

Closed as landed in substance, on the owner's ruling of 2026-11-19 (pimdir the-lines-before-v1), nothing written today. The state arrived through pimdir draft-10 and draft-11 (pimdir audit-2026-11-12 O7 and F16) rather than through this change's own shape:

- An unconfirmed intent is a parked row whose `queue.error` holds the reserved prefix `unconfirmed: ` (STORAGE §13, §15.2, §15.5), the open question of a column or a reserved `error` form answered by the second: an older owner reads it as parked, never as pending, so it is never drained again blindly.
- `PimdirStore::unconfirm_action(id, detail)` writes it through the canonical statement (src/client.rs), `PimdirActionStatus::Unconfirmed` reads it (src/client/producer.rs), and it leaves the state by `acknowledge_action` (confirmed), `retry_action` (pending again, src/client.rs, tests/queue_text.rs) or `cancel_action` (withdrawn).
- The CLI shows and resolves it: `queue status`, `queue list` and `queue retry` (src/cli/queue.rs, tests/cli_copies.rs `a_parked_row_is_retried_and_an_unconfirmed_one_shown`); pimdir checks/invariants.sh holds its scenarios.
- neverest reports a submission lost after the data as unconfirmed (src/sync/submit.rs).

Recorded where it landed: cairn/log/2026-11-13-the-crate-follows-draft-10-text-and-gate.md and cairn/log/2026-11-13-the-crate-follows-draft-11.md. Capabilities moved: none today.
