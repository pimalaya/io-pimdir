# Changelog

All notable changes to this project are documented in this file. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0/).

## [Unreleased]

### Added

- **`PimdirReader`, the read role (spec §8).** The format names three roles and the crate shipped two handles: `open_read_only` returned a `PimdirStore`, the type that also drains the queue, sweeps the objects and purges the trash, so a consumer that only reads held a handle that could destroy the store and "it never calls those" was the only thing keeping it from doing so. The reads move to a handle that takes no lock and carries no write at all; `PimdirStore` dereferences to one, so the projection stays a single implementation whichever role reads it. `open_read_only` is deprecated.

- **The pending-action overlay (spec §15.4).** A reader built with `with_pending` folds the queue's pending actions over the committed items, so a producer sees what it staged before the owner applies it: `set-flags` and `update` restate an item, `remove` and `move` take it out of a collection, and `move` and `copy` bring it into the target. All of them address an existing item, whose `seq` follows its link id store-wide (spec §9.1), so nothing invents an identifier. A queued create has no `seq` until the owner applies it and is reported apart, by `pending_creates` and `count_pending_creates`. Parked rows never overlay, their error saying they will not be applied without an operator. The choice is made when the reader is built, never per call, so one handle cannot answer two ways about one collection. A page keeps its meaning: the fold reads past the limit by the number of pending removals and cuts back afterwards, so a page comes back short only where the collection ends and a scan written to page until a short page does not end early.

- **`PimdirStore::cancel_action`**, cancelling one queue row as the owner while holding that role only for the length of the call. Cancelling is an owner write (spec §15.5) and the only retraction a queued create has, the kinds addressing an existing item being retracted by their inverse instead. A consumer that is otherwise a reader and a producer needed the whole owner handle to reach it, which is the handle it must not hold.

- **`PimdirBlobs::path`**, where a body under a hash lives. The sharded layout is normative (spec §5) and the format invites a consumer to stream a body straight to that path and index it with a byteless `StoreObject` afterwards (spec §14), which until now meant deriving the sharding a second time.

- **`conventions`, the per-kind derivations (spec Annex A).** One `derive(kind, body)` returning the `link_id`, the `meta` summary and the `sort_key` for `message/rfc822`, `text/vcard` and `text/calendar`, in the I/O-free core so a consumer running its own SQLite driver reaches it. Annex A is informative, so nothing reported the divergence three consumers writing it separately had already produced: one wrote a whole calendar summary and a resolved key where another wrote `{"v":1,"etag":…}` and an empty key for every item. The fallback ids are fixed here too (`alt:` for a message with no `Message-ID`, `hash:` for a card or a resource with no `UID`), because two writers disagreeing about one item's id link it twice and store one body twice. Checked against the format's own vectors, fixtures read as bytes and structures compared rather than JSON text.

- **`pimdir gc`, the collector.** It drops the object rows nothing references, unlinks their bodies and the orphan blob files a crash left, and reports what it freed. It takes the owner role, so it never runs beside a sync, and reports a producer mid-append rather than waiting on one. A store that wants its unreferenced objects bounded schedules the verb.

- **One owner, enforced (spec §8).** Every owning handle takes an exclusive advisory lock on the store directory and holds it for its lifetime; a second owner gets `PimdirError::Owned` immediately, naming the store, rather than the 30-second `busy_timeout` wait that used to end in a stall with no signal. The rule is about processes, so a two-sided sync or a multi-account owner opening several handles still holds one lock. Readers take none. Producers take a shared one, so several append at once while none of them keeps the owner out, and a body is never between the blob tree and the queue row that pins it while a collector runs. The kernel releases the lock with the process, so a crash leaves nothing to recover.

- **`bindings.ambiguous_handles`**, the other handles a source holds one item's identity under. Folded into version 1 and reconciled on open, so a store written by an earlier draft gains it without a resync. `pimdir check` reports the bindings carrying any: not a defect, but the reason those items stop syncing, and an operator looking at a frozen item has no other way to see why.

### Fixed

- **A handle-space rebuild froze every item of its collection.** A rekey drops the whole old spine and upserts each item under its new handle, in one batch (spec §12), and the hub diff read that as one source reporting an identity under a second handle: the duplicate-link-id floor kept the handle the server had just voided and recorded the new one as ambiguous, so the engine derived nothing for the item in either direction, and nothing could clear it, since what clears an ambiguity is the source reporting the recorded handle gone and the recorded handle was the live one. An IMAP `UIDVALIDITY` bump therefore froze the mailbox instead of renumbering it. The drop's reason is what separates the two: a `Superseded` drop licenses the rebind of the handle it names, and only that one, so a rebuild carrying a genuine second copy still freezes that one.

- **A trash page sorted the whole trash to return fifty rows.** `list_retained_page` moved onto the public `seq` (spec §14.1) and `items_retained` did not move with it, so the read could not ride its own index. An existing store is repaired on open: an index whose columns moved is now dropped and recreated, which the ensure batch could not do, `CREATE INDEX IF NOT EXISTS` keying on the name and leaving the old shape in place silently.

- **A process could refuse itself the store it had just released.** The owner lock is the process's and shared across handles, but a strong count reaches zero before the file description it named is closed, so a handle taken in between opened a second description and `flock` refused it against this process's own: `PimdirError::Owned` naming a store nobody else held, on no schedule, which nothing above can act on. The registry owns the description now and closes it as the last handle goes, inside the same critical section the next acquisition takes.

- **A body stored without a placement was destroyed at the end of the batch.** Every write swept the object rows at refcount zero and unlinked their blobs, so a consumer that streamed bodies into the blob tree and attached them in a later batch — which spec §14 invites, and which `STORE_OBJECT` inserting at refcount zero exists for — lost them silently, bytes included, before the batch that would have referenced them ran. No write collects any more; `pimdir gc` does.

- **A body lookup crossed accounts.** `lookup_objects` resolved a link id against every collection in the store, so two accounts holding the same vCard `UID`, which spec §9.2 names as a thing unrelated servers do, handed each other's bodies across: the receiving sync then believed the item was hydrated and never fetched the real one. It is scoped to the caller's own account now, which is the axis a link id is trustworthy on; across collections it still answers, which is what the read exists for.

- **A base of no revision, no body and unread markers round-tripped as no base at all**, so an agreed placement read as never-agreed and the sync re-derived the same push on every run. `bindings.base_present` records the fact its three value columns cannot express; those columns stay a witness for rows written before it.

- **A write silently repointed a binding to another handle, and the evidence went with it.** A binding pins one handle, so a source holding one link id twice (a double delivery, a retried append, a restore, a migration) had nowhere to put the second copy: the write repointed the binding, and no layer above could afterwards tell the source held the identity twice. Deleting the bound copy then propagated a delete that removed the only copy on a source nobody touched. The bound handle now stays and the incoming one is recorded, which freezes the item until the source holds the identity once again.

- **A write carrying a new sort key silently discarded it.** The diff that decides whether a row needs an `UPDATE` compared every column the statement writes except `sort_key`, so a key that changed and nothing else reported the row unchanged and no statement was issued. A key is derived rather than given, and a connector fixing its derivation, a tzdb update moving a zoned start, or the second source of a two-source sync all restate one; the item stayed where the first derivation put it, for good. The suite missed it because it only covered the other half of the invariant, that a write carrying no key must leave the stored one alone.

- **A descending page hid every item sorting above its first cursor.** "No cursor" was expressed as a key no real one could outrank, but a sort key is arbitrary text a writer derives, so no value is reserved and the sentinel was outranked by two of the same character. Such an item was invisible to every descending page, permanently, while the count still reported it. The statement now says what it means, a `NULL` cursor, and keeps the same keyset comparison and the same index.

- **Two owners draining one collection applied every action twice.** The pending rows are read outside any transaction, and the row was deleted at the *end* of the applying transaction, so a second owner holding the same list re-applied all of it; `add` and `copy` are not idempotent, and the operator CLI opens a second owner handle routinely. The delete is now the first statement of the transaction (`CLAIM_ACTION`, a `DELETE ... RETURNING id`) and a claim that deletes nothing skips the row: exactly-once is a property of the statement rather than a convention about who runs the drain.

- **A blob rename was never made durable.** The body was written, `fsync`ed and renamed, and the directory entry that carries the name was not synced, while the SQLite commit is. A power loss could leave a committed row pointing at a body that never arrived, which is the one asymmetry the write order exists to prevent.

- **A flag set the store could not decode read as a known-empty one**, an authoritative "this item carries no markers" that the merge took as one side's opinion: it cleared every marker the other side reported and persisted the result, turning a read failure into permanent loss. It now decodes as unknown, which holds no opinion.

- **`created_at` held epoch milliseconds** where the column is declared to hold an RFC 3339 timestamp, and the empty string when the clock predated the epoch. It is written by SQLite itself now, in the form the retirement clock already uses, which also keeps the crate free of a clock.

### Changed

- **A write no longer holds the database's writer lock across a file write.** Bodies land in the blob store before the transaction that indexes them opens, which spec §14 asks for in as many words: inside it, the same write held SQLite's single writer lock across a file write, two `fsync`s and a rename, serialising every other writer behind an I/O path that touches no database page. The queue drain, which builds its ops inside the transaction that claims its row, keeps writing inside it; the blob write is idempotent, so that costs one existence check.

- **The collector no longer holds every hash in memory.** It read the whole `objects` index into a set to diff the blob tree against, which is hundreds of thousands of names at the scale spec §1 promises, to answer a question that is always about one file. It asks per file on the primary key now (`OBJECT_EXISTS`, new in the format's statements).

- **A purge releases the pins its own delete reported.** `PURGE_ITEM` and `PURGE_RETAINED_BEFORE` return each removed row's `object_hash` and `conflict_object`, so the pins are settled from the statement that took them rather than from a read that visits every swept row a second time. `RETAINED_ITEM_BY_SEQ` and `RETAINED_BEFORE` are retired.

- **The format's own SQL is checked, and so is the format's one MUST vector.** The fidelity suite prepares every canonical statement against the canonical schema rather than only checking that each is inlined here by name: this crate holds the only toolchain that ever loads those files. `tests/objects.rs` checks object naming against `vectors/objects.json` (spec §16), both algorithms, whole and streamed, shard paths included. Both suites, and `conventions`, skip silently without the sibling spec checkout, so the new CI in this repository and in pimdir asserts they ran.

- **The same code, once.** A pure refactor, no behaviour attached: fourteen reads that each wrote out prepare-map-push-return share one `rows` helper; the statements and the `sql::ALL` index are declared by one macro, so the two tests that re-read this crate's own source to keep them in step are gone; `PimdirRetainedItem` is folded into `PimdirItem` as an optional `retention`, read by one row mapper instead of two; the operator tool's second read-only connection (`PimdirDb`) is folded onto the store as a diagnostics block; `park` becomes a call to `fail_action`. Breaking: `PimdirRetainedItem` no longer exists, `PimdirPlacement` carries the same typed columns as every other read (`ReplicaLinkId`, `ReplicaFlags`, `ReplicaLevel`), and `PimdirError::Version` splits into `Version` (a schema this crate does not service) and `Uncreated` (no schema yet, which only an owner creates).

- **`check` diagnoses and `--fix` repairs; neither reclaims.** `--fix` used to delete orphan blob files behind a `--grace` window and a confirmation prompt, while every write swept with neither. Both flags are gone: it now recomputes the drifted refcounts from the pointers that justify them and clears the bindings whose item is gone, which destroys nothing and needs no guard. Orphan files are reported, and `pimdir gc` takes them.

- **A purge reports the rows it retired, not the bytes it reclaimed.** It releases the references a retained item held; the bodies are freed by the collector, which is what can report them. `PimdirPurgeReport` loses its `bytes` field and `item purge` its byte count.

- **`item restore` queues rather than fails when the store is owned.** It appends its action first and takes the owner role only to apply it, so a restore issued while a sync runs reports `queued` and is drained by the owner that holds the store, which is what the queue is for. Purge and `queue cancel` still need the role and still say so.

- **A store handle names a source only where an operation acts as one.** `PimdirStore::open(dir)` and `open_read_only(dir)` take no source, and `for_source(source)` yields `PimdirSourceStore`, which carries the storage seam, the rekeyed write and the queue drain, and dereferences to the store for everything else. Breaking: the constructors lose a parameter. Purge, queue cancellation and every read consulted no source but had to be handed one anyway, so the CLI invented `"pimdir"` for its readers and scanned `bindings` for a name it then discarded. Both are gone, and a store an operator reads and sweeps now records no source at all.

- **`io-replica` is a path dependency** until the release carrying `ReplicaStatus::Ambiguous` is published; it becomes a version dependency again at that point. Taking those types meant taking the rest of the engine's new API with them: `load` carries a scope, `DropPlacement` carries a reason, and `ReplicaCollection` is gone.

- **A write reads only the rows its batch names.** Folding a batch into a collection loaded, cloned and diffed the whole collection, so the cost of one flag on one message was the size of the mailbox: measured 3.5 ms at a thousand items, 13 ms at four thousand, 59 ms at sixteen thousand, cleanly linear, against a promise of hundreds of thousands. The read is now scoped to the link ids the batch carries, with each dropped handle resolved through the new `bindings_by_handle` index, and the same measurement is flat at 150 to 175 µs across that range.

- **The residual is keyed rather than listed.** A first sync probes a whole collection before linking any of it, so the list grew to the collection size while every insertion, drop and lookup searched it linearly.

- **The drain answers its point questions with point reads.** The `Add` collision check, the handle lookup and the mutation it drives each loaded the whole collection, once per drained action.

- **`release_pins` is one statement** rather than one per hash: a purge of fifty thousand retained items was a hundred thousand point updates inside one transaction.

- The object sweep tests `refcount <= 0`, so a count a double release drove negative is collected rather than leaking for ever with nothing reporting it.

- New indexes: `objects_garbage` (partial, so the sweep stops scanning the whole table on every write transaction), `items_by_seq_global` (the store-global public id the format promises had no store-global index), `bindings_by_handle`, `items_by_conflict_object` and `queue_by_object`.

## [0.3.0] - 2026-08-24

### Added

- **The store owns the content hash its objects are named by.** `PimdirHashAlgo` implements both algorithms the format admits (`blake3`, recommended, and `sha256-128`) with the encoding spec §5 fixes, lowercase base32 (RFC 4648, no padding), and a store, a producer and the algorithm itself hand it out whole (`hash`) or incremental (`hasher`, for a body streamed into the blob store).

  Until now this crate stamped `store_meta.hash_algo` with `blake3` and hashed nothing, while every Rust consumer carried its own 128-bit FNV-1a rendered as hex and the Android app computed `sha256-128` as base32. The recorded algorithm was therefore false, the digest was not cryptographic (spec §2), the encoding was not the one a blob path is specified to use, and the two implementations of one store named the same body differently: no dedup, no blob found, and nothing erroring while it happened.

  `open_with_hash` declares the algorithm a store is created with, `open` adopts whatever an existing store records, and an open declaring a different one is refused with `PimdirError::HashAlgo`.

  **Breaking**: `PimdirBlobs::open` takes the algorithm too, since the blob directory is what names files by it; `PimdirStore::blobs` hands out a handle already bound to its store, which is how a consumer avoids picking one.

- **A store created before the rename cascades is refused on open**, with `PimdirError::Unreconcilable` naming the table. Every foreign key onto a renamable parent carries `ON UPDATE CASCADE` (spec §14), and no `ALTER TABLE` can add one, so the draft reconciliation cannot reach it; spec §6's other branch is to refuse the store and have the operator recreate it, which costs a resync of what the format calls a derived cache.

  Opened anyway, such a store works until something renames a collection, and then SQLite refuses the rename one dependent row down: a server-side rename or an account rename could never be applied, and nothing said so until one was attempted. Every store created by 0.2.0 is in this state.

- **An unknown flag set is written as `NULL`.** Spec §13 keeps two absences apart: `NULL` means nothing has read an item's markers, `'[]'` means it is known to carry none. io-replica had no unknown state, so this crate wrote at least `'[]'` and a probed placement claimed to carry no markers. `flags_to_json` now returns `None` for `ReplicaFlags::Unknown` and `flags_from_json` decodes a `NULL` column back to it, so the column means what the specification says it means.

  In a queue payload (§15.3) an unknown set encodes as `null` rather than `[]`: an action states an intent, so its set is known in every payload the format defines, and a nonsensical one stays legible instead of reading as a deliberate clearing of every flag.

- **The two schema stamps are checked against each other on open.** Spec §4.2 has `PRAGMA user_version` and `store_meta.version` mirror one another and calls a store where they disagree corrupt; this crate read only the pragma, so a half-applied schema change opened as a store at whichever version the pragma happened to hold. Both the owner and the read-only open now refuse it with `PimdirError::VersionMismatch`. A store whose `store_meta` row is absent is left alone, since refusing there would make a missing stamp unrepairable.

- **A collection can be paged in its kind's own order.** `items` gained a `sort_key` column and an `items_by_sort` index, and `list_items_page_asc` / `list_items_page_desc` return a keyset page ordered by it: newest first for mail, A to Z for contacts, a date range for calendars. Until now the only orderings a store could serve were by `link_id` or `seq`, neither of which means anything to a reader, so every consumer had to scan a whole collection into memory to show fifty rows.

  The cursor is the `(sort_key, seq)` pair rather than the key alone, because a key is not unique: two messages share a timestamp, two contacts share a name. `seq` breaks the tie, which is what stops a page boundary that lands inside a tie from skipping an item or serving it twice. The first page takes no cursor.

  An empty key means unknown and is the default, so an item is orderable before it has been summarised: it sorts to the end of a newest-first listing and to the head of an A-to-Z one.

  `set_sort_key` restates one item's key, for a store written before its kind had a convention or a consumer whose sync engine does not carry the key inline yet. An ordinary write never resets a key it does not carry.

- **A collection can declare which account it belongs to.** `collections` gained a nullable `account` column and the partial `collections_by_account` index (spec §9.2), a handle speaks for one account (`PimdirStore::open(dir, source).for_account("work")`, and the same builder on `PimdirProducer`), and `list_accounts`, `list_collections_by_account`, `collection_account` and `set_collection_account` read and restate it. Without the column, a merged view, whose defining operation is "everything except this account", had to reverse-engineer the owner's naming convention with a `LIKE 'work/%'` over collection ids. Folded into version 1, since the format is still a draft, and reconciled on open, so an earlier-draft store heals rather than failing on a missing column.

  **The account partitions nothing, which is the substance of the change.** Link ids, object hashes and `seq`s keep their store-wide meaning, so two accounts holding one `Message-ID` share a `seq` and one body reaching both is one object refcounted twice: scoping identity per account would compile a mail-shaped policy into a kind-agnostic store, and would leave one link id carrying two short forms. What reports the multiplicity instead is `link_placements` on the identity axis and `object_placements` on the dedup axis, each returning every live placement (`PimdirPlacement`) with the collection and account it sits in, and resolving nothing: a mail view lists them, because two receipts of a newsletter have two read states, while a contact view may offer to merge them.

  There is no `accounts` table: the store records which account a collection belongs to and nothing else, so credentials, endpoints and display names stay with whatever configures the owner. `list_accounts` is therefore what the collections say rather than a configured roster, and an account with no collection yet does not appear in it.

- **`rename_collection`**, which gives a collection a new id and carries its items, bindings, sources, queue rows and child collections with it. Every foreign key onto `collections(id)` is now `ON UPDATE CASCADE`, as is `bindings(collection, link_id)`, which is a parent one level down and refuses the cascade without it.

  This is the only safe way to change an id, and it matters because the obvious alternative is destructive: deleting a collection and recreating it under a new id cascades every item and binding away, turning a rename into a full re-download and discarding staged local changes. A server renaming a folder and an owner renaming an account both land here.

- **A spec-fidelity test suite** comparing the inlined `sql` module against the canonical pimdir specification checked out beside it: the schema semantically (columns, defaults, foreign-key actions and indexes, through SQLite's pragmas rather than by text), the presence of every canonical statement by name, and that every inlined statement prepares against the inlined schema. Statement *text* is deliberately not compared, since the specification permits an equivalent substitution; the three this crate uses are listed explicitly instead. Skips when the specification is not checked out beside this crate.

### Fixed

- **A store created by 0.2.0 was unreadable.** The draft-shape reconciliation on open (spec §6) did not carry `items.sort_key` or the `items_by_sort` index, although both were folded into version 1 after 0.2.0 shipped, so every paged read of such a store failed with `no such column: sort_key`. The column and the index are now reconciled with the rest, and a regression test derives an earlier-draft store from the current schema by dropping each folded-in column, so the next fold is covered without rewriting it.

  Still not reconcilable: a 0.2.0 store carries no `ON UPDATE CASCADE`, which `ALTER TABLE` cannot add, so `rename_collection` on one is refused by SQLite rather than silently orphaning its rows.

- **The inlined schema had drifted from the specification.** `sql::MIGRATION_0001` carried neither the `sort_key` column nor any `ON UPDATE CASCADE`, so this crate was creating stores that did not match the format it implements, and nothing detected it. The point of `sql` is to be the canonical copy a consumer runs on its own SQLite driver, so a silent disagreement is the worst failure it has; the fidelity test above exists so it cannot recur.

### Changed

- **`ItemRow.flags` is `Option<Vec<String>>`**, `null` in the JSON output while nothing has read the markers. Breaking for anything parsing `pimdir item list --output json`.

- **`PimdirItem` and `PimdirRetainedItem` gained a `sort_key` field.** Breaking for anyone constructing them.

- **The sort key io-replica now carries on a placement is bound on write.** `load` returns it, insert and update write it. This reverses the arrangement above, where the key was preserved by the update never naming it: that held only while nothing upstream carried a key, and a `load` that drops it now hands every save an unknown key, which the update would write back, blanking on every sync what the previous one derived. Both halves have to carry the key or neither can.

## [0.2.0] - 2026-08-07

### Added

- **A `pimdir` operator CLI**, shipped from this crate behind the `cli` feature (a `[[bin]]` with `required-features`, so a library consumer never compiles clap or any terminal dependency). It is to a store what `sqlite3` is to a database: `collection list`, `item list` (live or `--retained`, keyset-paged), `item show`, `item export`, `item restore`, `item purge` (one `seq`, `--older-than <DURATION>` or `--all`), `queue list [--parked]`, `queue cancel`, `store info`, `check [--fix]` and `export`, plus `completions` and `manuals`, each rendering as JSON under `--json` with logs on stderr.

  It **never interprets item content**: a store is kind-agnostic, so the tool prints ids, flags, levels, object hashes and the raw meta, and exports bodies byte for byte. Rendering a message or a vCard belongs to himalaya and cardamum.

  Reads open the store read-only, so inspecting a store while a sync runs is always safe. `item restore` goes through the queue as a producer and then drains that collection as the owner, reading the item back to report *applied* rather than trusting collection-wide drain counters; when a sync holds the lock the action stays queued and applies at the next drain. Purge, queue cancel and the orphan sweep have no action kind, so they take the owner role directly, and `PimdirError::Busy` reports as "another writer holds the store lock (a sync is running?)". Destructive verbs confirm (with counts and bytes when the store can price them) unless `--yes`, and refuse to prompt into a pipe or under `--json`. Listings never truncate silently.

  `check` closes two gaps the format leaves open: orphan blob files (a crash may leave one and nothing cleaned them) and refcount or reference drift. Only orphan files are reclaimable (`--fix`, guarded by a `--grace` window because a body is written before the row referencing it); drift and dangling rows are reported, never repaired.

- **The store retains items instead of deleting them.** An item whose last source binding vanishes is now soft-deleted rather than removed: `items` gained `retained_at` (RFC 3339, stamped by SQLite so the crate stays clock-free) and `retained_by`, plus a partial `items_retained` index. The row keeps its `object_hash`, so its body keeps a reference and survives garbage collection. A remote expunge therefore never destroys the local copy, which is what makes a store usable as a backup of a source it does not control. Retention is unconditional: whether a removal is terminal must read identically to every process that opens the store, so it is not configurable. How long to keep, and when to sweep, is the owner's schedule.

  `LOAD_ITEMS` hides retained rows from the sync seam. That is the condition of correctness, not an optimisation: io-replica's storage spec states that the merge reconciles only what `load` returns, so a hidden row is never re-derived, on a delta sync or on a full one. io-replica itself needed no change.

- **Purge, the only true delete.** `purge(collection, seq)` takes one retained item, `purge_retained_before(cutoff)` sweeps every item retired strictly before an RFC 3339 instant the caller computes from its own retention policy. Both release the row's object pin and let the ordinary refcount sweep unlink the body, and both refuse to touch a live item. `list_retained`, `count_retained` and `retained_bytes` are the trash view beside the live reads; `retained_bytes` is an upper bound on what a purge would reclaim, since a body a live item also points at survives.

- **A reappearing link id revives its retained row** (clearing `deleted`, `retained_at` and `retained_by`, adopting the new content, keeping the message's `seq`) instead of colliding on the primary key. One branch serves a source-side resurrection and a client restore alike, so restoring an item is an ordinary `Add` over the values retention preserved: no new action kind, no network.

- **An owner now skips the queue actions it cannot apply.** An unrecognised action kind decodes as `PimdirAction::Unknown { kind, payload, object_hash }`, payload verbatim and body still pinned, and the drain leaves the row pending (counted in `PimdirDrainReport.skipped`) instead of parking it, without blocking the actions behind it. Parking claims an action is permanently unappliable, which is wrong for an intent another owner can perform: this is what lets one queue carry store mutations any owner applies beside capability-bound intents such as a mail submission. Malformed payloads still park.

- `drop_action(id)` removes one queue row, pending or parked, releasing its object pin in the same transaction: one verb for cancelling a queued action and for acknowledging an intent performed out of band. `fail_action(id, error)` records a failed attempt, bumping `attempts` for a transient failure or parking with the reason for a permanent one.

- **The store persists a per-source content conflict.** `bindings` gained `conflicted` and `conflict_revision`, round-tripped through `ReplicaSourceBinding`, so the sync layer's memory of "this source and its own remote diverged, unresolved" survives a restart. Without it the merge re-derived the push its remote had already rejected on every run, never converging, and a client could not tell which items needed a human. Distinct from the item-level `conflicted` / `conflict_object`, which is the cross-source divergence; the two are persisted independently. Carried on `ReplicaSourceBinding` as of io-replica 0.3.0.

  The revision is meaningful only while conflicted (spec §11), so a resolved binding cannot hand a stale one to the next sync.

- A store written by an earlier draft of schema version 1 is now reconciled on open. The columns above were **folded into version 1** rather than added as version 2, the pimdir spec being still `draft`, so `PRAGMA user_version` stays `1`, which means an older store is not detectably out of date and would otherwise fail on a query much later. `init_schema` now adds any folded-in column it finds missing (and any index over one), guarded by `PRAGMA table_info` so it is a no-op for every store after the first open (spec §6's draft allowance). This machinery lapses when the spec freezes its first version.

### Changed

- **BREAKING**: bumped io-replica to `0.3`, whose `ReplicaSourceBinding` carries the per-source conflict this release persists.
- `PimdirAction::kind()` returns `&str` rather than `&'static str`, since an owner-defined kind is carried as data.

### Removed

- `PimdirActionError::UnknownKind`: an unrecognised action kind is no longer an error, so nothing can construct it.

- `sql::DELETE_ITEM`: a per-item hard delete has no caller and no counterpart in the format spec's canonical queries any more. An item no source holds is retained (`sql::RETAIN_ITEM`), and the only true deletes are `sql::PURGE_ITEM` and `sql::PURGE_RETAINED_BEFORE`.

## [0.1.0] - 2026-08-06

### Added

- Initial pimdir store: a SQLite index plus a content-addressed, two-level-sharded blob directory, implementing io-replica's storage seam (load, lookup_objects, write) for one source.
- no_std core reusable without the SQLite client: the canonical schema and statements (sql) and the model-to-column encodings (codec).
- Store-global public ids (seq): one per message, shared across every collection it is filed in, monotonic and never reused.
- Streaming blob ingest and read, so a large body is never held whole; a byteless object write indexes a body already streamed to its content-addressed path.
- Incremental, cross-collection-correct reference counting with blob garbage collection inside the write transaction; a crash leaves at worst an orphan blob, never a row without its body.
- Single-writer serialisation via BEGIN IMMEDIATE and a generous busy timeout, so several same-source handles overlap network while their writes serialise.
- An availability-aware, paginated client read surface (list_items, get_item, count_items, distinct_sources, seq_for_link) projecting the store as a local backend.
- The action queue table and collections.generation as part of the draft v1 schema, with user_version and store_meta.version kept in agreement and a store stamped with a higher schema version refused on open (the spec is a draft: draft stores are recreated, never migrated).
- The action queue (spec §14): PimdirProducer (the single enqueue transaction any non-owner process may run, pinning a pre-written body against garbage collection) and the owner's drain (drain_collection applies each action and deletes its row in one transaction, parking permanently failing actions), plus queued_collections, pending_actions (the read-your-writes overlay) and parked_actions.
- The action payload codec in the no_std core: PimdirAction (add, set-flags, remove, move, copy, update, addressing items by public seq) with a strict, versioned JSON round-trip.
- Collection generations (spec §15): the handle-space epoch on PimdirCollection and generation(), bumped atomically with a rebuild batch by write_rekeyed().
- Read-only store open (open_read_only): opens an existing store with SQLITE_OPEN_READ_ONLY, never creates anything, refuses any other schema version, and exposes the full read surface for frontend processes that must be unable to write.

[unreleased]: https://github.com/pimalaya/io-pimdir/compare/v0.3.0..HEAD
[0.3.0]: https://github.com/pimalaya/io-pimdir/compare/v0.2.0..v0.3.0
[0.2.0]: https://github.com/pimalaya/io-pimdir/compare/v0.1.0..v0.2.0
[0.1.0]: https://github.com/pimalaya/io-pimdir/compare/root..v0.1.0
