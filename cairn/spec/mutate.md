---
cairn: spec
capability: mutate
status: current
---

# Mutate

`PimdirMutate` is the I/O-free coroutine that applies a local edit to one collection offline, with no network. It loads the target placement, stages the resulting `PimdirWriteOp`s, and lets the driver write them; the remote is reconciled by the next `sync`. This is the write half of the "generic in the data, disciplined in the writes" rule: a client never writes storage rows directly; it stages a `PimdirMutation`, so the sync always knows what changed.

### Requirement: The offline mutation vocabulary
`PimdirMutation` SHALL stage a local edit to one collection offline, reconciled on the next sync:

- `SetFlags { handle, flags, base }`: set a placement's flags and mark it dirty (a pending create stays `Created`, an unresolved conflict stays `Conflict`, a tombstone stays a tombstone; the flag change rides along). Carrying no `base` it replaces the set by `flags`; carrying one it changes the set (below).
- `Remove { handle, if_match, resolves }`: tombstone a placement, kept until synced. A `Remove` or a `Move` of a delivered create (based under its provisional handle, pimdir SYNC §2) tombstones it like any based placement, its push held until the page naming its member lands it ([sync](sync.md)); only a pending create with no base is withdrawn. Absorbed as a staged delete (the item is marked deleted, its binding kept), so the next sync pushes the remove. A `Remove` settles a conflict only as a person's resolution naming it, on an `Edit`'s terms (below): resolving its binding's, the base adopts `conflict_revision` and `conflict_object` together, as an `Edit` does, so the delete pushes against what the remote holds (pimdir SYNC §7, vectors/sync/32); resolving the item's, it states a `Settle` before its tombstone ([hub](hub.md), vectors/sync/146). A `Remove` on a placement with no base, a pending create or the copy the hub offers a source lacking the item, withdraws it in the same write: the tombstone upsert is followed by a `Deleted` drop of its handle, so no binding is left, the item is retained when nothing else binds it and is a delete every other binding pushes otherwise (pimdir SYNC §7, §9, vectors/sync/51).
- `Edit`: store a new body and repoint the placement at it (full level), keeping the base so the next sync derives the push. An edit carrying `if_match`, the body its producer read (a queued `update`'s, STORAGE §15.3), SHALL fail as `PimdirMutateError::Replaced { handle, read, held }`, staging nothing, when the placement holds another body (pimdir SYNC §7, vectors/sync/87). An edit whose object is the one the base already holds stages nothing and SHALL leave the status where it found it, `PimdirPlacement::staged_edit` being the single reading of "there is a local content edit here"; every other edit marks the placement dirty. An edit settles a conflict only as an overrule or as a person's resolution naming it in `resolves` (below); a resolution of a conflicted placement resolves it whatever body it carries, the base adopting the remote state observed at conflict time, both halves of it (see below), and editing a tombstoned one revives it (see below).
- `Copy`: stage a `Created` placement in a target under the identity's provisional handle (below), carrying the source origin; the source is untouched.
- `Move`: stage a `Created` placement in the target under the identity's provisional handle (carrying the source origin), **and** tombstone the source. A move is thus a copy into the target plus a remove from the source, both derived on the next sync; the source's tombstone and the target's create land in their respective collection hubs. The destination the tombstone carries is what the store derives from the target's pending create on every load (pimdir SYNC §3); the one staged here serves a consumer keeping the placement as written. A move meeting a conflict is refused, staging nothing (below).
- `Add`: see below.
- `Abandon { handle }`: give up a delivered create whose member never arrived, on a person's request (below). It has no queue action.

A mutation SHALL touch the local replica only; the remote is reconciled by sync. It answers a `PimdirMutateReport` whose `events` carry the `Overruled` its write reported, a body a policy did not adopt kept as a retained item ([hub](hub.md), vectors/sync/85, 86), and the `Abandoned` of a delivered create given up, once the write committed.

### Requirement: Add stages a locally-authored create
`PimdirMutation::Add { link_id, flags, object, body, summary, sort_key }` SHALL stage a brand-new item with no remote origin (compose, import): a `PimdirStatus::Created` placement in the coroutine's collection under the link id's provisional handle, at `level = Full`, with `base = None` and `origin = None`, pointing at `object`; plus a `StoreObject` carrying `body`. Because the create has no origin, the next sync SHALL push it as `PimdirChange::Add { origin: None }`, an append that uploads the body rather than a server-side copy. `Add` SHALL NOT require an existing source placement, and SHALL fail (`PimdirMutateError::LinkExists`) rather than overwrite when a live (non-tombstone) placement already holds `link_id`; a tombstoned `link_id` does not block the create.

A `Remove` carrying `if_match`, the body its producer read and deletes (a queued `remove`'s, STORAGE §15.3), SHALL fail as `PimdirMutateError::Replaced`, staging nothing, when the placement holds another body, as an `Edit` does (pimdir SYNC §7, vectors/sync/145; src/mutate.rs `a_guarded_remove_refuses_a_replaced_body`).

A mutation naming a placement no link id names SHALL fail with `PimdirMutateError::Unnamed`: nothing keys a status staged on it. Every member a listing carries arrives named (pimdir SYNC §4), so only a store an earlier engine wrote holds such a row.

> Seed spec (Cairn, 2026-08-01): captures the offline mutation vocabulary, retro-documented when `Add` was added.

### Requirement: A provisional handle is the link id under U+0001
Every create the engine stages, an `Add`, a `Copy`, a `Move`, a vanished edit re-staged ([sync](sync.md)) or the copy the hub offers ([hub](hub.md)), SHALL sit under `PimdirLinkId::provisional`, `U+0001` followed by the link id (pimdir SYNC §3): no remote handle starts with a control character, so the namespace never collides with a server's, and one identity has one pending create per collection at a time. A minted key's provisional handle is `U+0001` then `dup:<hint>#<handle>`. The caller supplies no placeholder.

#### Scenario: Two clients stage the same copy
- GIVEN two processes copying one identity into one target through the queue
- WHEN both actions are applied
- THEN they land on one provisional handle, the second upsert restating the first, and one add is pushed

### Requirement: A staged create never takes a key its target holds
A `Copy` or a `Move` SHALL read the target collection for the identity it is carrying into it, and SHALL key the staged create under a minted key (upgrade.md) when a live placement there already holds that identity. The key is the identity minted over the provisional handle it derives, minted again while held, and the store pairs it with the identity for the create's origin and the tombstone's destination (pimdir SYNC §3). Refusing is not the answer, the way it is for an `Add`: the caller is asking for the copy, and a target holding the identity already is a target holding two resources once the create lands.

The read SHALL ask for the key a second copy would take beside the identity itself, and SHALL be made against the target rather than the collection the mutation reads, which cannot answer it. A source placement holding no identity yet stages its create without the read, having no key to collide with.

A `Copy` or a `Move` of a placement holding no body SHALL fail with `PimdirMutateError::Undeliverable` unless its binding is based and the run's source syncs the target (`PimdirMutate::syncs_target`, which `PimdirSourceStore::mutate` and the drain of a queued `move` or `copy` read from `collection_sources`, `false` by default): only that source holds the origin a bodiless create is copied from, and another source is offered the create once the body is held (pimdir SYNC §7; vectors/sync/135).

#### Scenario: A copy into a collection that holds the identity
- GIVEN a target collection holding a live placement under an identity
- WHEN that identity is copied into it
- THEN the staged create takes a minted key, and both rows keep their own body

#### Scenario: A tombstoned holder blocks nothing
- GIVEN a target whose only holder of the identity is a tombstone
- WHEN the identity is copied into it
- THEN the staged create takes the identity itself, a row on its way out holding no key against a create

### Requirement: An edit revives a tombstone, a flag change rides along
An `Edit` staging a body on a tombstoned placement SHALL revive it, leaving it `Dirty`, and SHALL drop the move destination the tombstone carried: new content beats a delete, the same rule the merge applies when the remote edits what was deleted locally, and the move that destination belonged to is not happening any more. Left on the revived row it turns the member's next plain delete into a relocation nobody asked for.

An `Edit` on a `Created` placement SHALL keep it `Created` and drop its origin: a server-side copy from the origin would deliver the body the edit replaced, so the create uploads the edited body instead (pimdir SYNC §7).

#### Scenario: An edit on a pending copy
- GIVEN a pending create staged by a copy, carrying its origin
- WHEN it is edited
- THEN it is still `Created`, points at the new body, and carries no origin

A `SetFlags` on a tombstoned placement SHALL leave it tombstoned, the marker riding along with the delete. A flag change is not content, so it settles nothing about whether the item is going.

The revival is what a hub-backed consumer resolves through: an item deleted on another source is projected here as a tombstone (hub.md), and the edit taken on it is both the resolution and the resurrection the hub reads from a live upsert.

#### Scenario: An edit brings a deleted row back
- GIVEN a tombstoned placement carrying the destination of a move
- WHEN it is edited
- THEN it reads `Dirty` and carries no destination

#### Scenario: A flag change leaves the delete standing
- GIVEN the same placement
- WHEN its flags are replaced
- THEN it is still a tombstone, with the marker and the destination it had

### Requirement: A mutation may restate the sort key
`Add` SHALL carry a sort key, and `Edit` SHALL carry an optional one on the same terms as its optional summary: absent keeps the stored key. An edit that changes what the key is derived from has to say so, or the item stays where it was in the list.

### Requirement: A resolution is measured against the remote it settled
The base an `Edit` resolving a conflict leaves SHALL be the remote state the resolution was merged against: `conflict_revision` as the base revision **and** `conflict_object` as the base object. A conflicted placement holding no base SHALL be given one from the same pair, its own resolution being where the two sides first agree.

Adopting the revision alone leaves the pair contradicting itself, the base claiming a revision its object was never the content of, and the sync's local-side signal is the object: a placement points at a body its base does not hold. A resolution keeping the ancestor of the divergence therefore read as nothing to push, while the adopted revision read as nothing to pull, so the decision never left the machine and the flag pass rebased the divergence away. Keeping the ancestor is the ordinary three-way merge answer, and the resolving tools offer it outright.

The four ways to resolve then fall out of the one comparison: keeping the local body, the ancestor, or a merge of the resolver's own pushes an `Update` gated on the recorded revision, and adopting the remote body wholesale owes no push and settles clean on the next run. The base is also the ancestor a later conflict is merged against, which is right for the same reason: after a resolution, the last state the two sides shared is the remote state the decision was taken against.

#### Scenario: Keeping the ancestor
- GIVEN a conflicted placement resolved with the body its base holds
- WHEN the collection is synced
- THEN an `Update` carrying that body is pushed, gated on the revision recorded at conflict time

#### Scenario: Taking the remote body
- GIVEN a conflicted placement resolved with the recorded diverging body
- WHEN the collection is synced
- THEN nothing is pushed and the placement lands clean

#### Scenario: A resolution with no base
- GIVEN a create-collision conflict, which has no base
- WHEN it is resolved with an edit
- THEN the placement is based on the recorded revision and body, and the next sync pushes the resolution instead of re-marking the conflict

Whether an edit is resolving SHALL be read from the divergence the placement carries (`conflict_revision`) rather than from its status, the two agreeing everywhere but on a tombstone: an item deleted on another source is projected as one while this source and its own server are still diverged, and the edit taken on it is the resolution. Reading the status leaves that base at a revision the server has moved past, so the push it derives is refused on every run, and the row is left carrying a divergence it claims to have settled.

#### Scenario: A resolution taken on a projected tombstone
- GIVEN a tombstone carrying a recorded revision and diverging body
- WHEN it is edited
- THEN the base adopts both, the divergence is cleared, and the next sync pushes against the revision it was measured on

### Requirement: A flag change states the set it read
`PimdirMutation::SetFlags` SHALL carry `base: Option<PimdirFlags>`, the set its caller read and changed into `flags` (a queued `set-flags`'s `base`, STORAGE §15.3). Carrying one, it SHALL apply the change alone to the set the placement holds when the mutation is staged (`PimdirFlags::changed`): that set, plus each flag `flags` holds and `base` lacks, less each flag `base` holds and `flags` lacks, every other flag as it stands, so a flag a sync landed or removed since is kept as the sync left it and the push the next sync derives names what the caller changed and nothing else (pimdir SYNC §7, vectors/sync/141). It SHALL be refused, staging nothing, as `PimdirMutateError::UnknownFlags` (code `flags-unknown`) over a placement whose flags are unknown: there is no set to change, and replacing it would write the unread flags as removed (vectors/sync/142). Carrying none, it replaces the set, the caller owning it whole.

#### Scenario: A flag landed since survives
- src/mutate.rs `a_flag_change_stating_its_base_keeps_what_landed_since`, `a_flag_change_stating_its_base_is_refused_over_an_unknown_set`; tests/queue.rs `a_queued_flag_change_stating_its_base_keeps_a_flag_landed_since`

### Requirement: A resolution names the conflicts it settles
`PimdirMutation::Edit` SHALL carry `resolves: Option<Vec<PimdirHash>>`, the diverging bodies a person's resolution was decided over: the `conflict_object` of each conflict it was shown (a queued `update`'s `resolves`, STORAGE §15.3), `None` for an ordinary edit. An `Edit` without `overrule` meets the item's conflict when its item is conflicted and its binding's when the placement carries a `conflict_revision`; meeting either, it SHALL be refused, staging nothing, as `PimdirMutateError::Unresolved { handle, met, resolves }` (code `conflict-unresolved`, `PimdirMutateError::code`) unless `resolves` names the `conflict_object` of each conflict it meets, and stating `resolves` and meeting none it SHALL be refused the same way, the conflict it was decided over being settled already. A conflict whose diverging body nobody has read is named by no resolution. An ordinary edit, made before the conflict arose or without it in view, so never discards a diverging body (pimdir SYNC §7). An overruling edit ignores `resolves`.

`PimdirMutation::Remove` SHALL carry `resolves` on the same terms (pimdir SYNC §7): meeting the item's conflict or its binding's it SHALL be refused, staging nothing, as `PimdirMutateError::Unresolved`, unless `resolves` names the `conflict_object` of each conflict it meets, and stating `resolves` and meeting none it SHALL be refused the same way. A remove staged before a conflict arose, or without it in view, so never deletes a diverging body unseen, and no `Remove` settles a conflict whose body nobody has read: a base never pairs the observed revision with the old body (vectors/sync/143, 144; src/mutate.rs `a_remove_settles_only_the_conflict_it_names`, `a_remove_resolving_no_conflict_is_refused`). A remove resolving the item's conflict stages `PimdirWriteOp::Settle` before its tombstone (`a_remove_naming_the_item_conflict_states_the_settle`).

### Requirement: A move meeting a conflict is refused
A `Move` meeting a conflict, the item's or that of the binding it runs through, SHALL be refused, staging nothing in either collection, as `PimdirMutateError::Unresolved { handle, met, resolves: None }` (`conflict-unresolved`), before it reads its target (pimdir SYNC §7, sync-draft-11). It states no `resolves`, so it is never a person's resolution: its tombstone would leave the conflict standing, the diverging body pinned under a placement no listing shows, and the create it stages would carry the item's body as though nothing diverged from it. The conflict is settled first, by an `Edit` or a `Remove` stating `resolves`, and the item moved after. It meets the conflicts an `Edit` meets, one a storage projects without stating its body included. A `Copy` meets none: it tombstones nothing, and the conflict stands on the placement it copies from. An `add` superseding a conflicted item tombstones it as a `remove` of it would (STORAGE §15.3), so it parks whole.

The drain stages a queued `move` through this mutation, so it parks naming the diverging body (STORAGE §15.2, §15.3), and a reader's overlay folds nothing for it, in the collection it leaves and in the one it names ([store](store.md), "A queued move meeting a conflict parks").

#### Scenario: The vectors, and the queue
- vectors/sync 155 (the binding's conflict) and 156 (the item's, through a binding that is not conflicted), each refused with the rows as they were, tests/vectors_sync.rs; src/mutate.rs `a_move_meeting_a_conflict_is_refused_and_a_copy_is_not`; tests/item_conflict_settle.rs `a_move_over_a_conflicted_item_is_refused_and_parks`, `an_add_superseding_a_conflicted_item_parks_whole`. Each fails with the refusal removed

The mutation reads the item's conflict from `PimdirLoaded::conflicted`, each loaded placement whose item is conflicted with the item's diverging body, which `PimdirStore`'s load answers from `PimdirHub::conflicted`. A storage stating no item conflict for a placement it projects `Conflict` with no binding conflict leaves the conflict unknown, read as one nobody has read: no edit settles it.

#### Scenario: The vectors
- GIVEN an item or a binding in conflict, or neither
- WHEN an `Edit` states no resolution over a conflict (124, 127), names another body (125), states one over no conflict (126), names an unread body (128) or one conflict of two (129)
- THEN it fails and the rows stay as they were; one naming each conflict it meets settles them (vectors/sync/28, 85, 86, 103, 104, 118 to 123; tests/vectors_sync.rs; src/mutate.rs `an_edit_settles_only_the_conflict_it_names`, `a_projected_conflict_the_storage_does_not_state_is_unread`)

### Requirement: A person's resolution settles the item's conflict
An `Edit` not stating `overrule` whose `resolves` names the item's diverging body SHALL settle its item's cross-source conflict whatever body it keeps and whichever source it is made through (pimdir SYNC §7): the item takes the edit's body, `items.conflicted` is cleared and its `conflict_object` released, the editing source's `shared_object` moves to that body, and every other source then projects `Dirty` where its base differs and pushes it. What it discards it loses: no retained item, no `Overruled`. The mutation stages `PimdirWriteOp::Settle { collection, handle }` before its upsert when the item it edits is conflicted (`PimdirLoaded::conflicted`), and the write settles it; an edit of an item in no conflict states none and folds by the three facts as before. The three facts alone read an edit restating the item's body as no change and one made through the source the item moved away from as a divergence again, which is why the edit states the settle.

#### Scenario: Every pick through either source settles
- GIVEN two sources diverged on one item under `manual`, the item holding P with S as its `conflict_object` and the source the item moved away from agreeing still on the ancestor
- WHEN a person's resolution, naming S, keeps P, takes S or writes a merge X, through either source
- THEN the item holds that body and no conflict, S's item reference is released, nothing is retained or reported, the editing source agrees on the body, and each source projects `Dirty` exactly where its base differs (tests/item_conflict_settle.rs, six cells; src/mutate.rs `a_persons_resolution_of_an_item_conflict_states_the_settle`)

### Requirement: An overruling edit keeps what it discards
An `Edit` stating `overrule`, a settle by a rule of the engine's or a connector's (a one-way mode restating the body its authority holds), SHALL keep each diverging body it does not adopt, the binding's `conflict_object` then the item's, each once, as a retained item under `dup:<link>#<edited handle>`, minted again over a held key, its summary derived from the body, reported `Overruled` on `PimdirMutateReport::events`, before any body the collection's policy keeps in the same write (pimdir SYNC §7, §9). The mutation stages `PimdirWriteOp::Overrule { collection, handle }` before its upsert, and the write reads the conflict there, the projection carrying no item conflict body. An overruling edit restating the shared body over the item's conflict clears it, the source's `shared_object` moving to that body. It SHALL be refused, staging nothing, as `PimdirError::Unread` (`conflict-unread`) while the binding's or the item's conflict holds no diverging body. Without `overrule` an edit is a person's choice, a resolution naming what it settles, and keeps nothing.

#### Scenario: The vectors
- vectors/sync 100 (a binding conflict), 101 (refused over an unread body), 102 (an item conflict), 103 (no overrule keeps nothing), tests/vectors_sync.rs; tests/overrule.rs `an_overruling_edit_over_an_unread_item_conflict_is_refused`

### Requirement: A delivered create is given up on request alone
`PimdirMutation::Abandon { handle }` SHALL give up a delivered create (`PimdirPlacement::delivered`: based under its provisional handle) of the run's source (pimdir SYNC §5, §7): it stages a `Deleted` drop of the provisional handle and nothing else, so the write marks the item deleted and, no other source binding it, retains it with its body, its summary and the flags staged on it, a `Remove` or a relocation held on it given up with the binding. It pushes nothing, and its report carries `PimdirSyncEvent::Abandoned(handle)`, the provisional handle. On any other placement, a pending create (which a `Remove` withdraws) or a member a listing named (which one removes), it SHALL fail as `PimdirMutateError::NotDelivered` (`create-not-delivered`), staging nothing. No sync, no round's absence and no count of runs gives a delivered create up, and no queue action names an `Abandon`: an owner stages it on a person's request, through `PimdirSourceStore::mutate`. Another source binding the item keeps its binding as it was and projects a `Tombstone` for it, as for any member gone from one source of several ([hub](hub.md)): the item is marked deleted and not retained, and that source's next sync pushes the delete (pimdir SYNC §7, vectors/sync/157).

An `Edit` or a `Remove` carrying `if_match` SHALL be refused (`Replaced`, `held` `None`) over a placement holding no body, as over one holding another (pimdir SYNC §7, vectors/sync/153, 154).

#### Scenario: The vectors, and a tombstoned one
- vectors/sync 150 (given up, retained, reported), 151 (refused on a pending create), 157 (given up beside another source, which owes the delete), 153, 154, tests/vectors_sync.rs; tests/store_text.rs `a_delivered_create_is_abandoned_on_request_alone` (a tombstoned delivered create goes with its held remove, which no vector states)
