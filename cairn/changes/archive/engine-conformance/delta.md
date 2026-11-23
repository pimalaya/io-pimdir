---
cairn: delta
change: engine-conformance
---

## ADDED Requirements

### Requirement: The key is derived as SYNC §4 states (sync)
A `PimdirChangeKey` SHALL be the FNV-1a 64 digest SYNC §4 fixes, over fields each followed by one `0x00`, a flag set folded as `known`, its count in decimal ASCII and each flag in code point order, so two engines over one store key one change alike and every vector's key reproduces.

### Requirement: A pending create is landed by its arrival (upgrade)
A fetch resolving a probe to a hint a pending create of the same source holds in the collection SHALL land that create rather than mint: a `Superseded` drop of the provisional handle, then the create upserted under the fetched handle with a base of the probe's flags, the fetched revision and the fetched body at `Full` or else the create's own, its staged flags, body, summary and sort key kept, its status `Clean` when they equal the base and `Dirty` otherwise (SYNC §6). Only a hint a based binding holds is minted.

### Requirement: A destination is what the store derives (sync)
A `Tombstone` placement's origin is its destination, the collection where the same source holds a pending create of the identity, derived by the store from its bindings (SYNC §3) and never stored; the engine SHALL derive `Remove { to }` from it and SHALL rely on no column of its own.

### Requirement: An accepted content push rebases the placement the flag merge wrote (sync)
The placement stashed for a content push SHALL be the one the flag axis last wrote for that handle, so an accepted `Update` rebases the merged flags rather than the row read before the merge (SYNC §5).

### Requirement: A rekey never writes a base it never reconciled (rekey)
A mutable member whose fetched revision differs from its old base's SHALL be carried as a pull, body dropped, level `Probed`, base object `None` at the fetched revision, or as a `Conflict` at the fetched revision with the base untouched when it also holds a local edit (SYNC §8). The `Meta` fetch a rekey issues SHALL go in chunks of `PimdirRekey::FETCH_CHUNK`.

### Requirement: The delete policy defaults to `Auto` (sync)
`PimdirDeletePolicy::Auto` SHALL be the default; the engine reads it as `Revert`, and a consumer that knows the binding count resolves it to `Keep` for a source bound beside others (SYNC §5).

### Requirement: A re-listed probe is not a pull (sync)
A probe an enumeration lists again with the flags the store holds for it SHALL derive no write, no event and no count; a probe a complete enumeration no longer lists, or a delta reports vanished, SHALL be dropped `Deleted` like any member.

### Requirement: A pull lowers the shared level (hub)
An upsert carrying no body while the item holds one SHALL lower the shared level to the placement's with the body it drops, rather than merge it as a maximum, so the item reads `Probed` for the upgrade to refetch.

### Requirement: A rejection counts for a pushed handle only (sync)
`PimdirSyncReport::rejected` SHALL count a `Rejected` outcome once per handle the chunk pushed, on the terms `pushed` counts an accepted one.

### Requirement: A tombstone's flags ride along (hub)
`absorb` SHALL adopt a `Tombstone` upsert's known flag set into the shared item, adopting no content (SYNC §9).

### Requirement: A body-less item is no divergence (hub)
`project` SHALL read a bound source whose base holds a body while the item holds none as agreeing on the content axis: the source owes nothing until a hydration gives the item a body again.

### Requirement: An edit on a create clears its origin (mutate)
An `Edit` on a `Created` placement SHALL drop its origin, so the create uploads the edited body rather than server-copying the one the origin holds.

### Requirement: A completed coroutine refuses every resume (coroutine)
A coroutine that completed SHALL answer `PimdirArgError::UnexpectedArg` to any resume, `None` included, and its states SHALL be named in the present tense for what the coroutine is doing while it waits.

## MODIFIED Requirements

### Requirement: A move delivers exactly one copy (sync)
Rewritten to SYNC §3 to §6: the create delivers by copy from its origin or by upload when the store holds the body; the remove relocates into the destination the store derives, which a connector that cannot relocate rejects; a relocated member arrives under a new handle and the fetch naming it lands the create; a create holding neither origin nor body stays visibly pending until restaged.

### Requirement: An enumeration is ordered by handle (sync)
The claim that protocols hand the listing over sorted goes: `PimdirHandle` orders as bytes, so an IMAP SEARCH's ascending UIDs are not sorted under it, and the engine sorts every snapshot.

### Requirement: Naming a probe gives it a base (upgrade)
Except when the hint is a pending create's, which the fetch lands instead.

## REMOVED Requirements

### Requirement: A never-fetched item stages the source half alone (sync)
A mutation of a probe is refused (mutate.md), so every tombstone with a destination carries a link id.
