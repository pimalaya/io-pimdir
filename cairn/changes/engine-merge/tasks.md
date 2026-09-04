---
cairn: tasks
change: engine-merge
---

- [x] Move change, collection, coroutine, hub, load, mutate, object, open, placement, rekey, remote, sync and upgrade into the core, renamed `Replica*` to `Pimdir*`.
- [x] Replace `ReplicaMeta` by `PimdirSummary`, one variant per kind with its addresses, derived by `summary::derive` and checked against vectors/summaries.json.
- [x] Inline the canonical schema and statements verbatim from migrations/storage/ and queries/storage/, plus the batch-scoped loads the diff writer needs.
- [x] Probes as rows, retention in the diff writer, the generation bumped on a `Rekeyed` drop, origins resolved from the bindings.
- [x] Delete the storage trait; `PimdirSourceStore` runs the verbs and exposes `service` for a foreign driver.
- [x] Run vectors/sync/ against the store.
- [x] Port the crate's tests and the engine's tests onto the store; port the CLI.
- [x] Fold the engine's capability specs into cairn/spec/ and rewrite README, CHANGELOG and the lib.rs header.
