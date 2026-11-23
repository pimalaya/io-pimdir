---
cairn: tasks
change: an-ambiguous-intent-awaits-confirmation
---

- [x] pimdir: STORAGE §15.2 (a drain skips an unconfirmed row), §15.4, §15.5 (the state, confirming, retrying, withdrawing), Annex B.2 (which intents can end so), the statements, a scenario in checks/invariants.sh (pimdir draft-10 and draft-11, `unconfirm_action` and its scenarios)
- [x] src/client: mark a row unconfirmed, confirm it (acknowledge with its receipt), retry it, read the state (`unconfirm_action`, `acknowledge_action`, `retry_action`, `PimdirActionStatus::Unconfirmed`)
- [x] src/intent.rs: the state in the intent vocabulary (read as `PimdirActionStatus::Unconfirmed` in src/client/producer.rs, the state being the row's, not the intent's)
- [x] CLI: the queue verbs show and resolve an unconfirmed row (`queue status`, `queue list`, `queue retry`; tests/cli_copies.rs)
- [x] Test: a submit ending unconfirmed is not drained again until confirmed or retried (pimdir checks/invariants.sh `unconfirmed:` scenarios; tests/queue_text.rs, tests/cli_copies.rs)
- [x] Spec: store.md; fold, log, CHANGELOG (cairn/log/2026-11-13-the-crate-follows-draft-10-text-and-gate.md, cairn/log/2026-11-19-an-ambiguous-intent-awaits-confirmation.md)
- [x] neverest: report a submission lost after the data as unconfirmed rather than failed (neverest src/sync/submit.rs)
