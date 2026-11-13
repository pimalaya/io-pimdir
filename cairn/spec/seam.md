---
cairn: spec
capability: seam
status: current
---

# Seam

The storage seam is the local index plus blob store the engine reconciles against. The consumer implements it (a sqlite index plus a blob dir in the reference driver); the engine reads it through `load` and `lookup_objects` and mutates it through a `write` batch of [`PimdirWriteOp`]s. The engine performs no I/O of its own: storage effects travel as coroutine yields.

### Requirement: DropPlacement is the retention decision point
The engine SHALL signal every removal, a local delete confirmed by the remote or a member vanished upstream, as a `PimdirWriteOp::DropPlacement`, and SHALL make no other assumption about what the storage does with it. A storage MAY hard-delete the row, or soft-delete it (retain the row, marked removed-upstream) for a backup that must never lose a copy the source expunged. No merge-core option governs this; retention is the storage's choice.

### Requirement: Hiding rows from load is safe
The merge reconciles only the placements a `load` returns. A storage that hides soft-deleted rows from `load` SHALL therefore not cause the engine to re-derive against them: the hidden row is invisible on every later sync, delta or full, so it is neither re-added nor looped over, and the retained copy survives.

#### Scenario: A backup keeps a remote expunge
- GIVEN a storage that soft-deletes on `DropPlacement` and hides such rows from `load`
- AND a member the remote expunges after a first sync
- WHEN the collection is synced again, whether delta or full
- THEN the row is retained but absent from `load`, and no re-derivation occurs

### Requirement: Object write carries bytes or a reference
`PimdirWriteOp::StoreObject` SHALL carry either the object's bytes or a reference to an object the consumer has already persisted to the blob store. The engine SHALL index the object from its `(hash, size)` in both cases, and SHALL have the storage write bytes only when they are present. Refcounting, dedup and garbage collection are unaffected.

### Requirement: A Full fetch may persist its own body
A consumer servicing a `PimdirTier::Full` fetch MAY stream the body straight into the blob store and report the object by `(hash, size)` with no inline bytes, so no full body is held in memory. The engine SHALL treat such a report as an already-stored object and emit a byteless `StoreObject`. `lookup_objects`, which lets the engine skip a Full fetch whose object already exists, is unchanged, so a persist-during-fetch stays idempotent under retry.

#### Scenario: A large body transfers without full materialisation
- GIVEN a source whose Full fetch streams the body into the blob store and reports it by (hash, size)
- WHEN the item is hydrated and later appended to another source
- THEN the engine holds no full body, indexes the object from its (hash, size), and the append streams from the stored blob

### Requirement: A placement carries a presentation sort key
`PimdirPlacement` and `PimdirFetchedItem` SHALL carry a sort key: the item's position in its collection's natural order, derived by the consumer wherever the summary is derived. The engine SHALL treat it exactly as it treats the summary, as an opaque value it ferries and never parses.

Empty SHALL mean unknown, and SHALL be the default, so an item is orderable from the moment it exists and no consumer has to invent a value. It is a plain value rather than an option because that is what the reference storage records, and one representation of "not known yet" is less to get wrong than two.

### Requirement: An unread flag set is unknown rather than empty
`PimdirFlags` SHALL carry an `Unknown` state distinct from a known-empty set, since the reference storage records the two apart (pimdir STORAGE §13: a `NULL` flags column means never read, `'[]'` means known to carry none). Known-empty SHALL remain the default, so unknown is stated by a source that read no markers rather than fallen back to by an ordinary write.

A source that read no markers reports an unknown set too, a listing naming members whose flags it did not read among them. The engine SHALL resolve the state on the first side that carries a set.

### Requirement: An unknown side holds no opinion in the merge
The flag merge SHALL treat an unknown side as neither an addition nor a removal: the result is the other side's set, two unknown sides stay unknown, and an unknown base is the same fact as no base on the flag axis, so nothing is derived from it and both sides' markers are kept.

Reading unknown as empty would make it an opinion, since element-wise an empty set says every flag the other side holds was removed here.

#### Scenario: A placement read without its markers learns them
- GIVEN a local placement whose flag set is unknown
- AND a source reporting that item with a marker set
- WHEN the collection is synced
- THEN the placement adopts the reported set and nothing is pushed

### Requirement: An unknown set never erases a known one
Absorbing an upsert whose flag set is unknown SHALL leave the shared set alone, on the same terms as an absent summary and an unknown sort key. A known set, empty or not, SHALL replace another known set: only unknown is inert.

### Requirement: A drop says whether the item is gone
`PimdirWriteOp::DropPlacement` SHALL carry a `PimdirDropReason`: `Deleted` when the item itself is gone (a local delete the remote confirmed, a member that vanished upstream), `Superseded` when only this row is gone, replaced by the server-assigned handle an accepted add reconciled a provisional handle to, or by the provisional handle a vanished edit is re-staged under ([sync](sync.md)), and `Rekeyed` when a rebuild renumbered it onto a new handle space (pimdir SYNC §10). Both license the binding to move; only `Rekeyed` is the store's signal to bump the collection's generation.

This refines the retention decision point above rather than replacing it: a storage still chooses what a removal means for its rows, but a storage that shares one item across sources SHALL propagate a delete only for `Deleted`. Reading a superseded row as a delete turns housekeeping into data loss on every other source, and it is what made the drop-then-upsert order of a rebuilt spine load-bearing.

#### Scenario: A rebuilt spine is not a mass delete
- GIVEN an item bound to two sources
- WHEN one source's row is dropped as `Superseded`
- THEN the item is not marked deleted and the other source projects it unchanged

### Requirement: A load states what it needs
`PimdirYield::WantsLoad` SHALL carry a `PimdirLoadSelection` in its `selection` (STORAGE §2 names `All`, `Handles` and `Links` a selection, a scope being the date window): `All`, `Handles`, `Links`, `Owing`, or `LastPage`. A mutation asks for the one placement it edits, or, for an `Add`, every row holding the link id it must not collide with; an upgrade asks for the handles it raises; a delta asks for `Owing` of the handles it lists and finds vanished, the store answering those and the placements `list_owing_bindings` and `list_lacking_items` name (pimdir SYNC §10), then for the `Links` of the hints it carries; a rebuild asks for the whole collection, and a round's last page for the whole collection with what the page states of its listing (`LastPage { narrowed, enumerated }`), the store answering the bindings the round finds absent from `list_unstamped_bindings` bound with them, because what it finds absent and whether it parks are counted over every based binding of its scope (pimdir SYNC §5), a page before the last asks for the `Handles` it lists and the `Links` of the hints it carries, and a sync first asks for `Handles` naming none, its sync state alone (checkpoint, coverage, round).

A load names the collection it reads, which is the coroutine's own except where a verb acts across two: a `Copy` or a `Move` reads its target for the identity it is carrying into it (mutate.md), that being the one question the collection it edits cannot answer.

The selection is a floor, not a ceiling: a storage SHALL return at least the placements it names and MAY return more, so a storage that ignores it stays correct, just as expensive as before. Under-delivering is not correct: a mutation that cannot see a colliding link id creates a duplicate. A `Handles` selection naming a provisional handle no binding of the source holds names the copy the hub offers under it, so a mutation can withdraw it.

#### Scenario: A one-row edit reads one row
- GIVEN a storage that honours the scope exactly
- WHEN any mutation other than `Add` runs
- THEN it is served only the placement it names, and still produces the same writes

#### Scenario: A delta loads what owes a push
- src/sync.rs `a_delta_loads_what_owes_a_push_never_the_collection`: a delta listing one member and dropping another asks for `Owing` of both and for its hints' `Links`, never `All`
- tests/owing_load.rs `a_delta_loads_what_it_names_and_what_owes_a_push`: over 50 clean placements, one off its base by a bit and one by a row, `Owing` of one clean handle answers those three

### Requirement: A write batch is applied in order
The store's write SHALL apply the ops in the order they are listed, and atomically. Ordering is what a batch naming one handle twice rests on, which the engine does emit: a sync that resurrects a locally deleted item writes the provisional row, pushes it, and supersedes the same handle once the remote assigns one, and the drop is the answer.

A storage MAY NOT group the batch by op kind, or otherwise reorder it: what looks like an independent set of rows is not one.

A rebuild relies on it outright: every `Rekeyed` drop precedes every upsert ([rekey](rekey.md)), the drop being what licenses a handle the new space reuses, and a store applying the batch out of order refuses the rebind.

#### Scenario: A rebuilt spine reuses a handle
- GIVEN a placement the old handle space held under a handle the new one reuses
- WHEN the collection is rebuilt
- THEN the batch drops that handle `Rekeyed` first and writes it once after, the store bumping the generation

### Requirement: A listed member carries its meta
`PimdirRemoteItem` SHALL carry a `PimdirRemoteMeta`: the identity hint (never a minted key), the summary and addresses Annex A derives, the sort key, and the body when the listing read it (pimdir SYNC §4). A connector reads it in the listing itself: an IMAP `ENVELOPE` with the header fields Annex A needs, `Content-Type` among them, or `summary::mail::derive_meta` over the header block; a Graph `$select` or a JMAP `Email/get` building `PimdirMailSummary` with the source's own attachment flag; a DAV `multiget` carrying the body. A page is a `PimdirRemoteSnapshot` (`complete`, `last`, `cursor`, an optional `checkpoint`, `narrowed` and, on a narrowed round's last page, `enumerated`), answered as `PimdirEnumerated::Page`, or `CursorRejected` when the source refuses a resumed round's cursor. A head (`PimdirListing::Head`) is answered as a page, `PimdirRemoteSnapshot::delta` for a delta head and `band_head` for a band head, or as `HeadRejected` or `NoHead`.

### Requirement: Nothing reaches the store unnamed
A write SHALL refuse an upsert carrying no link id for a handle no binding holds (`PimdirError::Unnamed`, pimdir SYNC §10); one for a bound handle folds into its binding's item as before. The store holds no probe rows.

### Requirement: A stated key is named on a server without its escape
`PimdirLinkId::stated` SHALL answer the identity a stated key names on a server, the key with its one leading `esc:` stripped (pimdir STORAGE §9): what a connector compares with or hands its server for the `link_id` of an `Add` or a `Remove`. The engine names no identity to a server itself.

### Requirement: The push window a source leaves unguarded
A precondition guards a push only where the source enforces it (an `If-Match`, a `STORE UNCHANGEDSINCE`); a source enforcing none leaves a window between the listing and the push in which a server change under an `Update`, a `Remove` or the flags a `SetFlags` names is lost (pimdir SYNC §4). The seam documents it; no row says which sources leave it open.

### Requirement: A server collection is deleted only when enabled
An engine mirroring collections between sources SHALL NOT delete a collection on one source's server because another source stopped listing it unless its configuration enables it, and then only when that collection is empty or every member was reconciled in the run (pimdir SYNC §5). The library mirrors no collection: the rule is the connector's and its configuration's, documented on the seam; `delete_collection` refuses a collection holding queued actions (the-store-follows-storage-draft-05).

### Requirement: A write answers what the merge recorded
`PimdirSourceStore::write` SHALL answer a `PimdirWritten` (`conflicted`, the handles whose upsert the write recorded as a divergence on the item; `overruled`, each body a policy did not adopt with the key of the retained item keeping it), and `service` SHALL answer `PimdirYield::WantsWrite` with `PimdirArg::Written`. Every verb accepts `PimdirArg::Write` as a write recording nothing beyond its ops, for a storage of its own.

### Requirement: A storage answers only bodies it read
A storage SHALL answer `PimdirYield::WantsVerify` with the bodies that read whole and hash to their name, and `lookup_objects` with only such bodies, so the engine never pushes, nor links a placement onto, a body that is missing or fails its name (pimdir STORAGE §7). A sync asks the verification for every body a push chunk uploads (sync.md), and a `Full` upgrade for the bodies its requested placements hold at `Full` as their source last agreed on them, fetching again those left out (upgrade.md). The reference storage moves a body failing its name into quarantine/ as it answers either (store.md).

### Requirement: A narrowed listing says so
A connector narrowing a round's listing by any date filter SHALL set `PimdirRemoteSnapshot::narrowed` on every page of that round and on no other page, and on its last page SHOULD hand `enumerated`, every handle the collection holds on the source read by an enumeration no date filter narrowed, where the source offers one (pimdir SYNC §4, §5): IMAP narrowing by `SENTSINCE` hands `UID SEARCH ALL`, JMAP narrowing by `after` an `Email/query` filtered by the mailbox alone, Graph narrowing by `receivedDateTime` the folder's message ids, Gmail narrowing by `after:` the label's ids where it lists them. A filter lists a member only when it matches, so it misses an undated member received before its window and one whose `Date` runs ahead of its arrival, and the engine finds such a member absent only on the enumeration's word. The constructors (`round`, `page`, `delta`, `band_head`) state neither.

### Requirement: A capped listing says so
A connector knowing that its source caps a collection's listing SHALL set `PimdirRemoteSnapshot::capped` on every page of that collection's rounds and on no other (pimdir SYNC §4, §5). The constructors state none.

### Requirement: A lookup answers every holder with its witnesses
`PimdirArg::LookupObject` SHALL carry, by link id, a `PimdirHeldObject` per row of the account holding a body under it: the body's `PimdirObject` and the `subject`, `date` and `sender` its mail summary states, the three `None` for a row holding none (`lookup_objects`, pimdir STORAGE §14). The verb, not the storage, decides which holder agrees ([upgrade](upgrade.md)), so a storage answers every holder, once per distinct answer, and only bodies it read whole. Before draft-10's second batch the reply was one `PimdirObject` per link id, the size its only witness. `PimdirHeldObject::agrees(summary)` is the agreement of STORAGE §9 on the three witnesses, byte for byte, an absent one matching an absent one and a date the store takes in no column read as absent; the upgrade reads it with the size, and the write recording a body's stand-ins reads it to tell another message under the key ([store](store.md)).

### Requirement: A load says which creates are offered
`PimdirLoaded` SHALL carry `offered`, the handles of the loaded `Created` placements the source binds not: the copies the store offers it of items other sources hold (`PimdirHub::offered`, pimdir SYNC §3, §9). A sync reads every other `Created` placement, and every delivered create, as a create of the source's own, and reports a member minted apart from one on a summary disagreement (`PimdirSyncEvent::Disagreed`, [sync](sync.md)). The field is empty by default: a storage stating none has every create read as the source's own, and so reports a disagreement with an offered copy too, never fewer.

### Requirement: The seq decision stays the storage's
No yield carries the public id: the write assigns a `seq` as it inserts an item, from the summary the upsert carries. A storage SHALL run `seq_for_link_any` bound with the `subject`, `date` and `sender` of the mail summary being written, the date `NULL` when unparseable, and with the three `NULL` for another kind or a message carrying no summary, and SHALL run it for no writer-derived key (pimdir STORAGE §9.1). `held_elsewhere` keeps its three parameters and reads both rows' summaries itself.
