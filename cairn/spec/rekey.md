---
cairn: spec
capability: rekey
status: current
---

# Rekey

`PimdirRekey` is the I/O-free coroutine that rebuilds one collection onto a new handle space. A source may renumber every member without any of them changing (an IMAP `UIDVALIDITY` bump, a provider migration, a mailbox restored from backup), and every handle the replica holds is void at once. The spine is re-enumerated and the stored state is carried onto it **by link id**, which is the only identifier that survived, so bodies, summaries, bases and pending local edits are kept rather than re-fetched.

It is a distinct verb rather than a resync because a resync cannot tell the two apart: reading the new handles as unknown members and the old ones as vanished would delete the collection and download it again, losing every staged edit on the way.

### Requirement: A rebuild carries state over by link id
A rekey SHALL match each member of the new handle space to the placement holding the same link id, and carry that placement's body, summary, level, base, flags and pending local state onto the new handle. A member the old space does not account for is an ordinary new placement; a placement the new space does not account for is gone from the source.

Identity is the only thing a handle-space change leaves intact, so it is the only thing the match may key on. Matching on anything derived from a handle would match nothing, and matching on content would pair two copies of one body.

### Requirement: A rebuild's drops say the row is superseded
Every drop a rebuild emits for a placement its own batch re-writes SHALL carry `PimdirDropReason::Rekeyed`, never `Deleted`. The item is not going anywhere: the same batch upserts it under its new handle, and a storage sharing one item across sources reads a `Deleted` drop as the item being gone and propagates a removal to sources nobody touched.

The reason is also what licenses the rebind. A storage pins one handle per binding and refuses to repoint it, because a repoint is how a second copy of one identity is swallowed; a rebuild is the one case where the repoint is correct, and the superseded handle is what tells the two apart. The licence is per handle: a rebuild batch that also carries a genuine duplicate SHALL still refuse that one, which keeps a renumbering from re-keying the copies onto each other.

A minted key (upgrade.md) is carried like any other: the rebuild matches on it, so renumbering two copies of one hint does not merge them, they having never been one item.

A drop for a row the rebuild carried nothing for SHALL carry `PimdirDropReason::Deleted` instead. A rebuild enumerates the whole handle space, so a row no member of the new space accounts for is a row the source no longer holds, which is the fact a complete enumeration states in an ordinary sync. Reading it as housekeeping keeps the deletion on the source it happened on: a hub keeps the item alive, no other source hears about it, and it is mirrored back to the source it was deleted on.

#### Scenario: A renumbered collection is not a duplicated one
- GIVEN a placement bound under a handle the rebuild supersedes
- WHEN the batch drops that handle and upserts the item under a new one
- THEN the binding follows the new handle, reads clean, and takes no repoint it was not licensed for

#### Scenario: A rebuild that also lost a member
- GIVEN a collection whose source expunged one member and then renumbered every handle
- WHEN the collection is rebuilt
- THEN the renumbered row is dropped as `Rekeyed` and the expunged one as `Deleted`

#### Scenario: A deletion during a bump reaches the other sources
- GIVEN an item bound to two sources
- WHEN one source expunges it and rebuilds its handle space
- THEN the item is deleted across the hub rather than mirrored back to the source it left

### Requirement: A rebuild keys two copies of one hint apart
A rebuild SHALL walk the new members in handle order and give each the identity its `Meta` fetch resolved while the rebuild has not handed that identity out; a member resolving to one already taken SHALL be keyed under the minted key an old copy of that hint carries, and failing that under a mint of its own handle.

Carrying the old minted key is what the mint's own determinism rests on. The key is derived from the hint and the handle it was minted from, and a handle-space change is exactly what takes that handle away, so a rebuild carries the key rather than re-deriving it. What the source reports for both copies is the hint they share, the minted key being the replica's own and never a thing a fetch returns.

Merging the two instead keeps one body, one summary and one set of pending edits for two resources the source holds, and loses the other at the write that noticed the problem.

The keys of the pending creates a rebuild leaves untouched SHALL count as taken, those rows staying in the collection while the rebuild runs.

#### Scenario: Two copies of one hint are carried apart
- GIVEN a collection holding one hint twice, the second copy under a minted key
- WHEN the handle space is rebuilt and the source reports the shared hint for both members
- THEN the first member takes the hint, the second is carried onto the key it was minted under, and each keeps its own body

#### Scenario: A new copy is minted from its own handle
- GIVEN a rebuild whose new space holds two members of one hint and only one old row to carry
- WHEN the spine is rebuilt
- THEN the second member takes a key minted from its own handle

#### Scenario: A pending create keeps its key
- GIVEN a collection holding a pending create under an identity
- WHEN a rebuilt member resolves to that identity
- THEN the member takes a minted key, the create keeping the one a push is about to land

### Requirement: A rebuild is the only bump of the handle-space epoch
The consumer SHALL commit the rebuild's write batch and the collection's epoch bump in one transaction (pimdir STORAGE §12: `collections.generation`), so a reader deriving an epoch-dependent protocol value from the store never sees a rebuilt spine under the old epoch. Ordinary syncs, full resyncs from an expired checkpoint and content changes SHALL NOT bump it.

### Requirement: A fetch refreshes the key at every tier
An upgrade SHALL adopt the key from the fetched item at both tiers, unlike the link id, which is kept once resolved. The key is a projection of content rather than an identity, so the later and better-informed derivation wins: a full body carries the real date where an envelope may have carried none.

### Requirement: A key survives a rekey
Rebuilding a collection onto a new handle space SHALL carry each placement's key over, preferring the one the rekey's `Meta` fetch resolved and falling back to the key the old placement held, so a handle-space change does not un-sort a collection.


### Requirement: The store bumps the generation on a rekeyed batch
The engine emits no op for the epoch bump: a batch carrying a `Rekeyed` drop is a rebuild, and the store bumps the collection's generation in the transaction applying it (pimdir STORAGE §12, SYNC §8).
