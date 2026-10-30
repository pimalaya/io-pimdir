---
cairn: spec
capability: sync
status: current
---

# Sync

`sync` reconciles one collection's local replica against one remote through a three-way merge of Local, Base and Remote per placement, keyed on the handle. Flags merge element-wise and never conflict; only divergent mutable content is kept as a conflict. It is tuned by `PimdirSyncOptions`.

It reconciles what the replica holds and never fetches a payload: every member a listing carries arrives named by its meta, raising a placement to its body is [upgrade](upgrade.md)'s, and rebuilding a collection onto a new handle space is [rekey](rekey.md)'s. A listing is a delta from the checkpoint or a round over a scope, answered page by page, each page landing in one write (pimdir SYNC §4, §5).

### Requirement: Three-way reconcile
The engine SHALL merge each candidate placement over `(local, base, remote)`, pushing local-won changes and pulling remote-won changes, comparing per-placement identities (the flag set and, for mutable-content backends, a content revision) rather than raw bytes.

### Requirement: Push-outcome discipline
The engine SHALL confirm a push with the remote before rewriting local state: a flag, content, delete or create push is stashed and applied only on an `Accepted` outcome; a `Rejected` (or unreported) outcome leaves the placement dirty, tombstoned or provisional so the next sync retries it.

### Requirement: Push direction
`PimdirSyncOptions.push` SHALL be the master push switch. When false the source is treated read-only: local flag and content changes are kept dirty and never pushed, remote-won changes are still pulled, and a local delete is applied to the replica only. When true, the `PimdirPushRights` refinement SHALL gate each push kind independently.

#### Scenario: Read-only source keeps local edits
- GIVEN a placement with a locally-changed flag set and `push = false`
- WHEN the collection is synced
- THEN the engine emits no push and the placement stays dirty

### Requirement: Granular push rights
`PimdirSyncOptions` SHALL carry a `PimdirPushRights` refinement with an independent boolean for each push kind (`flags`, `content`, `add`, `remove`), defaulting to all-permitted. When `push` is true, the engine SHALL derive a push of a given kind only when the matching right is permitted; a forbidden push kind is treated like the read-only path for that kind alone (the local change is kept pending and never pushed, and a forbidden delete is not applied to the replica), while other kinds still propagate.

#### Scenario: Flags allowed, deletes forbidden
- GIVEN a source with `push = true`, `rights.flags = true`, `rights.remove = false`
- AND a placement with a locally-changed flag set and another locally tombstoned
- WHEN the collection is synced
- THEN the flag change is pushed
- AND no remove is pushed, and the tombstone is retained (not dropped) for a later sync

### Requirement: Per-item delta events
The sync SHALL emit a `PimdirSyncEvent` for each per-item outcome it produces (a member added, its flags changed, its content changed, it vanished, it conflicted, a create the remote accepted, or a body a policy did not adopt) in order, and carry them on `PimdirSyncReport.events`, each once the write recording it committed (see "An event follows its commit"). Events are spine-level data (a handle, no body), so emitting them enters no I/O. Hooks and richer reporting ride the events; the report's counters summarise them.

#### Scenario: A remote add emits Added
- GIVEN a remote that lists a member absent locally
- WHEN the collection is synced
- THEN the report carries a single `Added` event for that handle

#### Scenario: An accepted create is reported under its assigned handle
- GIVEN a locally-created member the remote accepts and assigns a handle
- WHEN the create is confirmed
- THEN the report carries a `Created` event for the server-assigned handle

### Requirement: Headless conflict resolution
`PimdirSyncOptions` SHALL carry a `PimdirConflictPolicy` (`Manual`, `PreferLocal`, `PreferRemote`; default `Manual`) applied when content diverges on both sides of a based placement. `Manual` marks the placement conflicted and waits for the consumer's edit, recording the observed remote revision as `conflict_revision` and the diverging remote body as `conflict_object` beside it. `PreferRemote` pulls the remote, the local edit it drops kept by the write as a retained item and reported `Overruled` (pimdir SYNC §5, §9, vectors/sync/14); a pull the write cannot let replace another source's body is recorded as the item's conflict instead and reported `Conflicted` in place of `ContentChanged` (vectors/sync/83). `PreferLocal` pushes the local body as an `Update` gated on the observed remote revision, and falls back to `Manual` when the source may not push content. Immutable-content backends report no revision and so never reach a content conflict.

#### Scenario: PreferRemote discards the local edit
- GIVEN a based placement edited locally and changed on the remote, `conflict = PreferRemote`
- WHEN the collection is synced
- THEN the remote content is pulled, the local body is a retained item, and the run reports `ContentChanged` and `Overruled`

### Requirement: A move delivers exactly one copy
A move is staged as a `Created` placement in the target plus a `Tombstone` in the source, each derived by its own collection's sync in whichever order the consumer runs them, and both halves can deliver the item on their own (pimdir SYNC §5). The create delivers by a server-side copy from its origin, or by uploading the body when the store holds it. The remove delivers by relocating the member into its destination, and is a plain delete once the destination holds the identity: `PimdirChange::Remove` carries the `link_id` its destination would receive, and a connector SHALL relocate only while that destination does not already hold it. A copy the target held before the move is not the move's delivery: a remove whose destination's create is minted (mutate.md) carries no `link_id`, so the connector relocates (vectors/sync/48). A connector that cannot relocate SHALL reject a remove carrying a destination rather than delete (SYNC §4): the destination has not received the member, and a delete would take the only copy.

Origin and destination are what the store derives from its bindings, never columns of the engine's own (SYNC §3): a `Created` placement's origin is where the same source binds the identity with a base present and, when the placement has a body, that body as its base; a `Tombstone` placement's origin is its destination, the collection where the same source holds a pending create of the identity, under that create's handle. Both pair a key with a create keyed by it or by the key minted over its provisional handle, the bare key first (`origin_for_link`, `destination_for_link`, vectors/sync/48 and 49). The engine SHALL derive `Remove { to }` from a tombstone's origin and `Add { origin }` from a create's, and SHALL rely on nothing it staged itself surviving a store round trip.

A relocated member is listed by the target's next listing under a new handle, and the page naming it lands the create rather than minting a second copy. A create holding neither an origin nor a body cannot deliver: it stays visibly pending until the consumer restages it, and the engine derives no push for it. Neither half may be dropped in favour of the other. A mutation of a placement no link id names is refused (mutate.md), so every tombstone with a destination carries a link id.

#### Scenario: The target syncs first
- GIVEN a linked member moved into a target
- WHEN the target's sync copies it, then the source's sync derives its remove
- THEN the destination already holds the link id, so the source is deleted rather than relocated, and the target holds exactly one member

#### Scenario: The source syncs first
- GIVEN the same move, the target's member not hydrated
- WHEN the source's sync runs first
- THEN the remove relocates the member, and the target's next listing carries it with its meta, the page naming it landing the pending create under the arrived handle: exactly one member

#### Scenario: A connector that cannot relocate
- GIVEN the same move over a connector answering no relocation
- WHEN the source's sync runs first
- THEN the remove is rejected and the tombstone stays; the target's sync copies from the origin, and the source's next remove, its destination no longer pending, is a plain delete

#### Scenario: A body-less move beside a held copy
- GIVEN a member holding no body moved into a target already holding its identity, the create minted
- WHEN the source syncs first, or the target does
- THEN the remove relocates carrying no link id, or the create copies from its origin and the remove is a plain delete: the target holds both copies and the move loses nothing

### Requirement: A lost push record can abandon a move
Where a move's staged edit rides ahead of its remove, a crash between the update being serviced and the write recording it SHALL leave the move abandoned rather than half-applied: the next run enumerates a revision the tombstone's base does not name, and an enumerate carries a revision and no body, so the replayed echo is indistinguishable from a remote edit. Edit-beats-delete SHALL then win, replacing the tombstone with a fresh pull, and the member SHALL stay in the source collection, live and clean at the pushed revision.

This is the conservative reading and the only one available: the alternative is deleting content on the strength of a revision the replica cannot account for. Nothing is lost either way. The edit landed, the member is where it started, and the consumer is free to stage the move again.

A move carrying no staged edit is unaffected. Its remove relocates the member, so a lost push record leaves the next enumerate listing nothing for the handle, and the tombstone is dropped as a delete both sides already agree on.

#### Scenario: A crash between a move's edit and its record
- GIVEN a member with a staged content edit, moved into a target
- WHEN the update the move pushes ahead of its remove lands and the write recording it is lost
- THEN the next run reads the revision as a remote edit, the tombstone is replaced by a fresh pull, and the member stays in the source collection

### Requirement: A read-only source reverts a local delete
Where `PimdirSyncOptions::push` is false a local delete can never propagate and the replica mirrors the source, so the merge SHALL treat the tombstone as a refused delete (below), reverted for a source alone in its collection. Applying it and waiting for a later enumerate to re-add the member only works against a complete snapshot: an incremental enumerate never lists an untouched member again, so the dropped row would never come back, leaving the replica permanently short of an item the source still holds. Reverting also keeps whatever the placement had cached.

What the revert undoes is the delete alone, and the placement lands on what it still owes: see the refused-delete rule below.

#### Scenario: A delta enumerate
- GIVEN a read-only source with a locally deleted member, unchanged upstream, owing nothing else
- WHEN an incremental enumerate lists nothing
- THEN the placement is written back clean, keeping its body

### Requirement: Both axes reconcile, every run
The flag axis SHALL run for every placement present on both sides, including one whose content axis derived a push. A push result is matched by handle, so one handle yields at most one change: the flag axis withholds its own push in that case, but still merges and writes. Skipping it outright loses a remote flag change until some later run happens to list the item again, which an incremental enumerate may never do.

A content push accepted in the same run SHALL rebase the placement the flag merge wrote, never the one read before it (pimdir SYNC §5): the engine keeps one pending entry per handle and the flag axis refreshes it whenever it writes the handle, so the row an acceptance lands is the last one the merge wrote and the pulled flag survives the rebase.

#### Scenario: A remote flag change beside a local content edit
- GIVEN a placement with a staged content edit whose remote also changed a flag
- WHEN the sync derives the content push
- THEN the merged flags are written in the same batch

#### Scenario: The push is accepted
- GIVEN the same placement
- WHEN the content push is accepted
- THEN the last row written for the handle holds the pulled flag and the pushed body as its base

### Requirement: A re-listed member is not a pull
A member a page lists again with the flags and the meta the store holds for it SHALL derive no write, no event and no count: nothing changed. One listed with other flags merges them; one whose meta moved, its revision not, has its summary and sort key refreshed in the page's write, no event reported. A base-less row holding a body whose member a listing no longer carries, a create-collision conflict whose remote side went, SHALL be resurrected as a `Created` placement, as a based edit is: new content beats a delete; a base-less row holding none is dropped.

### Requirement: A rejection counts for a pushed handle only
`PimdirSyncReport::rejected` SHALL count a `Rejected` outcome once per handle the chunk pushed, on the terms `pushed` counts an accepted one: a result naming a handle nobody pushed, or naming one twice, cannot inflate either.

### Requirement: A vanished local edit is re-staged as a create
A member absent from a complete enumeration, or listed vanished by a delta, whose placement holds a body its base does not SHALL NOT be dropped: new content beats a delete on both sides (pimdir SYNC §5, vectors/sync/16). The merge SHALL write a `Superseded` drop of its handle and a `Created` upsert under the link id's provisional handle (`PimdirLinkId::provisional`, `U+0001` then the link id), body, flags, summary and sort key kept, base and origin gone, reporting `Vanished` and counting the pull; the next run adds it, no push going in the same run. A placement whose body its base holds is dropped `Deleted` as before.

#### Scenario: The server deleted what was edited offline
- GIVEN a placement edited locally whose member a complete enumeration no longer lists
- WHEN the collection is synced
- THEN its handle is dropped `Superseded`, the body sits on a `Created` placement under the provisional handle, and the next sync appends it

### Requirement: A create waits for the page that lands it
A `Created` placement SHALL derive no `Add` from a page before the last of a round, nor from a delta while a round is open (pimdir SYNC §5, vectors/sync/25): a page not landed yet may carry the create arrived, by a relocation or an add whose record was lost, and the page naming it lands the create; pushing first lands two copies. A member a page lists in the same run is named before any create is derived, so a page carrying the arrival lands it rather than pushing (vectors/sync/21). A pending create carrying no origin whose summary `date` the run's scope excludes SHALL wait for a round whose scope holds it, an arrival relocated with its `Date` kept being listed by no page of that scope, and is counted in `PimdirSyncReport::waiting` (vectors/sync/43). A mutable arrival carrying a body other than the staged one SHALL land as the both-changed case (pimdir SYNC §6): its base holds neither body nor revision, the staged body and the arrival's flags stay, and the policy decides. Landing it on the arrival's body as the base read the staged body as an edit and pushed it over a body the two never agreed on.

#### Scenario: A copy into a collection never listed
- GIVEN a pending create in a target whose first listing carries other members
- WHEN the target is synced
- THEN those members are named by the page that lists them and, none being its arrival, the create is pushed once the round's last page has landed

#### Scenario: An old message relocated into a scoped target
- GIVEN a pending create dated August and a run over a scope since September
- WHEN the round's last page lands without it
- THEN no `Add` is derived and the report counts it waiting

#### Scenario: Two servers holding one identity under two bodies
- GIVEN a source holding a card and a second source whose server holds the same `UID` under another body, the hub offering the first as a pending create
- WHEN the second source syncs under `Manual`
- THEN the create lands conflicted, its body the staged one and the server's recorded as the divergence, the base holding no body, and nothing is pushed

### Requirement: An item-level conflict is held, a binding conflict is reconciled
`reconcile_content` SHALL take its conflict branch for a placement carrying a `conflict_revision`, the divergence between this source and its own remote. A `Conflict` placement carrying none is the item's cross-source conflict projected onto this source ([hub](hub.md)): it SHALL derive no push, a pending create under it included, until an `Edit` or a `Remove` settles the item (pimdir SYNC §3, §7). A remote revision its base does not hold SHALL still be recorded, whatever the conflict policy: the binding is marked conflicted with that revision and its diverging body wanted, reported as `Conflicted`, since an incremental enumeration never lists the member again (pimdir SYNC §5, vectors/sync/33).

#### Scenario: Two sources diverged under `manual`
- GIVEN an item flagged conflicted by the hub, every binding projecting `Conflict` with no revision
- WHEN either source is synced and its remote lists nothing new
- THEN nothing is pushed and nothing is re-marked, until an edit through one source settles it

#### Scenario: A server edit lands while the item is conflicted
- GIVEN an item flagged conflicted by the hub, and one source's remote listing the member at a revision past that binding's base
- WHEN that source is synced
- THEN the binding is marked conflicted with the listed revision, its body wanted, and the run reports one conflict

### Requirement: A push is counted when it matched
`PimdirSyncReport::pushed` SHALL count the changes this run derived and the remote accepted, not the results the consumer reported: a result naming a handle nobody pushed, or naming one twice, cannot inflate it.

### Requirement: A run records its pushes in bounded chunks
A sync SHALL push its derived changes in bounded chunks, yielding the writes a chunk produced before the next chunk is pushed: one `WantsPush` per chunk, each followed by the `WantsWrite` recording its outcomes. The bound is the engine's (`PimdirSync::PUSH_CHUNK`), not a consumer option, because what it bounds is a crash window rather than throughput.

Pushes stay at-least-once, but the window is one chunk rather than one run: a crash between a serviced push and the write recording it replays only the chunk whose write never landed, since every earlier chunk is already recorded and a later one was never sent. A driver therefore MUST NOT assume one push and one write per run; the reference driver services yields in a loop and needs no change.

Only the handles of the chunk being serviced SHALL be resolved when its outcomes land: a handle a chunk never reported on is left pending exactly as before, and a later chunk's handles are still waiting for their own outcome.

#### Scenario: A crash after the first chunk
- GIVEN a run deriving more changes than one chunk holds
- WHEN the first chunk is pushed and recorded, and the write recording the second chunk is lost
- THEN the placements of the first chunk are recorded clean, and only the second chunk's are still pending

### Requirement: The checkpoint lands in the last write
The checkpoint, a delta's or the one a round closes with (`CloseRound`), SHALL land in the write that follows the final chunk, and SHALL stay the pre-push one, which is what makes the engine's own echo re-listed by the next delta enumeration. An intermediate chunk's write SHALL NOT carry it, so an interrupted run resumes from the same cursor rather than from one claiming its unrecorded pushes were seen. A page's resume cursor and the checkpoint an open round carries are not the source's checkpoint: they land with their page (`SetRoundCursor`), in the write after its last chunk.

### Requirement: Every change carries an idempotency key
A `PimdirChange` SHALL be a `PimdirChangeKind` (what the remote is asked to do, the four verbs that were the change itself) plus the `PimdirChangeKey` naming it. The key SHALL be derived as pimdir SYNC §4 fixes it, so two engines over one store key one change alike and every vector's key reproduces: FNV-1a 64 bits over fields each followed by one `0x00` byte, rendered as sixteen lowercase hexadecimal digits; the fields being the collection id, the handle, the kind as `add`, `remove`, `set-flags` or `update`, then the kind's own, an optional value as the field `1` followed by the value or the field `0` alone, and a flag set as the field `unknown` or the field `known`, the count in decimal ASCII, then each flag in code point order. `add` folds the link id, the flags, the origin as `1`, its collection and its handle or `0` alone, then the object hash; `remove` the destination; `set-flags` its `add` set then its `remove` set, each a known flag set; `update` the object hash. The same derived change SHALL key the same on every run, and changes differing in any of those SHALL key differently.

A precondition is deliberately not part of it: `if_match` states what the change was attempted against, not what it makes true, and a retry of one operation is one operation.

The split is what keeps the key honest. `PimdirChange::new` is the only way to make one, so a change cannot exist without a key; the engine derives a *kind* and keying it is the last thing that happens to it, so there is no state in which a keyed change is half-built or names something other than what it carries.

Recording the key is what makes the at-least-once contract actionable for every kind: an add could already be recognised by its `link_id`, but a flag set, a content update and a remove carried nothing a consumer could log to recognise a replay of *this* operation.

#### Scenario: A replayed change keys the same
- GIVEN a change an interrupted run pushed
- WHEN the next run derives it again from the same local state
- THEN it carries the same key, and the consumer recognises the replay

### Requirement: An enumeration is ordered by handle
`PimdirRemoteSnapshot::items` SHALL be sorted by handle and SHALL list each handle at most once (pimdir SYNC §4). The merge walks it beside the local placements in that order rather than indexing it, which is what keeps a whole-collection sync from copying both key spaces to join them.

The engine SHALL NOT depend on a consumer honouring this: a snapshot that arrives unsorted is sorted, and a handle listed twice is collapsed to its first item, so getting it wrong costs a pass rather than correctness. `PimdirHandle` orders as bytes, so an IMAP SEARCH's ascending UIDs are not sorted under it (`"10"` sorts before `"9"`), and a connector handing them over numerically is one the engine sorts on every run.

#### Scenario: An unordered enumeration
- GIVEN a snapshot whose items arrive in any order
- WHEN the collection is synced
- THEN it derives exactly what the same snapshot sorted derives

### Requirement: The merge joins the two sides rather than copying them
The merge SHALL pair local placements with remote items by walking both in handle order, taking each placement rather than copying it per candidate. A copy SHALL be made only where a write takes ownership of one.

This is a shape requirement rather than a behaviour one, and it is stated because the shape is what the cost is: the candidate set, the order it is merged in and every change derived from it stay exactly as they were, and the property comparing a delta run against a full one is the guard on that.

A delta snapshot keeps its own rule for which of the joined handles is a candidate: the ones it reported changed or vanished, plus every locally non-clean handle, whose pending push it would otherwise never revisit. A never-based one (a staged create) is a candidate with no remote state to merge against, since the enumeration will not mention it until it lands.

### Requirement: A write batch is bounded and cut between candidates
The merge SHALL hand a write batch over once it holds `PimdirSync::WRITE_CHUNK` writes, rather than holding one write per member until the last candidate is resolved. What this bounds is memory rather than a crash window: a lost batch costs a re-merge, where a lost push costs a round trip.

A batch SHALL be cut between candidates and never inside one. The writes one candidate derives are consistent only together: a vanished local edit is a `Superseded` drop of its handle beside the `Created` upsert carrying its body, and a cut between them would lose that body if the next batch never landed.

The checkpoint rule is unchanged and is what makes a partially merged run safe to resume: an intermediate batch carries no checkpoint, so a run interrupted mid-merge re-enumerates from the same cursor and re-derives whatever it had not recorded.

#### Scenario: A merge larger than one batch
- GIVEN a snapshot deriving more writes than one batch holds
- WHEN the collection is synced
- THEN the writes arrive in several batches, and only the last one carries the checkpoint

### Requirement: A refused delete is decided from the other sources
A local delete that cannot go, `push` false, `rights.remove` false, or a move's staged edit that cannot ride ahead of it (the move must not go without the edit, or the relocated member would carry the body the edit replaced), SHALL be decided from whether another source syncs the collection, which the consumer states through `PimdirSync::beside_other_sources` (pimdir SYNC §5). The std client answers it from `collection_sources`, the sources holding a binding or a checkpoint in the collection, this handle's own left out.

A source alone reverts: the delete alone is undone and the placement lands on what it still owes with whatever it had cached, `Conflict` while it carries an unresolved divergence, `Dirty` while it carries a staged body its base does not hold, and `Clean` when it owes neither. The move destination the tombstone carried SHALL be dropped with the delete it belonged to. A source beside another holds the tombstone as it is, since a revert would read as a resurrection there and the other source pushes the delete; the row stays in the trash until that source has dropped it. Either way the engine SHALL derive no push and SHALL NOT apply the delete to the replica.

Landing such a row `Clean` loses what the delete was not. The content axis derives a push from a dirty placement alone, so a staged body left clean is pushed by no later run and the replica keeps content the source has never heard of; an unresolved divergence left clean reads as an ordinary local body on the next run and is pushed over the remote it diverged from, deciding for the consumer the question they were asked; and a destination left on a settled row turns the next plain delete of that member into a relocation nobody asked for.

Reverting alone is what an incremental enumeration needs: a held tombstone hides a member the source still holds, and hides it for good, since a delta never lists an untouched member again. Holding beside others is what the hub needs: a revert states that this source still holds the member, which the hub reads as the item being alive (add-beats-delete across sources), clearing the deletion for every source and mirroring the item back to the one it was deleted on. There is no configured policy: the answer is a fact about the collection, not a preference.

Deletion is the only axis needing this. A refused flag or content change stays dirty and re-derives every run, but a refused delete has to be either undone or held.

#### Scenario: The two refusals agree
- GIVEN a tombstoned placement the source still holds
- WHEN it is synced with `push = false`, and again with `rights.remove = false`
- THEN both follow the same rule: reverted for a source alone, held beside another

#### Scenario: A staged edit outlives the delete
- GIVEN a placement with a staged content edit, then deleted locally, on a source that will not take the delete
- WHEN the collection is synced
- THEN the delete is reverted, the placement reads `Dirty`, and a later writable run pushes the edit

#### Scenario: A refused delete does not settle a divergence
- GIVEN a conflicted placement deleted locally, on a source that will not take the delete
- WHEN the collection is synced
- THEN the placement reads `Conflict` again, still waiting for the consumer's edit

#### Scenario: A reverted move goes nowhere
- GIVEN a move whose source half the source refuses
- WHEN the collection is synced
- THEN the placement lands clean with no destination, so a later delete of it is a delete

### Requirement: The flag axis settles the flag axis alone
Wherever the flag axis writes a placement it has settled, whether it pulled the remote set, rebased onto a shared move or recorded an accepted push, it SHALL leave the status where it found it while the content axis still owes a push for that placement, exactly as it leaves an unresolved conflict alone.

A placement owes a content push when it is dirty and points at a body its base does not hold, which is the one reading the content axis derives from. Landing it clean loses the edit outright rather than deferring it: the content axis derives from a dirty placement, so no later run derives anything for a row that reads as in sync, and the replica keeps a body the source has never heard of, with nothing left to say so.

The flag merge itself is unchanged. It runs for every placement present on both sides, folds remote-won flags in, and derives its own push under the same rights as before.

#### Scenario: A source that takes flags and refuses content
- GIVEN a placement with a staged content edit and a local flag change, on a source permitting flag pushes only
- WHEN the collection is synced
- THEN the flag change is pushed and recorded, and the placement still reads dirty with its staged body

#### Scenario: A remote flag change over a local edit
- GIVEN a placement with a staged content edit whose remote moved a flag
- WHEN the collection is synced
- THEN the merged flags are written and the placement still owes its content push

### Requirement: A conflict keeps the body it diverged from
A placement marked conflicted SHALL carry `conflict_object`, the remote body at the revision `conflict_revision` names, so the divergence can be read without asking the remote for it. Both SHALL be set together, taken together into the base when an edit resolves the conflict (see [mutate](mutate.md)), and dropped together when the tracked revision moves: a body that outlives the revision recorded beside it describes a version the server no longer holds, and a resolver trusting it would merge against a phantom.

The engine fetches nothing, so the body is requested rather than taken: marking a conflict marks the body wanted and the [upgrade](upgrade.md) pass supplies it. A conflict whose body has not yet landed is visible and unresolvable, as a placement named at `Meta` is visible and unreadable until its body lands.

Storing it is what lets resolution leave the process that found it. A resolver holding base, local and remote needs no credentials, no backend and no network, and a conflict between two hand-edited bodies is decided by a human long after the run that found it.

The hub round-trips both halves through `PimdirBinding`, on the per-source axis and independently of the cross-source `PimdirHubItem::conflict_object`.

#### Scenario: The diverging body is stored, not refetched by the resolver
- GIVEN a based placement edited locally and changed on the remote, `conflict = Manual`
- WHEN the collection is synced and the wanted body is supplied
- THEN the placement holds the remote body beside the observed revision, and base, local and remote are all readable from the store

#### Scenario: A remote that moves again invalidates the stored body
- GIVEN an unresolved conflict whose stored body matches its recorded revision
- WHEN a later sync observes a newer remote revision
- THEN the recorded revision advances and the stored body is dropped in the same write

### Requirement: Events report what the remote changed
A sync's `PimdirSyncEvent`s SHALL report what was pulled (`Added`, `FlagsChanged`, `ContentChanged`, `Vanished`), a divergence (`Conflicted`, the source's own or one its write records on the item, which replaces the `ContentChanged` of the pull the item did not take), an accepted add under its assigned handle or a create a page landed (`Created`), and a body a policy did not adopt (`Overruled { handle, link_id }`, naming the retained item), in order, each once the write recording it committed, and nothing for an accepted flag, content or delete push (pimdir SYNC §5). An upgrade and a mutation report `Overruled` alone, on `PimdirUpgradeReport::events` and `PimdirMutateReport::events`; a rekey reports none.

### Requirement: A page names every member it carries
Every member a page lists SHALL be named in the page's write from its `PimdirRemoteMeta` (pimdir SYNC §4, §6), in handle order and against the whole collection, a page before the last loading the `Handles` it lists and the `Links` of the hints it carries: a hint a pending create of this source holds, keyed by the hint or by the hint minted over its provisional handle, the bare key first, lands that create (vectors/sync/50) (a `Superseded` drop of the provisional handle, the create under the listed handle, based on the reported flags, revision and carried body or else its own, its staged flags and body kept, reported `Created`); a free hint keys it and a held one is minted, the item inserted at `Meta`, or `Full` with a body the member carried, based on the reported flags, revision and body, reported `Added`. A bound member's meta refreshes its summary and sort key when its revision did not move, a walked attachment mark kept while it holds its body, and a mutable member stating another hint is keyed afresh. A pull takes the member's meta, and the body it carried as its body and base at the listed revision, the level `Meta` when it carried none (vectors/sync/11); under `Manual` a carried diverging body lands in `conflict_object`. A remote edit past the base of a local tombstone revives the placement as such a pull, nothing pushing. A tombstone also holding a staged edit, a body its base does not, is the both-changed case and follows the conflict policy instead (pimdir SYNC §5): `Manual` revives it conflicted at the observed revision, `PreferRemote` pulls, `PreferLocal` pushes the staged body.

### Requirement: The merge is a module of its own
`Join`, `Merge` and `Candidate`, the walk of both sides in handle order and the delta rule narrowing it, live in `sync/join.rs`; the unit tests of every verb live beside their module in a `tests.rs`. A module with code and submodules is `foo.rs` plus `foo/`.

### Requirement: A run chooses its listing from the coverage
A sync SHALL read the source's checkpoint, coverage and round first, a `Handles` load naming none, and ask for one listing (`PimdirEnumerate`: a `PimdirListing` and a `PimdirScope`, pimdir SYNC §5): the open round resumed from its cursor when it lists the scope asked for and is a band round exactly when this run would list a band (`PimdirRound::band`); else a round, opened in the write of its first page, when no round ever closed (a store from an earlier draft included), a round is open over another scope or of the other kind, `full` is set, or the scope reaches outside the coverage, over the band alone when the connector's checkpoint is bound to no scope (`PimdirRemote::scope_bound`, `PimdirSync::scope_bound`) and the band adjoins the coverage; else a delta from the checkpoint, recording the narrower coverage (`SetCoverage`) when the scope lies strictly inside it. A connector answering a round to a delta request has one opened over the scope; one answering a delta to a round request is merged as a delta. A cursor the source rejects (`PimdirEnumerated::CursorRejected`) restarts the round under a new id; a rejected delta checkpoint opens a round.

#### Scenario: A widening on IMAP
- GIVEN a coverage since September and a connector bound to no scope
- WHEN the scope widens to July
- THEN the round lists July to September alone, hands no checkpoint, and closes with the coverage since July, the checkpoint kept

#### Scenario: A band round open in a store an earlier build reconciled
- GIVEN a round open over the band July to September that the store reads as no band round, its column added on open
- WHEN a sync bound to no scope widens to July
- THEN it restarts the round as a band round rather than resuming it from its cursor

### Requirement: A round lands page by page
Every page SHALL land in one write: its members named and merged, the handles it listed stamped with the round's id after its upserts (`Stamp`), and, in the write after its last push chunk, its cursor with any checkpoint it carries (`SetRoundCursor`) or, on its last page, the round closed (`CloseRound`) with the checkpoint that page carried, else the one an earlier page carried, else the source's own, and the coverage of its scope or of the band's span; `OpenRound` leads the batch of the round's first page. A page before the last merges its members and vanished handles alone; a delta and a round's last page merge as a delta. A round interrupted between pages keeps what landed, and `PimdirSync::report` answers what those pages did.

#### Scenario: An interrupted round resumes
- GIVEN a round whose first page landed with the cursor `p1` before the connector failed
- WHEN the next sync runs under the same scope
- THEN it asks for the round from `p1`, and the last page closes it with the coverage of its scope

### Requirement: Absence means deleted in scope only
The deletes a round infers SHALL be the based bindings of its source no page of the round stamped and whose item's summary `date` falls in its scope, or is unknown and the round is no band round (`NULL`, an empty `date` a connector hands being stored as `NULL`, so a round resumed in a later run and one opened and closed in the same run find the same members absent), read when its last page lands (`PimdirLoaded::unstamped`; for a round that page opens, the based placements it did not list), each handled as a vanished member is. A band round (`OpenRound { band: true }`, `sources.round_band`) SHALL infer no delete of an undated member: its date filter never returns undated mail, so the member is left to rounds over a whole scope and to deltas, and a vanished handle still applies to it; the in-memory rule (`RoundPlan::finds_absent`) and `list_unstamped_bindings` agree. A placement out of scope is neither dropped, pulled nor pushed on the evidence of its absence; a vanished handle applies whatever the date; the engine filters no page by date and names every member a page carries. A `Remove` is derived from a tombstone the consumer staged and from nothing else.

#### Scenario: A message older than the scope
- GIVEN a bound member dated August and an undated one, both unlisted, under a scope since September
- WHEN the round's last page lands
- THEN the August member stays as it was, and the undated one is dropped `Deleted`

#### Scenario: A band round and an undated member
- GIVEN a coverage since September, a bound member dated August and an undated one, both unlisted, and a connector bound to no scope
- WHEN the band round widening to July lands its last page, in the run that opened it or in a later one resuming it
- THEN the August member is dropped `Deleted` and the undated one stays as it was

#### Scenario: An empty date on a resumed round
- GIVEN a bound member whose summary carries an empty `date`, unlisted by a round that an earlier run opened
- WHEN the resumed round's last page lands
- THEN the member is dropped `Deleted`, as it is by a round opened and closed in one run

### Requirement: A scope bounds mail only
`PimdirSyncOptions::scope` is unbounded by default. `PimdirSourceStore::sync` and `prepare_sync` SHALL refuse a bounded scope on a collection whose kind is not `message/rfc822`, naming the kind (`PimdirError::Scope`).

### Requirement: A resumed round lists its head first
A run resuming an open round SHALL first ask for the round's head when the connector lists heads (`PimdirRemote::heads`, `PimdirSync::heads`). The head is a delta head from the round's checkpoint, or for a band round from the source's checkpoint, when the connector lists delta heads; else a band head from `round_started_at`, when it lists band heads; else none. It is checked against the run's scope and merged as a page before the last.

A delta head stamps its members and records its checkpoint as the round's (`SetRoundCursor` with no cursor). A band head stamps nothing and records nothing, the round's last page listing its band again. Neither derives an `Add`, infers an absence or moves the cursor. `HeadRejected` restarts the round over the run's scope, never a band, and `NoHead` resumes the round (pimdir SYNC §5, vectors/sync/62 to 69).

#### Scenario: An arrival during an open round
- GIVEN a round left open on checkpoint h1000 and cursor t1
- WHEN the next run's head lists a new member with checkpoint h1001, and the last page lands
- THEN the member is named and stamped, it survives the close, and the round closes on h1001

### Requirement: The superseded delete is held until the add is pushed

`PimdirLoaded::held` SHALL list the handles of the loaded tombstones `list_superseded_pending` names, which the client loads on every scope. The sync SHALL derive no `Remove` (and no ride-ahead `Update`) for such a tombstone and leave it pending; a remote edit over it still revives it. Once the superseding create is accepted, withdrawn or gone, the tombstone derives a plain `Remove` (SYNC §5).

#### Scenario: a refused push leaves the source in place
- GIVEN task t1 in tasks-a superseded by an add into tasks-b, both synced by one source
- WHEN tasks-a syncs, tasks-b syncs and its create is rejected, and tasks-a syncs again
- THEN no `Remove` of t1 is pushed and t1 stays on the server; once the create is accepted, the next sync of tasks-a pushes a plain `Remove` (tests/engine/supersedes.rs, vectors/sync 71 and 72)

### Requirement: A create keeping its identity names the member to move

`PimdirLoaded::superseded` SHALL map each loaded pending create whose item superseded one of its own identity elsewhere, bound by this source with a base, to that binding (`placement::PimdirSuperseded { collection, handle, if_match }`, `list_superseded_bindings`). The sync SHALL carry it on the create's `PimdirChangeKind::Add` as `supersedes`, outside the change key. A connector able to move members SHALL be free to deliver it by a move then a write of `object`, answering the moved member's handle; the superseded tombstone's delete that follows finds the member gone and is accepted (SYNC §3, §4).

#### Scenario: a filing moved on a server keeping one identity per account
- GIVEN msg-a in inbox, filed into archive with its identity kept, on a remote refusing an append whose identity another collection holds and moving superseded members
- WHEN archive syncs, then inbox
- THEN the member moves into archive under its handle, never in two places, and inbox's delete passes with nothing left (tests/engine/supersedes.rs, vectors/sync 73); without a move the create is refused and the source kept

### Requirement: A page that cannot be resumed past is refused
A round page (`complete`) that is neither `last` nor carries a cursor SHALL complete the sync with `PimdirArgError::UnresumablePage`, writing nothing of the page: read as the last, it would find absent every member the round has not listed (pimdir SYNC §4). What earlier pages landed stays, the round open. A last page carrying a cursor closes the round.

#### Scenario: A connector dropping its cursor
- GIVEN two synced bindings and a round page listing one of them with `last` false and no cursor
- WHEN the sync takes the page
- THEN it completes with `UnresumablePage` and writes nothing, the other binding kept (src/sync.rs `a_page_neither_last_nor_cursored_is_refused`)

### Requirement: An add accepted without a handle waits for its page
An `Add` answered `Accepted` with no assigned handle (an IMAP `APPEND` on a server without UIDPLUS) SHALL leave the create pending: its provisional binding, staged body and flags kept, no `Superseded` drop written, no `Created` reported, and no second push in the run; the page naming the member lands it (pimdir SYNC §4, vectors/sync/92).

#### Scenario: An APPEND answered with no UID
- GIVEN a pending create the remote accepts with no assigned handle
- WHEN the outcome is settled
- THEN nothing is written for it and nothing is reported (src/sync.rs `an_add_accepted_without_a_handle_stays_pending`)

### Requirement: Another message under a held Message-ID is minted apart
A member whose hint the collection holds under an item another source binds SHALL name that item only when its mail summary agrees with the item's on `subject`, `date` and `sender`, and SHALL otherwise be keyed by the identity minted over its handle (pimdir SYNC §6, §9, vectors/sync/93). The page's own naming applies it to an offered copy it would land; the write applies it to every upsert binding a new source to an item another source binds, since a page before the last and an upgrade name their members against this source's holders alone (pimdir SYNC §10).

#### Scenario: Another message reusing the identifier
- GIVEN source a holding `basic-1@example.org`, the stand-up notes
- WHEN source b writes a member under that key whose summary names another subject and sender
- THEN b's member is keyed `dup:basic-1@example.org#77` and a's item is left as it was; the same message served by b joins a's item (tests/another_message.rs)

### Requirement: A body is verified before it is pushed
Before a chunk is pushed, the sync SHALL yield `PimdirYield::WantsVerify` with every body its changes upload (an `Add` holding no origin, an `Update`), answered by `PimdirArg::Verified`, the bodies whose blob reads whole and hashes to its name (`PimdirSourceStore::verify`, pimdir STORAGE §7). A change whose body is missing or corrupt SHALL be withheld, its placement left as the merge wrote it, and counted in `PimdirSyncReport::unreadable` (vectors/sync/94).

### Requirement: An event follows its commit
A sync SHALL report an event only once the write recording it has been answered, and every event of an answered write (pimdir SYNC §5): the events derived for a batch wait in the coroutine and move to the report when the write is answered, so a run whose write fails reports nothing of it.

#### Scenario: A write never answered
- GIVEN a page naming a new member
- WHEN the run yields the write and it is not answered
- THEN the report carries no event, and `Added` once it is (src/sync.rs `an_event_waits_for_the_write_recording_it`)

### Requirement: A carried body equal to the edit settles it
A member listed at a revision past the base and carrying the very body the placement's local edit holds SHALL settle as the push whose record was lost: the base adopts the revision and that body, nothing conflicts and nothing is pushed (pimdir SYNC §5; sync-audit S13).

#### Scenario: A lost push record
- GIVEN a local edit `h2` over `h1@r1` and a listing at `r2` carrying `h2`
- WHEN the collection is synced
- THEN the placement is `Clean` at `h2@r2` with no conflict (src/sync.rs `a_carried_body_equal_to_the_edit_settles_it`)

### Requirement: The flag axis with no base merges the union
A placement present on both sides with no base SHALL merge its flags as the union of the two known sets, no side being known to have removed anything, pushing what its server lacks (pimdir SYNC §5; sync-audit S15); one whose flags agree is left alone.

#### Scenario: A base-less placement on both sides
- GIVEN a placement with no base holding `flagged` and its member listing `seen`
- WHEN the collection is synced
- THEN both flags are kept and `flagged` is pushed as an addition (src/sync.rs `no_base_merges_the_union_of_both_sets`)

### Requirement: A flag push names what it adds and removes
`PimdirChangeKind::SetFlags { handle, add, remove }` SHALL carry two known sets in place of a replacement set, so a flag the server changed after the listing survives the push (pimdir SYNC §4, vectors/sync/02, 03, 09, 26, 27, 42). The engine SHALL derive both from the flag merge against the set the source holds, the one the run listed, else the base: `add` what the merge carries and that set lacks, `remove` what that set carries and the merge lacks; a placement whose merged flags are unknown pushes none, and one deriving two empty sets pushes nothing. A connector SHALL apply both as deltas (an IMAP `STORE +FLAGS` and `-FLAGS`, a JMAP keyword patch), never as a replacement. The change key's `set-flags` fields are `add` then `remove`, each a known flag set.

#### Scenario: A local addition beside a server's own flags
- GIVEN a placement adding `\Flagged` to a member the server lists with `\Seen`
- WHEN the collection is synced
- THEN the push adds `\Flagged` and removes nothing, keyed `892471aec94fc225` (vectors/sync/02, src/change.rs `the_derivation_reproduces_the_vectors_keys`)

### Requirement: A mass absence parks the round
A round's last page finding absent more than half of the based bindings of the source in the round's scope, counted as the deletes are, SHALL NOT close the round when those bindings number at least 20 (`PimdirParked::MASS_ABSENCE`), unless the run carries `PimdirSyncOptions::accept_absences` (pimdir SYNC §5, vectors/sync/77, 78). It drops nothing and writes no `CloseRound`: the page lands as a page before the last, its members named and stamped, its checkpoint recorded with no cursor, and `PimdirSyncReport::parked` carries the absent and the bound counts. A round that held a cursor is drawn anew in that write, so the next run lists it from its first page and decides again. `accept_absences` is per run and never stored.

### Requirement: A delta page is the last
A delta page (not `complete`) that is not `last` SHALL complete the sync with `PimdirArgError::UnresumablePage`, writing nothing of it: a delta is one page, always the last, and its checkpoint would claim the changes the pages it says follow list (pimdir SYNC §4; sync-audit S12).

#### Scenario: A paged delta
- GIVEN a delta page with `last` false, a cursor and a checkpoint
- WHEN the sync takes it
- THEN it completes with `UnresumablePage` and lands no checkpoint (src/sync.rs `a_delta_page_not_last_is_refused`)

### Requirement: A restated identity keeps the local edit of the old one
A mutable handle stating another identity under itself retires the old binding (SYNC §5); a local edit the remote never saw beats that delete, so the engine SHALL re-stage the old placement as a pending create under the provisional handle its key derives, licensed by a `Superseded` drop of the handle, before the handle names the new identity, as it re-stages a vanished member holding one.

#### Scenario: An edited item whose handle now names another
- tests/engine/sync_audit.rs `a_restated_identity_keeps_the_local_edit_of_the_old_one` (failed before the change: the local body was dropped with the old binding)

### Requirement: The report says what a run left pending
`PimdirSyncReport::unanswered` SHALL count the pushes the connector reported no outcome for, each left pending as a rejected one is; `rejected_creates` SHALL name, by provisional handle, the pending creates the remote rejected in the run, also counted in `rejected`, so a create its server refuses run after run is reported rather than pushed for ever in silence (SYNC §5's push discipline). A run returns once its listing's last page landed, so a round it opened or resumed is closed when it returns Ok, unless `parked` says it parked it. No new event kind: the vectors' event vocabulary is closed.

#### Scenario: An unanswered push and a refused create
- tests/engine/sync_audit.rs `an_unanswered_push_is_counted`, `a_rejected_create_is_reported_on_every_run` (both failed before the change: nothing counted, nothing named)

### Requirement: An unread arrival landing a create conflicts
A mutable member listed at a revision with no body that lands a pending create holding a body SHALL leave the binding conflicted at the listed revision with no `conflict_object`, its base at that revision naming no body, whatever the source's policy (SYNC §6): nothing has read what the server holds at that revision. The page SHALL report `Conflicted` beside the landing's `Created` and push nothing for it. An upgrade landing a create from a fetch carrying no body SHALL do the same; one carrying a body other than the staged one SHALL record it as the divergence's `conflict_object`, the base at the fetched revision naming no body. The upgrade fetching the body of such a binding clears the conflict when it equals the staged one, the add whose record was lost having landed.

#### Scenario: The vectors
- vectors/sync 97 (the landing under `prefer-local`) and 98 (the upgrade fetching the staged body clears), tests/vectors_sync.rs
- src/upgrade.rs `an_unread_fetch_landing_a_mutable_create_conflicts`, `a_fetched_body_other_than_the_create_conflicts`
