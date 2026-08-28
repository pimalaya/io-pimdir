---
cairn: change
change: duplicate-link-id-mints-an-item
---

# Delta

## ADDED Requirements

### Requirement: A binding never changes handle, and nothing is recorded in its place
A write that resolves an existing `(collection, link_id, source)` to a different handle SHALL be refused with a typed error, leaving the bound handle and every other column untouched. The store SHALL record no trace of the incoming handle.

A binding pins one handle, so a write carrying a second one is either a rebuild (licensed per handle by the drop reason, below) or a source holding two resources whose identities resolved to one key. The second is now the engine's to resolve before it writes, by minting a key for the second copy, so the store's whole obligation is to refuse the collision rather than to describe it. Applying it would repoint the binding from the copy it held to another, which is where the evidence used to die: silently, at the write, before any later rule could act on it.

#### Scenario: A colliding write is refused
- GIVEN a stored binding for an identity
- WHEN a write carries the same `(collection, link_id, source)` under a different handle
- THEN the write fails, the bound handle is unchanged, and no ambiguity is stored

### Requirement: A minted link id is an ordinary key
The store SHALL persist whatever `link_id` the engine assigns, SHALL NOT parse it, and SHALL NOT re-canonicalise one. A minted key (pimdir SPEC §9, `dup:<hint>#<handle>`) is subject to every rule a bare key is: `seq` allocation, retention and revival, dedup by object hash, the reader's pages, and the queue.

The key is the store's, the hint is the format's, and the two are only equal by default. A store that treated a prefix as meaning something would make the engine's assignment reversible by accident and would change a `seq` a consumer has already shown.

#### Scenario: Two resources under one hint are two items
- GIVEN a collection holding an item keyed by a bare hint
- WHEN a write upserts a second item keyed `dup:<hint>#<handle>` for the same source under another handle
- THEN both items exist with their own `seq`, their own binding and their own object reference

## MODIFIED Requirements

### Requirement: A superseded handle licenses its own rebind
A write batch dropping a handle with `ReplicaDropReason::Superseded` SHALL let the binding holding it move to whatever handle the same batch upserts for that identity, by deleting the binding and inserting the new one (SPEC §10, §12). A batch dropping a handle for any other reason, or not dropping it at all, SHALL keep the bound handle and **refuse the write**.

The two are indistinguishable from the rows: a rebuilt spine and a source reporting one identity under a second handle produce the same before and after, and only the drop's reason separates them. Reading a rebuild as a collision refuses every write of a renumbered collection, under handles the server has just voided.

The licence SHALL be per handle rather than per batch, so a rebuild carrying a genuine second copy of an identity still refuses that one and the floor stays intact inside the one operation that legitimately repoints.

#### Scenario: A renumbered collection is not a duplicated one
- GIVEN a binding on a handle a rekey batch supersedes
- WHEN the batch drops it and upserts the item under a new handle
- THEN the binding follows the new handle and projects `Clean`

## REMOVED Requirements

### Requirement: An identity a source holds twice is recorded, never repointed
**Reason**: the second copy now has an item of its own (pimdir SPEC §9), so there is nothing left to record on the binding of the first. `bindings.ambiguous_handles` and the frozen projection it fed are removed with it. The half of the requirement that still holds, that no write repoints a binding, is restated above as a refusal.
