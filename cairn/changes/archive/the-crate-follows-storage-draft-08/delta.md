---
cairn: delta
change: the-crate-follows-storage-draft-08
---

Folds into spec/store.md, spec/summaries.md, spec/sync.md, spec/upgrade.md, spec/search.md and spec/spec-fidelity.md, each phase folding its own part. Scenarios are written with the tests that land them; the requirements below fix the intent.

## ADDED Requirements

### Requirement: A store of another draft is refused
Every profile SHALL refuse, as `PimdirError::Version` naming both versions and writing nothing, a store whose `store_meta.format_version` is not `storage-draft-08` or which lacks the column (STORAGE §4.2, §6).

### Requirement: The store reads three compatibility versions
The crate SHALL read `version`, `read_version` and `produce_version` from `store_meta`; while the format is a draft the marker governs and the three stay 1 (STORAGE §4.2).

### Requirement: A store has an identity and an incarnation
The crate SHALL answer `store_id` and `incarnation` with the change cursor, and SHALL raise the incarnation through `raise_incarnation` when it restores a copy (STORAGE §4.5, §8).

### Requirement: The store records the sync version that wrote its sync state
An engine's first write SHALL record `sync-draft-07` through `set_sync_version`, and a store naming another sync draft SHALL be refused as `PimdirError::SyncVersion` before any verb runs (STORAGE §4.3, SYNC §10).

### Requirement: Every writing connection deletes securely and syncs fully
Every connection writing pimdir.db or index.db SHALL set `secure_delete = ON` and `synchronous = FULL` (`fullfsync = ON` on macOS); a reader or a producer SHALL wait on a busy timeout (STORAGE §4.1, §11.5).

### Requirement: A body's name is checked before it becomes a path
A hash SHALL be 52 characters of `a-z2-7` before any path is built from it, whatever it was read from, and no symbolic link SHALL be followed under objects/ (STORAGE §5).

### Requirement: A blob write never replaces a file
A blob SHALL move onto its name by a move that fails when the name exists, the existing file kept and the temporary discarded (STORAGE §5).

### Requirement: A body failing its name is quarantined
A body failing its name SHALL read as missing; the owner SHALL move it into quarantine/ under the exclusive staging lock and SHALL NOT unlink it; nothing reads quarantine/ as bodies (STORAGE §5, §7).

### Requirement: A queued body is read from its row
The drain SHALL read a queued body from `queue.object_hash` and SHALL park a row whose payload's `object` differs (STORAGE §15.3).

### Requirement: A malformed payload is parked
A payload missing a member its schema under payloads/ requires, stating one of another type, or holding one its version does not define SHALL be refused at the enqueue and parked at the drain, never applied in part (STORAGE §15.3).

### Requirement: An erase removes an item's bytes
`PimdirClient::erase` SHALL purge the item, collect, refresh the index with its thread prune, run `optimize_text` and checkpoint both files with `TRUNCATE` (STORAGE §11.5).

### Requirement: Store files are the user's alone
The crate SHALL create directories `0700` and files `0600`, SHALL open a lock file without following a link, and SHALL open a store from elsewhere with `trusted_schema = OFF` and the defensive flag (STORAGE §18).

### Requirement: A recorded error carries no credential
Error text the crate records in a column SHALL be scrubbed of a URL's userinfo, an `Authorization` header and any credential-named value first (STORAGE §9.2, §18).

### Requirement: A path built from a name stays in its directory
A path derived from a name SHALL neutralise separators, `.`, `..`, NUL, control and bidirectional formatting characters and stay in its directory (STORAGE §18).

### Requirement: A header is read as Annex A.1 reads it
`message_id`, `in_reply_to`, `subject`, `sender_name`, `date` and an `alt:` key SHALL derive by Annex A.1's parses at revision 3, every WHATWG label decoded and an unknown one kept literal.

### Requirement: A zoned time resolves through the IANA database
A `TZID` SHALL resolve through the resource's `VTIMEZONE`, then the IANA database, else as floating; a skipped wall time SHALL take the offset before the transition; a `VTIMEZONE` rule SHALL be followed for at most 1,000,000 periods (Annex A.3).

### Requirement: A narrowed listing proves only what its filter could list
A round whose pages state `narrowed` SHALL find absent only a member dated before the round opened, or one its `enumerated` handles lack (SYNC §4, §5).

### Requirement: An empty listing parks
A round or a rekey naming no member SHALL park while any based binding of its source is absent, unless the run carries `accept_absences` (SYNC §5, §8).

### Requirement: A mutable revive waits at Meta
A bodiless placement of a mutable kind reviving a retained row SHALL revive it at `Meta`, keeping the retained body over the reviving binding's handle, reported `Overruled` (STORAGE §11, SYNC §6).

### Requirement: No policy settles against an unread body
A divergence where either body is unread SHALL be recorded as `manual` records one; the upgrade or the fill reading the missing body SHALL then settle it by the policy (SYNC §5, §6, §9).

### Requirement: PreferLocal keeps the body it pushes over
Before pushing over an observed body, the engine SHALL keep it as a retained item, reported `Overruled`, in a write committed before the push (SYNC §5, §9).

### Requirement: A query is bounded
A query over 1,024 scalar values, 32 open groups or 128 terms SHALL be refused as `limit-exceeded`, in SEARCH §8.3's pass order.

### Requirement: Occurrences are bounded and a cut is reported
An item SHALL write at most 10,000 occurrences and examine at most 1,000,000 periods of a rule, record its cut through `set_cut`, and a dated answer's coverage SHALL count the cut items through `cut_before` (SEARCH §7, §8.2).

### Requirement: An index names its version and shape
The indexer SHALL write `search-draft-08` and `canonical` in `index_meta`, refuse an index of another shape or a later version as `PimdirError::IndexVersion`, and rebuild an earlier one of its own shape (SEARCH §2, §3).

### Requirement: Floating times are read in a recorded zone
The indexer SHALL state its floating zone, `UTC` by default, and re-expand every live calendar item when it differs from `index_meta.floating_zone` (SEARCH §7).

### Requirement: A body that cannot be read is tried again
A placement indexed bodiless for a missing or failing body SHALL be recorded in `missing` and folded again by every pass until indexed or deleted (SEARCH §4).

### Requirement: A message that leaves prunes its thread
When the last mail placement holding a link id goes, the refresh SHALL prune its thread through `unparent_messages`, `prune_messages`, `thread_rows` and `delete_thread` (SEARCH §9).

## MODIFIED Requirements

### Requirement: Schema version
The schema version SHALL be the number of canonical migrations vendored at the pinned pimdir revision, and an owner SHALL run no migration while the format is a draft.

### Requirement: A store declares its algorithm once and is refused on a mismatch
A store SHALL name `blake3` in `store_meta.hash_algo`, written by `init_store_meta`; the crate SHALL offer no other algorithm.

### Requirement: Blobs are content-addressed and sharded
A body SHALL live at `objects/<hash[0:2]>/<hash>`, one shard level.

### Requirement: A blob rename is durable
The no-replace move and the shard directory it may create SHALL be flushed before the write reports success.

### Requirement: The store records the derivation revision
An owner SHALL record its Annex A revision, 3, through `set_derivation` in the transaction of its first derived write, lower or higher than the store's.

### Requirement: A store derived under revision 1 is re-derived on open
Renamed "A store derived under an earlier revision is re-derived on open": an owner opening a store at revision 1 or 2 SHALL re-derive every held message and task through `list_held_mail` and `list_held_tasks` before recording 3.

### Requirement: Collections refusing tasks
`capability::CALENDAR_COMPONENT_VTODO` SHALL name `calendar.component.vtodo` (STORAGE Annex B), read only where a row states it; the gate is a SHOULD and the owner's backstop decides.

### Requirement: Another message under a held Message-ID is minted apart
A message SHALL agree (subject, date and sender byte for byte, a retained row on size too) with any row holding its key, a pending create, a live item, a tombstone or a retained row, before it is landed, joined or revived; one disagreeing SHALL be minted (STORAGE §9).

### Requirement: An item conflict's late body meets the policy
The upgrade SHALL settle a binding conflict it fetches the body of by the source's policy, and the fill SHALL settle a `prefer-` item conflict it meets (SYNC §5, §9).

### Requirement: The walk of a message's parts is streamed and bounded
The walk SHALL read at most 10,000 entities, the message's own counted, none deeper than 64, and stand for at most 1,000 parts (Annex A.1, STORAGE §14.3).

### Requirement: The index is derived and rebuilt, never migrated
The index SHALL be rebuilt when built from another store or incarnation, at an earlier version of its shape, or under another tokenizer or schema text, and refused otherwise as SEARCH §2 says.

### Requirement: A rename past a cut pass is reconciled
The purge count alone SHALL reconcile keys, a rename counting in `store_meta.purges` (SEARCH §4).

### Requirement: The vendored copy is the specification's, byte for byte
spec/ SHALL hold migrations/, queries/, vectors/ and payloads/ as pimdir holds them at the revision spec/REVISION names, vectors/ and payloads/ checked against vectors/manifest.json.

### Requirement: The check degrades to a skip, never to a failure
Renamed "The vectors are read from the vendored copy": every suite SHALL read spec/ and none SHALL skip; the comparison with a pimdir checkout runs at the pinned revision when one sits beside this one.

## REMOVED Requirements

### Requirement: The capability tables reconcile on open
Drafts are exact (STORAGE §4.2): nothing is reconciled.

### Requirement: Open reconciles against the canonical text
Drafts are exact: an earlier draft's store is refused.

### Requirement: An owner open that reconciles nothing
No open reconciles.

### Requirement: A reader or a producer reads an older store as it stands
An older store is refused by every profile.

### Requirement: A migration runs after the reconciliation and keeps every row
Migrations exist from storage-v1 on; none runs at this draft.

### Requirement: A store of another shape is refused
`store_meta.shape` is gone (D3): one on-disk format.

### Requirement: A storage-draft-06 store is reconciled with its lists
A storage-draft-06 store is refused.

### Requirement: A reader reads the task columns an owner has not reconciled as NULL
No store reaches a reader unreconciled.

### Requirement: The index reads an older store as its reconciliation would leave it
An older store is refused before the index reads it.
