---
cairn: delta
change: revised-standard-alignment
---

## ADDED Requirements

### Requirement: A vanished local edit is re-staged as a create (sync)
A member the enumeration no longer lists whose placement holds a body its base does not SHALL be re-staged: a `Superseded` drop of its handle and a `Created` upsert under the link id's provisional handle, reported `Vanished`, the next run adding it.

### Requirement: A create waits for the probes (sync)
A `Created` placement SHALL derive no `Add` while the collection holds a probe or a listed member the replica lacks.

### Requirement: An item-level conflict is held, a binding conflict is reconciled (sync)
The conflict branch of the content axis SHALL run only for a placement carrying a `conflict_revision`; a `Conflict` carrying none derives nothing until an `Edit` or a `Remove` settles the item.

### Requirement: A rebuild's drops keep the base the absorb compares against (hub)
`absorb` SHALL keep the binding a `Superseded` or `Rekeyed` drop removes aside for the upsert of the same batch that rebinds the source.

### Requirement: A provisional handle is the link id under U+0001 (mutate)
Every staged create SHALL sit under `PimdirLinkId::provisional`; the caller supplies no placeholder.

### Requirement: A fetch moves the base (upgrade)
A `Full` fetch over no staged edit SHALL set the base to the fetched body and revision; over a staged edit the remote moved past it SHALL mark a `Conflict` with the fetched body as `conflict_object`.

### Requirement: A mutable member restating its hint is a new identity (upgrade)
A fetched hint differing from the link id held, mutable kinds only and never a `dup:` key, SHALL key the handle afresh.

### Requirement: A collection can be deleted (store)
`delete_collection` SHALL cascade and recompute the refcounts in one transaction.

### Requirement: A refused delete is decided beside the collection's sources (store)
`sync` SHALL hand the engine `COLLECTION_SOURCES` less this handle's own source.

### Requirement: Conflicted items are listable (store)
`list_item_conflicts` SHALL run `LIST_CONFLICTED_ITEMS`.

### Requirement: A lookup answers hash and size (store)
`lookup_objects` SHALL answer `PimdirObject`s.

### Requirement: The schema check covers the triggers (store)
`check` SHALL verify every canonical table and trigger, `Stale { missing }` naming the first absent.

## MODIFIED Requirements

### Requirement: Headless conflict resolution (sync)
Three policies: `KeepBoth` is gone.

### Requirement: A read-only source reverts a local delete (sync)
A refused delete like any other, reverted for a source alone.

### Requirement: A refused delete is decided from the other sources (sync)
Was "follows one policy": `PimdirDeletePolicy` is gone, `PimdirSync::beside_other_sources` decides, held beside another source and reverted alone.

### Requirement: A write batch is bounded and cut between candidates (sync)
The pair a cut must not split is the vanished edit's drop and create.

### Requirement: Events report what the remote changed (sync)
Only a sync reports events.

### Requirement: A pulled member is a probe (sync)
A tombstone holding a staged edit over a remote edit follows the conflict policy.

### Requirement: The hub propagates a delete across sources (hub)
A tombstone upsert clears the item's cross-source conflict.

### Requirement: The hub resolves cross-source content conflicts by policy (hub)
The three facts read before the upsert against the base held; mutability judged across every binding; an immutable kind's base adopts the shared body; every binding of a conflicted item projects `Conflict` with no revision.

### Requirement: A created placement carries the origin the store knows (hub)
The offered copy sits under the link id's provisional handle.

### Requirement: The offline mutation vocabulary (mutate)
`Copy` and `Move` take no placeholder; a `Remove` settles a conflict against the recorded revision.

### Requirement: Add stages a locally-authored create (mutate)
`Add` carries no handle.

### Requirement: A staged create never takes a key its target holds (mutate)
A copy or move of a placement with neither a body nor a based binding is `Undeliverable`.

### Requirement: A mutable body is fetched, never linked (upgrade)
`LookupObject` answers `PimdirObject`s and a link needs the summary's size to agree.

### Requirement: An upgrade supplies a conflict's diverging body (upgrade)
A diverging body equal to the placement's own clears the conflict.

### Requirement: A rebuild carries state over by link id (rekey)
Every `Rekeyed` drop precedes every upsert.

### Requirement: A rekey never writes a base it never reconciled (rekey)
A `Conflict` is carried as it is; an item-level one gains no revision unless the remote moved.

### Requirement: A rebuild keys two copies of one hint apart (rekey)
Pairing by base revision, then body, then handle order.

### Requirement: A drop says whether the item is gone (seam)
`Superseded` covers the handle of a vanished edit re-staged.

### Requirement: A write batch is applied in order (seam)
A rebuild relies on the order outright, drops first.

### Requirement: A collection can be renamed without losing its contents (store)
The rename retargets the queued moves and copies.

### Requirement: Producers append, only the owner pops (store)
`PIN_OBJECT` before the row; the drain store-wide, bounded retries that never stop the pass, each action run as a source binding its item or syncing its collection, a move into an undeclared kind parks.

### Requirement: Pending actions are readable (store)
Store-wide too.

### Requirement: An item is retained, never deleted, when its last binding goes (store)
`HELD_ELSEWHERE` bound to the body; the trash lists every deleted row, `at` `None` for a held tombstone.

### Requirement: A reappearing link id revives its retained row (store)
A revive keeps a retained body the incoming placement carries none of.

### Requirement: A store never collects itself (store)
The collector refuses in flight and recomputes first.

### Requirement: The change feed is the triggers' (store)
`STAMP_ITEM` requests a stamp; the cursor is the last stamp drawn.

### Requirement: Item mutations enqueue, then drain when the owner role is free (cli)
The drain is store-wide.

### Requirement: A statement of the crate's own serves no profile (spec-fidelity)
`LIST_OBJECT_HASHES` is a diagnostic; the two owner statements written tonight went upstream.

## REMOVED Requirements

### Requirement: A keep-both duplicate is a new item (sync)
No keep-both.

### Requirement: The delete policy `Auto` reads the binding count (store)
Replaced by the source count handed to the engine.
