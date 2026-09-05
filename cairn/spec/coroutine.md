---
cairn: spec
capability: coroutine
status: current
---

# Coroutine

Every verb in the crate is a state machine implementing one contract: a driver resumes it, it yields what it wants (`PimdirYield`), the driver services that and resumes it with the matching reply (`PimdirArg`), until it completes. The engine performs no I/O of its own, which is what the contract is for: storage and remote are `Wants` variants rather than traits injected into the engine.

The rules here are about the contract itself, not about what any one verb decides. What the verbs decide lives under [sync](sync.md), [upgrade](upgrade.md), [rekey](rekey.md), [mutate](mutate.md), [hub](hub.md) and [storage](storage.md).

The five verbs are `PimdirOpen`, `PimdirUpgrade`, `PimdirMutate`, `PimdirSync` and `PimdirRekey`. `PimdirOpen` is the one with no capability file of its own, because it decides nothing: it is the offline read, a `WantsLoad` of a whole collection answered straight back to the caller, and it exists so a consumer projecting a replica needs no second code path for "read it without touching the network". Its rules are the contract's own.

### Requirement: An offline read is a verb
Reading a collection without a remote SHALL go through the same coroutine contract as everything else (`PimdirOpen`), rather than through a direct storage call. A consumer holding the engine holds one way to reach a replica, so a projection built offline and one built during a sync cannot drift apart, and a storage implements one seam rather than a seam plus a back door.

`PimdirOpen` SHALL scope its load to the whole collection: it is one of the two verbs that reason about what is *missing* from a replica rather than about named rows, its answer being the projection itself.

### Requirement: One error for a broken coroutine contract
A driver that resumes a coroutine with an arg not matching the pending yield, or without the arg the yield required, SHALL be told so through `PimdirArgError`. It is one type for every verb because it is one bug, in the driver rather than in the coroutine, and the caller knows which verb it resumed.

`PimdirOpen`, `PimdirUpgrade`, `PimdirRekey` and `PimdirSync` SHALL return it directly: they read local state, ask for remote state and stage writes, and none of that can fail inside the engine. A verb with failures of its own (`PimdirMutate`) SHALL compose it beside them rather than restate its variants.

### Requirement: A completed coroutine does not resume
Every coroutine SHALL hold a terminal state and answer any resume after completion, `None` included, with `PimdirArgError::UnexpectedArg`. Handing back a default output instead is worse than useless: an empty report or an `Ok(())` is exactly what a run that genuinely did nothing returns, so a driver with a loop bug is told it succeeded. `MissingArg` is for a yield still pending, not for a run that is over.

#### Scenario: A driver resumes a finished run
- GIVEN a coroutine that completed
- WHEN the driver resumes it again, with an arg or without
- THEN it answers `UnexpectedArg` rather than an empty success

### Requirement: A state is named for what the coroutine is doing
Every coroutine's `State` SHALL name its variants in the present tense for what the coroutine is doing while it waits for the caller (`Loading`, `Enumerating`, `Fetching`, `Pushing`, `Writing`, `CheckingLinks`), never `Pending*` or `Await*`: a state is the coroutine's own activity, not the caller's obligation.
