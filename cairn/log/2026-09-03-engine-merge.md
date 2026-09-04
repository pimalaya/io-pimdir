---
cairn: log
change: engine-merge
date: 2026-09-03
---

# io-replica folded in, the store on the 2026-09-03 spec

The sync engine is now this crate's core: the five verbs, the hub and the seam vocabulary moved from io-replica under the `Pimdir` prefix, their unit tests with them, and the storage trait went, the store servicing its own yields. The store caught up with the spec of the same day: typed summaries and addresses under Annex A, probes as rows, the change feed and its triggers, the refcount floor, and the canonical statements verbatim, generated from the spec's own files.

Four places where the engine and the spec disagreed settled on the spec's side, each in the capability file that owns it: an item no source holds stays in the hub for the store to retain (hub), a pulled member is a probe with no base and a named probe takes its base from what the source reported (sync, upgrade), a KeepBoth fork is minted `dup:<hint>#<provisional handle>` (sync), and a rebuild drops with `Rekeyed`, which is what bumps the generation (rekey, seam). The spec's sync vectors, run against the real store, decided the event rule the spec had left ambiguous: an accepted push reports nothing.

The engine's own suites, ported onto the store the same day, found what a memory hub had hidden: a batch superseding a provisional handle was refused as a rebind, a bound placement with no base and no body projected `Dirty` and pushed a bogus flag set, a copy of a locally edited item would have server-copied the unedited body, and a mutation on a probe was silently lost. The first three settled on the spec's side (hub, seam), the last is a refusal (mutate), and SYNC §3's origin rule now names the body.

Verified by the vectors: ten sync cases and twenty-two summary cases reproduce, and the engine's 245 unit tests pass in the core. The crate's own tests and the engine's integration tests were ported onto the store, and the CLI onto the typed summaries. No migration is offered: a store from an earlier draft is refused as stale.
