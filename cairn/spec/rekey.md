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

The batch SHALL list every `Rekeyed` drop before every upsert (pimdir SYNC §8, §10): a new handle may be an old one another member held, and the drop is what licenses the store to move the binding. The hub keeps the dropped binding's base aside for the upsert that rebinds the source ([hub](hub.md)).

A listed flag set that is unknown SHALL leave the base's flags as they are, holding no opinion (pimdir SYNC §3, §8): a flag change still owed stays owed, and the next sync pushes it. Taking the unknown set as the base read the owed change as agreed and never pushed it (vectors/sync/201, R4 of pimdir cairn/changes/a-smaller-sync, a silent loss).

A pending create staged from this collection into another carries an origin naming a handle the rebuild voided. The store derives the origin anew on every load from the bindings by link id (SYNC §3), so such a create copies from the carried handle; a consumer keeping the staged origin sees it fall back to an upload when the store holds the body, and rejected until restaged otherwise.

### Requirement: A rekey never writes a base it never reconciled
A mutable member whose listed revision differs from the one its old base held changed on the remote while the handles did (pimdir SYNC §8). The rekey SHALL carry it as the pull a sync would make, at the fetched revision, the body the listing carries stored as the placement's and the base's at `Full`, or, carrying none, the body dropped, level `Meta`, base object `None` (vectors/sync/212); or as a `Conflict` at the fetched revision with the base untouched when the placement also holds a local edit, a body its base does not, the listed body its `conflict_object` when the listing carries one. A tombstone holding a local edit is that case too: carried as a `Conflict` at the fetched revision, never revived as a pull keeping the edit only in an item retained apart (vectors/sync/209, R3); a tombstone holding none is revived as the pull SYNC §5 makes. A tombstone whose binding is conflicted SHALL keep its conflict, a delete settling none (SYNC §3), listed at its base revision or at its conflict's, and derives no `Remove` (vectors/sync/207, 208; R2b, R2c). A `Conflict` the old handle held is carried as it is, revision and diverging body kept while the fetched revision is the one recorded or the listing states none, a listing stating no revision stating no move (vectors/sync/202; R1, a silent loss: the conflict was cleared and its body made collectable); an item-level conflict ([hub](hub.md)), which records no revision, SHALL gain none unless the remote moved, since a binding conflict pinned at a revision the server never left holds that source's pushes for ever while the other sources settle the item. A base claiming the fetched revision while holding the old body is the one thing the rekey MUST NOT write: the next sync would read the stale body as current, or push the local edit last-writer-wins. A local edit carried in conflict keeps the summary and sort key it states, the fetch's describing the server's body nobody adopted (pimdir SYNC §8, STORAGE §9.3; tests/engine/summary_kept.rs, failing before it).

#### Scenario: A remote edit over a clean placement
- GIVEN a placement clean at `r1` whose new handle the listing reports at `r2`
- WHEN the collection is rebuilt
- THEN it lands on its new handle with no body, at `Meta`, its base at `r2` with no body

#### Scenario: A remote edit over a local edit
- GIVEN a placement edited locally over a base at `r1` whose new handle the listing reports at `r2`
- WHEN the collection is rebuilt
- THEN it lands conflicted at `r2`, its base still `r1` and its old body, so nothing pushes last-writer-wins

### Requirement: A rebuild reads every page before its one batch
A rekey SHALL list a round over the whole collection whatever the scope, every page read before its one batch is written, a rejected cursor relisting it from the start, and name each member from its meta, fetching nothing (pimdir SYNC §8): the old handles being void, it is the one listing whose absence covers every member. The batch lands the last checkpoint a page carried, none keeping the old one, and leaves the coverage as it was.

A page neither `last` nor carrying a cursor SHALL complete the rekey with `PimdirArgError::UnresumablePage`, and a page that is not a round's (`complete` false) with `PimdirArgError::IncompleteListing`, writing nothing: the rebuild's absence covers every member, so only a whole round may answer it.

#### Scenario: A delta answering a rekey
- GIVEN an old spine of two placements
- WHEN the rekey's round request is answered by a delta listing one member
- THEN it completes with `IncompleteListing` and writes nothing (src/rekey.rs `a_delta_answering_a_rekey_is_refused`; a page neither last nor cursored, `a_page_neither_last_nor_cursored_is_refused`)

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
A rebuild SHALL walk the new members in handle order and give each the identity its meta names while the rebuild has not handed that identity out; a member resolving to one already taken SHALL be keyed under the minted key an old copy of that hint carries, and failing that under a mint of its own handle.

Carrying the old minted key is what the mint's own determinism rests on. The key is derived from the hint and the handle it was minted from, and a handle-space change is exactly what takes that handle away, so a rebuild carries the key rather than re-deriving it. What the source reports for both copies is the hint they share, the minted key being the replica's own and never a thing a listing or a fetch returns.

Merging the two instead keeps one body, one summary and one set of pending edits for two resources the source holds, and loses the other at the write that noticed the problem.

A member resolving to no old binding SHALL be named as a page names one (pimdir SYNC §6, §8): it lands this source's pending create whose key it states and whose summary it agrees with, the provisional handle dropped `Superseded` and `Created` reported (vectors/sync/205), joins a create the store offers this source (`PimdirLoaded::offered`), an item it does not bind yet, reported `Added` (vectors/sync/204), and is minted apart and reported `Disagreed` when its summary disagrees with the pending create's (vectors/sync/206). A pending create's key counts as taken only while no member lands it: counting it taken whatever the member minted the arrival apart and pushed the add again, a second message on the server (rule D10 of cairn/changes/what-the-consumers-need, neverest NV1).

Where one hint had several old copies, a member SHALL be matched first to the old copy whose base names its fetched revision, then to the one holding its body when the fetch carried one, and in handle order only among what neither tells apart (pimdir SYNC §8), so a renumbering that swapped two resources under one `UID` carries each one's flags and pending edit onto itself.

#### Scenario: Two copies of one hint are carried apart
- GIVEN a collection holding one hint twice, the second copy under a minted key
- WHEN the handle space is rebuilt and the source reports the shared hint for both members
- THEN the first member takes the hint, the second is carried onto the key it was minted under, and each keeps its own body

#### Scenario: A new copy is minted from its own handle
- GIVEN a rebuild whose new space holds two members of one hint and only one old row to carry
- WHEN the spine is rebuilt
- THEN the second member takes a key minted from its own handle

#### Scenario: A pending create lands on its member
- GIVEN a collection holding a pending create under an identity
- WHEN a rebuilt member states that identity and agrees with the create's summary
- THEN the create lands on the member, one item, reported `Created`, and no second add is pushed (vectors/sync/205)

#### Scenario: Two copies at one revision
- tests/engine/sync_audit.rs `a_rekey_matches_copies_of_one_hint_by_their_body` (failed before the change, which matched no body: the rebuild carried one copy onto the other's handle and the store refused it)

### Requirement: A rebuild is the only bump of the handle-space epoch
The consumer SHALL commit the rebuild's write batch and the collection's epoch bump in one transaction (pimdir STORAGE §12: `collections.generation`), so a reader deriving an epoch-dependent protocol value from the store never sees a rebuilt spine under the old epoch. Ordinary syncs, full resyncs from an expired checkpoint and content changes SHALL NOT bump it.

### Requirement: A fetch refreshes the key at every tier
An upgrade SHALL adopt the key from the fetched item at both tiers, unlike the link id, which is kept once resolved. The key is a projection of content rather than an identity, so the later and better-informed derivation wins: a full body carries the real date where an envelope may have carried none.

### Requirement: A key survives a rekey
Rebuilding a collection onto a new handle space SHALL carry each placement's key over, preferring the one the rekey's `Meta` fetch resolved and falling back to the key the old placement held, so a handle-space change does not un-sort a collection.


### Requirement: The store bumps the generation on a rekeyed batch
The engine emits no op for the epoch bump: a batch carrying a `Rekeyed` drop is a rebuild, and the store bumps the collection's generation in the transaction applying it (pimdir STORAGE §12, SYNC §8).

### Requirement: A rekey missing most members, or listing none, parks
A rekey whose listing leaves more than half of the source's based bindings in the collection with no member, those bindings numbering at least 20, SHALL write nothing (no drop, no upsert, the checkpoint and the generation unmoved) unless `PimdirRekey::accept_absences` is set (`PimdirSourceStore::rekey(collection, accept_absences, remote)`), and `PimdirRekeyReport::parked` carries the counts (pimdir SYNC §8, vectors/sync/89). A rekey whose listing names no member SHALL park the same way while any based binding of the source is absent, whatever their number (src/rekey.rs `an_empty_listing_parks_the_rekey`). The old checkpoint still names the old handle space, so the next run names the rekey again.

### Requirement: A rekey from a capped listing parks on any absence
A rekey whose listing the source caps, a page of it stating `PimdirRemoteSnapshot::capped` ([seam](seam.md); pimdir SYNC §4), SHALL write nothing (no drop, no upsert, the checkpoint and the generation unmoved) while any based binding of the source in the collection resolves to no member, whatever their number, unless `PimdirRekey::accept_absences` is set, and `PimdirRekeyReport::parked` carries the counts (pimdir SYNC §8, vectors/sync/159). A capped listing leaves out members the source still holds, so such a binding is no absence the listing proves, and its old handle names nothing in the new space: it can be neither dropped as gone nor carried as it is. Accepted, the rekey is written as any other, each such binding dropped `Deleted`, or re-staged where it holds an edit. A capped rekey every based binding resolves in is written.

The bindings counted absent there are every based one no member resolved to, a delivered create aside: those the rebuild would drop `Deleted` and those holding an edit it would re-stage as a pending create, which the next sync would add beside the member the cap left out. The guard of a rekey no cap bounds counts the `Deleted` drops alone, as it did.

One page stating the cap is enough, the last or not: a cap is the collection's, which a connector states on every page of its rounds. A listing restarted over a rejected cursor is read anew.

#### Scenario: One absent of three under a cap
- GIVEN three based bindings and a capped listing naming two of them under new handles
- WHEN the collection is rebuilt
- THEN nothing is written and the run reports 1 absent of 3 bound (src/rekey.rs `a_capped_rekey_parks_on_one_absent_member`, sync vector 159, each failing with the cap unread); under `accept_absences` the two are dropped `Rekeyed` and the third `Deleted` (`a_capped_rekey_whose_absences_are_accepted_is_written`); the listing naming all three is written (`a_capped_rekey_every_binding_resolves_in_is_written`); with no cap one absent of three is written (`an_uncapped_rekey_missing_one_member_is_written`)

#### Scenario: An edit the cap left out
- src/rekey.rs `a_capped_rekey_parks_on_an_edit_no_member_resolves_to` (fails with the `Deleted` drops alone counted: the edit is re-staged and the rekey written); `a_cap_stated_on_any_page_caps_the_rekey` (fails with the last page alone read)

### Requirement: A re-staged edit takes a provisional handle
A rebuild re-staging an edited row no member carries (SYNC §8) SHALL key it under the provisional handle its link id derives, its binding moved there by a `Superseded` drop of the old handle, which is void and may be one the new space gives another member.

#### Scenario: A renumbering that gave the edited member's handle away
- tests/engine/sync_audit.rs `a_rekey_restages_an_uncarried_edit_under_a_provisional_handle` (failed before the change: the batch bound two items to one handle and the store refused it)

### Requirement: Every old handle is dropped before any upsert
A rebuild SHALL drop every old handle but one carried onto itself, every drop before every upsert (SYNC §8): a handle the new space reuses for another member is still dropped, `Rekeyed` when its member was carried elsewhere and `Deleted` when it resolved to none, and the mass-absence guard counts the latter.

#### Scenario: A server renumbering from 1
- tests/engine/sync_audit.rs `a_rekey_reusing_a_gone_members_handle_drops_it` (the store already retired the old binding), `a_rekey_counts_a_gone_member_whose_handle_is_reused_as_absent` (failed before the change: 24 of 25 absent counted)

### Requirement: A rebuild knows every old copy of an identity by its key
A rebuild SHALL find the old copies of a member's identity by the key each carries, never by the handle it holds now: the placement keyed by the hint, and each keyed by a key minted from it over whatever handle (`PimdirLinkId::minted_from`), a rekey carrying the key and not the handle. A minted key whose `#` could end several keys goes to the longest one the listing names. Among the copies the member takes the one its revision or body tells, else the bare key while an old row holds it or no copy is left, else the first remaining copy in handle order, so a copy whose bare sibling is gone is carried too, and only then a mint of its own. A second rekey therefore carries a copy the first rekey carried, a copy staged beside the identity and delivered, and a lone minted copy, with their bodies and pending state, and no other source of the collection is told to remove one.

#### Scenario: The copies a first rekey or a copy left
- GIVEN one mailbox holding a `Message-ID` twice, hydrated, rekeyed once, a flag then staged on the minted copy; a `Copy` staged beside the identity and delivered; a minted copy whose sibling was deleted
- WHEN the server renumbers and the collection is rekeyed
- THEN every copy is carried (`rekeyed` counts it, `pulled` and `dropped` do not), with its body and staged flag, nothing retained, and a second source mirroring the mailbox keeps both messages on its server (tests/engine/rekey_copies.rs)

### Requirement: A rebuild grows with the collection
A rebuild SHALL index the old copies by identity once (`PimdirRekey::copies`) rather than scan every old placement per listed member, and the hub's absorb SHALL find the binding a drop or a delivery names through an index of the batch's handles, checked to bind it still, rather than walk every item per drop; so a rekey of a whole collection, every handle dropped then upserted, costs in the engine and in the store's write what the collection's size does, not its square.

#### Scenario: Measured
- GIVEN mailboxes of 2,000 to 16,000 distinct messages, renumbered
- WHEN each is rekeyed
- THEN the coroutine's and the write's times grow about as the collection does (tests/engine/rekey_scale.rs, ignored, run with `--release --ignored`)

### Requirement: A rekey reports the members it names
A member no old placement resolves to by link id SHALL be named in the rekey's batch as a page names one and reported `PimdirSyncEvent::Added` on `PimdirRekeyReport::events` once the batch committed (pimdir SYNC §5, §8); a parked rekey reports none. A create the rekey lands is reported `Created`, a member minted apart from a pending create `Disagreed`, and each body its write kept apart (`PimdirWritten::overruled`) `Overruled`, as a sync reports them (vectors/sync/203, 205, 206).

### Requirement: A repeated handle keeps its first entry
A rekey listing a handle twice SHALL read only its first entry, its flags, link id, summary, sort key and revision alike, as a page does (pimdir SYNC §4). Mixing the two carried the first entry's flags under the last's identity and dropped the carried item `Deleted` (vectors/sync/210, R0).

### Requirement: A delivered create is landed as a page lands it
A delivered create a member resolves to SHALL be landed as a page lands it (pimdir SYNC §6, §8), through the one landing a sync uses (`PimdirSync::land`): a `Superseded` drop of its provisional handle, its staged flags kept, its base the listed flags with the staged body, and `Created` reported (vectors/sync/203, R5).

### Requirement: A listed body is stored as a page stores it
A body the listing carries, inline or persisted, SHALL be stored (`StoreObject`) in the rekey's batch: an arrival's becomes its body and its base's at `Full` (vectors/sync/211), a pull's as above (212), a conflict's as its diverging body (209), a landed create's as a page lands it (R8).

### Requirement: A base-less binding is carried as it is
A conflicted binding with no base under a handle the renumbering voids SHALL be carried onto the member resolving to it with its conflict and a base of the listed flags naming no revision and no body, which claims nothing reconciled; when no member resolves to it, it SHALL be re-staged as a pending create under its provisional handle, as a sync re-stages an edit whose member went (pimdir SYNC §5, §8; rule D1). No vector states that starting state (a binding has a base unless provisional): tests/engine/rekey_cases.rs pins both.

#### Scenario: A message arriving during a renumbering
- **GIVEN** a source renumbering `10` into `20` while a new message arrives as `21`
- **WHEN** the rekey runs
- **THEN** `21` is named under its own key and reported `Added` (vectors/sync/190)
