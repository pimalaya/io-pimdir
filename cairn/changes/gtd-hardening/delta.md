---
cairn: delta
change: gtd-hardening
---

# Delta

## ADDED Requirements

### Requirement: A reader reads the task columns an owner has not reconciled as NULL

`PimdirReader` SHALL probe `task_summary` for `priority`, `categories`, `parent` and `mid` before every statement naming them (`list_tasks_page_asc`, `get_task`, `load_task_summaries`) and read each one the store lacks as `NULL`, so a listing on a store an owner at io-pimdir 3b4bed8 reconciled never fails "no such column" (STORAGE §6). The query client SHALL do the same on the attached store: `priority:` reads such a task as of no priority, `category:` as carrying none, and an item read from its summary reads the columns `NULL`.

#### Scenario: the 3b4bed8 shape lists
- GIVEN a store whose `task_summary` has the 3b4bed8 shape, holding a task stating priority 1 and `CATEGORIES:inbox,home`
- WHEN a reader lists the collection, reads the item, lists the trash, and a query client asks `category:inbox` and `priority:none`
- THEN every read answers, the task with no priority and no category, `category:inbox` matching nothing and `priority:none` matching it; once the owner opens the store, the columns are backfilled and read (tests/older_task_summary.rs)

### Requirement: An owner open that reconciles nothing

`PimdirStore::open_current(dir)` SHALL open the existing store as its owner, taking the same lock as `open` (`PimdirError::Owned` at once when another process holds it), and SHALL create, migrate and reconcile nothing. It SHALL refuse, writing nothing, as `PimdirError::NotCurrent` with `PimdirNotCurrent::Absent` when no pimdir.db is under `dir`, `PimdirNotCurrent::Version { found }` when `user_version` is not this crate's, and `PimdirNotCurrent::Shape { object }` naming the first table, index or trigger whose text differs from the canonical schema or that one side lacks, which `open` would reconcile. A damaged store is refused as `open` refuses it (`Stale`, `VersionMismatch`, `HashAlgo`).

#### Scenario: a store another revision reconciled
- GIVEN a store whose `task_summary` lacks the task organisation columns
- WHEN `open_current` opens it
- THEN it answers `NotCurrent(Shape { object: "task_summary" })` and the table is left as it was; `open` then reconciles it and `open_current` opens it (tests/open_current.rs)

### Requirement: An add superseding an item

`PimdirAction::Add` SHALL carry `supersedes: Option<PimdirSupersedes>`, `PimdirSupersedes { collection, seq }`, encoded as the payload's `supersedes: { "collection", "seq" }` (STORAGE §15.3) and decoded strictly (a malformed one parks). The drain SHALL stage the add, parking it for a live duplicate link id before anything else, then in the same transaction tombstone the superseded item as a `remove` would and record the pair (`supersede_item`); a superseded item absent applies the add alone, and one no source binds is retained. The gate SHALL run the add against its collection and a `remove` of the superseded item against the sources binding it, as producer and as the owner's backstop.

#### Scenario: a parked add leaves the source in place
- GIVEN task 1 live in Inbox and a queued add into Next carrying `supersedes: {Inbox, 1}`, Next holding a live item under the add's link id
- WHEN the owner drains
- THEN the row is parked, task 1 is still live in Inbox and no supersession is recorded (tests/engine/supersedes.rs)

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

### Requirement: Collections refusing tasks

`capability::X_VTODO` SHALL name `x-vtodo` (STORAGE §15.6). The gate SHALL refuse with `PimdirError::Unsupported` naming `x-vtodo` an `add` or `update` whose new resource is a VTODO, and a `move` or `copy` of one, into a collection where a source concerned states `x-vtodo` `none` there or source-wide; a source stating no row refuses nothing by it. `PimdirReader::accepts_vtodo(collection)` SHALL answer whether no source syncing the collection states `none` there, for a client choosing where tasks go.

#### Scenario: an events-only calendar
- GIVEN source caldav declaring `x-vtodo` `none` on Events
- WHEN a producer enqueues an add of a VTODO into Events
- THEN it is refused naming `x-vtodo` and caldav, a VEVENT is queued, `accepts_vtodo("Events")` is false and true elsewhere (tests/capabilities.rs)

### Requirement: Which message ids a mail replies to

`PimdirReader::replied(message_ids)` SHALL answer which of the given ids some live mail replies to (`list_replied`), mail under the `sent` or `drafts` role aside: each id trimmed and stripped of one pair of enclosing angle brackets, an empty one dropped, compared bare and exactly, answered once each in code point order; `collections.role` missing reads as `NULL`.

#### Scenario: a reply and the person's own
- GIVEN INBOX holding a mail replying to asked@example.org and Sent (role `sent`) one replying to mine@example.org
- WHEN a reader asks for `<asked@example.org>`, ` mine@example.org `, never@example.org, asked@example.org and an empty id
- THEN it answers asked@example.org alone (tests/replied.rs)

## MODIFIED Requirements

### Requirement: Spec fidelity

spec/ SHALL be re-vendored from pimdir's gtd-hardening change: `supersessions` in 0001_init.sql, `supersede_item.sql`, `list_superseded_pending.sql` and the new `rename_queue_targets.sql`.

## REMOVED Requirements
