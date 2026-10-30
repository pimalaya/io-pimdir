---
cairn: tasks
change: errors-carry-a-stable-code
---

- [x] src/client.rs: `PimdirError::code()`; the codes documented in its rustdoc and asserted where the failure is reproduced (tests/queue.rs, tests/stale_store.rs, tests/open_current.rs, tests/staging.rs) rather than by a variant-by-variant table test
- [x] Display: `Stale`, `NotCurrent`, `Version` worded as above; src/cli/mod.rs `report` the same
- [x] `#[non_exhaustive]` on `PimdirError`, `PimdirSyncEvent`, `PimdirEnumerated`, `PimdirQueryErrorCode` (`PimdirArgError` left exhaustive)
- [x] Opaque SQL and JSON error types in the public variants (`PimdirSqlError`, `PimdirJsonError`)
- [x] Spec: store.md (cli.md states no message of its own); fold, log, CHANGELOG (breaking)
- [x] Tell the apps (Phase A: JSON codes from `code()`): migration notes in cairn/log/2026-10-30-errors-carry-a-stable-code.md
