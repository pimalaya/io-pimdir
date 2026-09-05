---
cairn: spec
capability: mutate
status: current
---

# Mutate

`PimdirMutate` is the I/O-free coroutine that applies a local edit to one collection offline, with no network. It loads the target placement, stages the resulting `PimdirWriteOp`s, and lets the driver write them; the remote is reconciled by the next `sync`. This is the write half of the "generic in the data, disciplined in the writes" rule: a client never writes storage rows directly; it stages a `PimdirMutation`, so the sync always knows what changed.

### Requirement: The offline mutation vocabulary
`PimdirMutation` SHALL stage a local edit to one collection offline, reconciled on the next sync:

- `SetFlags`: replace a placement's flags and mark it dirty (a pending create stays `Created`, an unresolved conflict stays `Conflict`, a tombstone stays a tombstone; the flag change rides along).
- `Remove`: tombstone a placement, kept until synced. Absorbed as a staged delete (the item is marked deleted, its binding kept), so the next sync pushes the remove.
- `Edit`: store a new body and repoint the placement at it (full level), keeping the base so the next sync derives the push. An edit whose object is the one the base already holds stages nothing and SHALL leave the status where it found it, `PimdirPlacement::staged_edit` being the single reading of "there is a local content edit here"; every other edit marks the placement dirty. Editing a conflicted placement resolves it whatever body it carries, the base adopting the remote state observed at conflict time, both halves of it (see below), and editing a tombstoned one revives it (see below).
- `Copy`: stage a `Created` placement in a target under a caller-supplied `placeholder`, carrying the source origin; the source is untouched.
- `Move`: stage a `Created` placement in the target under a caller-supplied `placeholder` (carrying the source origin), **and** tombstone the source. A move is thus a copy into the target plus a remove from the source, both derived on the next sync; the source's tombstone and the target's create land in their respective collection hubs. The destination the tombstone carries is what the store derives from the target's pending create on every load (pimdir SYNC §3); the one staged here serves a consumer keeping the placement as written.
- `Add`: see below.

A mutation SHALL touch the local replica only; the remote is reconciled by sync.

### Requirement: Add stages a locally-authored create
`PimdirMutation::Add { handle, link_id, flags, object, body, summary, sort_key }` SHALL stage a brand-new item with no remote origin (compose, import): a `PimdirStatus::Created` placement in the coroutine's collection under the provisional `handle`, at `level = Full`, with `base = None` and `origin = None`, pointing at `object`; plus a `StoreObject` carrying `body`. Because the create has no origin, the next sync SHALL push it as `PimdirChange::Add { origin: None }`, an append that uploads the body rather than a server-side copy. `Add` SHALL NOT require an existing source placement, and SHALL fail (`PimdirMutateError::LinkExists`) rather than overwrite when a live (non-tombstone) placement already holds `link_id`; a tombstoned `link_id` does not block the create.

A mutation naming a probe, a placement with no link id, SHALL fail with `PimdirMutateError::Probed`: the store holds a probe as flags only, so a status staged on it would be lost. A `Meta` upgrade names it first.

> Seed spec (Cairn, 2026-08-01): captures the offline mutation vocabulary, retro-documented when `Add` was added.

### Requirement: A staged create never takes a key its target holds
A `Copy` or a `Move` SHALL read the target collection for the identity it is carrying into it, and SHALL key the staged create under a minted key (upgrade.md) when a live placement there already holds that identity. Refusing is not the answer, the way it is for an `Add`: the caller is asking for the copy, and a target holding the identity already is a target holding two resources once the create lands.

The read SHALL ask for the key a second copy would take beside the identity itself, and SHALL be made against the target rather than the collection the mutation reads, which cannot answer it. A source placement holding no identity yet stages its create without the read, having no key to collide with.

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
