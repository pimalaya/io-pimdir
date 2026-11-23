---
cairn: delta
change: the-store-follows-storage-draft-05
---

## MODIFIED Requirements

### Requirement: Blobs are content-addressed and sharded
An object's bytes SHALL live at `objects/<hash[0:2]>/<hash[2:4]>/<hash>`, immutable once written, so an identical body delivered twice is stored once. `PimdirBlobs` reads a blob independently of the SQLite connection, so a body can be read while the store is mutably borrowed to service a sync. It SHALL be read-only: `get`, `reader`, `path`, `files` and the hash, with no writer, so a handle that holds no lock (a reader's, or one opened from a path) cannot put a body in the tree.

`get` SHALL check the bytes it read against their name (STORAGE §7): a file that does not hash to its name answers `None`, the state of a missing one, so no parser, index, gate or drain is handed corrupt content. `reader` streams unverified; a consumer reading it whole hashes what it read.

A body SHALL be written only through `PimdirBlobStage`, which a producer (`PimdirProducer::stage_body`, `blob_writer`) or an owner (`PimdirStore::blob_stage`) hands out and which holds that handle's lock for as long as it lives: a producer's shared staging lock, an owner's owner lock (STORAGE §8). A body written through it is therefore never in the window a collector runs in.

### Requirement: A blob rename is durable
Writing a blob SHALL go through a temporary file in the shard directory named uniquely to the writer (the body's name, the process id and a per-process counter), so two processes storing one body never write, then rename, each other's bytes; a temporary another writer holds is left alone, and a failed write removes its own. It SHALL sync the shard directory after the rename, and every directory above it up to objects/, since the write may have created the shards themselves. Syncing the file makes its bytes durable and says nothing about the name that reaches them, while the database commit is durable, so without it a crash can leave a committed row pointing at a body that never arrived: the one asymmetry the write order exists to prevent.

#### Scenario: Another writer's temporary
- GIVEN a temporary file another writer holds under the body's old shared name
- WHEN a producer stages the same body
- THEN the body lands under its name and the other temporary keeps its bytes (tests/store_rules.rs `a_blob_write_never_takes_another_writers_temporary`)

### Requirement: A collection can be deleted
`delete_collection(collection)` SHALL remove a collection the operator no longer wants, `DELETE_COLLECTION` cascading over its items, bindings and sources with retention not applying, then `RECOMPUTE_REFCOUNTS` in the same transaction, since the cascade releases pins no statement returns (STORAGE §14). It answers whether a row went. A collection holding a pending or parked queue row SHALL be refused as `PimdirError::Queued { collection }` (code `collection-queued`), nothing deleted: `collections_keep_queued` refuses the delete, which would drop the action with no receipt; the operator cancels the rows first.

#### Scenario: A queued action holds its collection
- tests/store_rules.rs `a_collection_holding_a_queued_action_is_not_deleted` (pending, then parked, then dropped and deleted)

### Requirement: A collection keeps the role its source states
`PimdirStore::set_collection_role(collection, role)` SHALL set or clear `collections.role` on a declared collection (pimdir STORAGE §14), the old holder of the role in the account losing it in the same statement (`collections_role_moves`), both collections stamped in the feed. The column is an open vocabulary no constraint checks (§13): the owner SHALL refuse a role outside its collection's kind vocabulary (`collection::role_of_kind`) as `PimdirError::Role { collection, role }` (code `collection-role`), and a reader SHALL read a role it does not know, or one of another kind's, as `None`. `PimdirCollection::role` SHALL carry it on `list_collections` and `list_collections_by_account`; a reader of a store its owner has not reconciled SHALL read every role as `None` when the column is absent.

#### Scenario: A server moves its Sent folder
- **GIVEN** `imap/Sent` holding `sent`
- **WHEN** the owner sets `sent` on `imap/Sent Items`
- **THEN** `imap/Sent` holds no role, `imap/Sent Items` holds `sent`, and both are above the reader's cursor

### Requirement: A binding persists what it last agreed with the hub on
The `bindings` table SHALL carry `shared_object`, round-tripped through `PimdirBinding`: the hub's shared body this source last reconciled against, which is the base of the cross-source merge. It is meaningful on every binding, conflicted or not, and `NULL` only until the source has folded once, where the sync base stands in for it.

This is the second base, for the second axis, and the store MUST keep both. `base_object` is what the source last agreed with its own **remote** and only a sync moves it, which is what keeps a pending push derivable; `shared_object` is what it last agreed with the **hub** and every live absorb moves it. Read one as the other and a source disagrees with itself: its own unpushed edit leaves the sync base behind the shared body exactly as another source folding in does, so the next edit is filed as a cross-source conflict and dropped.

`shared_object` SHALL reference `objects(hash)` and be pinned as `base_object` is (STORAGE §5): counted in the write path's refcount diff, in `recompute_refcounts` and in the collector's reachability, released when the binding agrees with another body, and released by `release_bases_before` with the item's own body. It is the common ancestor a merge reads, so it outlives the item's own body until the binding reconciles against what replaced it.

#### Scenario: The agreement point survives a reopen
- GIVEN an ordinary unconflicted binding written by a sync
- WHEN it is read back after a restart
- THEN it carries the shared body the reconcile settled on

#### Scenario: A conflicted binding carries one too
- GIVEN a binding whose own remote diverged from it
- WHEN it is read back
- THEN its agreement point is there beside the conflict, the flag gating only the conflict pair

#### Scenario: A second offline edit across a restart is not a conflict
- GIVEN a store whose single source edited an item offline, with the push still pending
- WHEN the store is reopened and a second offline edit is absorbed
- THEN the hub adopts it and the item holds the second body

#### Scenario: The agreement point pins its body
- GIVEN a source whose agreement point is the only pointer at a body
- WHEN the collector runs, then the source agrees with another body and the collector runs again
- THEN the first run keeps the body readable, the second takes it (tests/shared_object_pin.rs)

### Requirement: The store repairs what it can recompute
`recompute_refcounts` SHALL settle every object's count from the six columns that pin one (STORAGE §5, §7), `bindings.shared_object` among them, returning how many disagreed, and `clear_dangling_bindings` SHALL delete the bindings whose item is gone, returning how many. Both recover a fact the store already holds. `refcount_drift` compares the stored counts with the same six columns.

Every other dangling row stays reported and untouched: an item whose object row is missing is still the item, and a queue row whose body is missing is still an intent somebody expressed, so deleting either would destroy data rather than repair it.

### Requirement: The queue derives summaries from bodies
A queued `add` or `update` SHALL carry no summary: the drain reads the body the producer wrote and derives the key, the summary and the sort key under Annex A for the collection's declared kind, parking an `add` that names no link id and derives none.

A body an `add` or `update` names that the blob store lacks, or whose file does not hash to its name, SHALL fail the row as a failure of the environment (`PimdirError::Io`, `NotFound`), never apply it: its attempts are bumped and it stays pending, then parks past `MAX_ATTEMPTS` (pimdir STORAGE §15.2). A file failing its name is unlinked by the owner under the staging lock taken exclusively, left for the next read when a producer holds it shared, so the next write of that body writes it anew (STORAGE §7).

#### Scenario: A body unlinked after its enqueue
- GIVEN a queued `add` whose blob was unlinked after the enqueue
- WHEN the owner drains `MAX_ATTEMPTS` times
- THEN each drain but the last counts it `retried`, the last parks it with the error naming the body, and no item was created (tests/queue.rs `a_queued_add_missing_its_body_is_retried_then_parked`, and `a_queued_update_missing_its_body_is_retried_then_parked` for an `update`)

#### Scenario: A body failing its name
- GIVEN a queued `add` whose blob had a byte flipped
- WHEN the owner drains, the producer stages the body again, and the owner drains again
- THEN `get` answers none, the first drain retries and unlinks the file, and the second applies the add (tests/store_rules.rs `a_body_failing_its_name_is_no_body_and_the_owner_unlinks_it`)

### Requirement: An owner skips the actions it cannot apply
An action kind the store does not recognise SHALL decode as an opaque action (kind, raw payload, and the body hash the payload pins) rather than an error, so one queue can carry store mutations any owner applies beside capability-bound intents (a mail submission) only a specific owner can perform. A payload stating a version `v` this owner does not know, for any kind, SHALL be skipped the same way (STORAGE §15.2). A payload that is not JSON, or states no `v`, SHALL still park.

The drain SHALL **skip** a row it cannot apply: the row stays pending, `error` `NULL` and `attempts` unbumped, is never parked (parking means permanently unappliable), and never blocks later actions in the same collection. The drain report SHALL count skips beside applies and parks.

#### Scenario: A later producer's payload
- tests/store_rules.rs `an_unknown_payload_version_is_skipped_not_parked`

## ADDED Requirements

### Requirement: A stated identity beginning like a derived key is escaped
The key a body states (a `Message-ID`, a vCard or iCalendar `UID`) SHALL be filed under `esc:` and the hint when the hint begins with one of the reserved prefixes `alt:`, `hash:`, `dup:`, `part:`, `file:` or `esc:` (STORAGE §9, `PimdirLinkId::RESERVED`, `PimdirLinkId::from_hint`), so it never reads as a key the writer derived. A connector naming the identity to its server strips the one leading `esc:`.

#### Scenario: A card whose UID reads like a hash key
- the summaries vector case card-reserved-uid.vcf (tests/summaries.rs)

### Requirement: An edit names the body it replaced
`PimdirAction::Update` SHALL carry `if_match: Option<PimdirHash>`, encoded as the payload's optional `if_match` (STORAGE §15.3): the body the producer read and replaced. The drain SHALL park the row, naming both bodies, when the item holds another body, and apply a row stating none over whatever the item holds.

#### Scenario: Two producers edit one read
- tests/store_rules.rs `an_update_whose_read_body_was_replaced_parks`

### Requirement: A write staged with its body stages nothing when refused
`PimdirProducer::enqueue_with_body(collection, body, |hash| action)` SHALL hash the body, build the action from its hash, run the gate of §15.6 on the bytes in hand and, for an `add`, the duplicate check of `enqueue_add` on its link id or on the key the drain would derive from the body, then write the body and append the row in one transaction, answering the row id. A refused write SHALL leave objects/ as it was. `stage_body` with `enqueue` or `enqueue_add` keep working, a refused write there leaving its body for the collector.

#### Scenario: A duplicate and an unsupported create
- tests/staging.rs `a_create_refused_for_its_link_id_stages_nothing`, tests/capabilities.rs `a_refused_create_stages_nothing`

### Requirement: A calendar item has an attachment when its field is not empty
The calendar extraction SHALL put each `ATTACH`'s `FILENAME` (or `X-FILENAME`) and `FMTTYPE` in the `attachment` field, and `has:attachment` SHALL match a message whose `mail_summary.attachment` is 1 or a calendar item whose indexed `attachment` column holds a token (SEARCH §6, §8), read from `object_text_docsize` through the `pimdir_column_size` function every search connection registers, the index being contentless. This requirement belongs to search.md.

#### Scenario: An event attaching a file
- the search vectors `has:attachment` and `has:attachment kind:task` over items 13, 16 and 23 (tests/search.rs)
