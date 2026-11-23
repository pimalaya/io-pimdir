---
cairn: delta
change: the-store-version-contract
---

## MODIFIED Requirements

### Requirement: Schema version
The store schema is version 1 (`user_version` 1), migrations/storage/0001_init.sql verbatim. build.rs SHALL generate `sql::MIGRATIONS`, every canonical migration in file order, and the owner open SHALL run STORAGE §6's steps in order, nothing else changing the schema:

1. Read `user_version`. A store above `sql::VERSION` SHALL be refused as `PimdirError::Version { found }` (code `store-newer`, its message saying to upgrade this program) before anything is written, the journal mode included. A database at version 0 that declares any table is not a pimdir store and SHALL be refused as `PimdirError::Foreign` (code `store-foreign`), as SHALL one whose `store_meta.format` is not `pimdir`. A store lacking a core table or trigger is refused as `PimdirError::Stale` by every role, the table scan running before `store_meta` is read. The declared algorithm (`open_with_hash`) is checked here, so a mismatch is `PimdirError::HashAlgo` with nothing reconciled.
2. A version 1 store is reconciled to 0001_init.sql alone ("Open reconciles against the canonical text"), before any migration; no other version is reconciled.
3. Each migration above the store's version runs in its own transaction, which also sets `user_version` and, through the canonical `set_store_version`, `store_meta.version`; a fresh database runs `init_store_meta` (`:version`, `:hash_algo`, `:derivation`) in the transaction applying 0001_init.sql.

`PRAGMA journal_mode = WAL` is set only after step 1 passed, by the owner, `open_current` and the producer alike, since it writes the database header.

### Requirement: The two schema stamps must agree
`PRAGMA user_version` and `store_meta.version` mirror one another (STORAGE §4.2), so a store where they differ is corrupt rather than a store at either version. Every role SHALL compare `store_meta.version` with the store's own `user_version`, not with this crate's version, and refuse a disagreement with `PimdirError::VersionMismatch`.

A store carrying no `store_meta` row SHALL be left alone: the row is seeded by whoever created the schema, and refusing there would make a missing stamp unrepairable.

### Requirement: Open reconciles against the canonical text
Opening a version 1 store as its owner SHALL compare every table, index and trigger with the one 0001_init.sql alone creates in an empty in-memory database, by its `sqlite_schema` text with comments dropped and whitespace collapsed (STORAGE §6). A store that differs SHALL be reconciled in one transaction, foreign keys off:

1. The data a new constraint would refuse is repaired first: `repair_shared_objects` and `recompute_refcounts` (once `bindings` holds `shared_object`, else after it is rebuilt), `repair_levels`, `repair_kinds`.
2. A table that differs is rebuilt from the canonical text (its indexes and triggers dropped, renamed aside with `legacy_alter_table` on, created, its shared columns copied, its `AUTOINCREMENT` counter carried, the old one dropped).
3. What is missing is created, what differs recreated, what the canonical schema lacks dropped.
4. The columns a rebuild added are backfilled with the canonical statements: `backfill_shared_object` then `repair_shared_objects`, `backfill_invitation` over the held bodies by `list_held_mail` then `link_invitations_of`, `backfill_task_organisation` over the held bodies by `list_held_tasks` then the four task rules with a `NULL` link id; then `recompute_refcounts` again, and no foreign key left dangling.

A current store SHALL be left untouched.

#### Scenario: An older shape reconciled
- tests/files.rs `an_earlier_role_constraint_is_rebuilt_on_open` (the rebuilt text equals a fresh store's), tests/scoped_sync.rs `an_earlier_draft_store_is_reconciled_on_open` (a level 0 row repaired to `Meta`), tests/linking.rs `an_earlier_store_backfills_the_invitation_on_open`, tests/references.rs `an_earlier_store_gains_the_references_on_open`.

### Requirement: An owner open that reconciles nothing
`PimdirStore::open_current(dir)` SHALL open the existing store as its owner, taking the same lock as `open` (`PimdirError::Owned` at once when another process holds it), and SHALL create, migrate and reconcile nothing. It SHALL refuse, writing nothing, as `PimdirError::NotCurrent` with `PimdirNotCurrent::Absent` when no pimdir.db is under `dir`, `PimdirNotCurrent::Version { found }` when `user_version` is not this crate's, and `PimdirNotCurrent::Shape { object }` naming the first table, index or trigger whose text differs from the canonical schema or that one side lacks, which `open` would reconcile. A damaged or foreign store is refused as `open` refuses it (`Stale`, `VersionMismatch`, `Foreign`, `HashAlgo`).

#### Scenario: a store another revision reconciled
- GIVEN a store whose `task_summary` lacks the task organisation columns
- WHEN `open_current` opens it
- THEN it answers `NotCurrent(Shape { object: "task_summary" })` and the table is left as it was; `open` then reconciles it and `open_current` opens it (tests/open_current.rs)

## ADDED Requirements

### Requirement: A store this crate may not write is never written
A store above this crate's version, a foreign database, and a store whose algorithm is not the one declared SHALL be refused by the owner open before any byte of pimdir.db changes; the reader and the producer SHALL refuse the first two as well.

#### Scenario: A newer store in rollback-journal mode
- GIVEN a released store stamped one version above this crate's, in rollback-journal mode, holding a table this crate does not know
- WHEN the owner, a reader, a producer and `open_current` open it
- THEN each is refused (`Version`, `store-newer`) and pimdir.db is byte for byte as it was (tests/store_version.rs `a_store_above_this_version_is_refused_by_every_role_and_never_written`; also `a_foreign_database_is_refused_and_left_as_it_was`, `a_declared_algorithm_is_checked_before_anything_is_reconciled`)

### Requirement: A reader or a producer reads an older store as it stands
A reader or a producer meeting a store below this crate's version, or a version 1 store its owner has not reconciled yet, SHALL read it: a column it lacks as `NULL`, a table it lacks as empty, running no statement naming either (STORAGE §6). The summary loads read `mail_summary.invitation` and the task organisation columns through `schema::summary_load_sql`; the collection, coverage, reference and file reads keep their own guards.

### Requirement: A released store opens whole
A store io-pimdir 0.5.1, 0.7.0 or 0.8.0 wrote SHALL open as owner with every row of every table kept: collections (one of a media type outside today's closed set, recorded kindless), items (a retained one included), bindings, objects, receipts, references, and the queue, its parked row still parked and its pending rows applying at the next drain; the invitation and the task organisation are re-derived from the held bodies, the refcounts settle with no drift, and the next `open_current` finds it current.

#### Scenario: The released fixtures
- tests/fixtures/store-v0.5.1, store-v0.7.0 and store-v0.8.0, written by those releases with tests/fixtures/store_gen.rs, read by a reader and a producer before the owner opens them (tests/store_version.rs `a_store_released_by_*_opens_without_losing_a_row`)

### Requirement: A migration runs after the reconciliation and keeps every row
A migration above version 1 SHALL run after the reconciliation of a version 1 store, so a script assuming 0001's canonical shape applies; it SHALL move both stamps to its number and keep every row of every table. A store at version 2 or above is never reconciled to 0001.

#### Scenario: A synthetic 0002
- GIVEN a version 1 store missing an index an earlier draft lacked, and a 0002 that drops that index, adds a column and a table
- WHEN the runner opens it with both migrations
- THEN both stamps read 2, every table keeps its row count, a second open drops nothing it added, and this crate (version 1) refuses it as `Version { found: 2 }` (src/client/schema.rs `migration::a_migration_runs_after_the_reconciliation_and_keeps_every_row`)

### Requirement: The store records the derivation revision
The store SHALL record in `store_meta.derivation` the Annex A revision its summaries, addresses and sort keys were derived under (STORAGE §6): `init_store_meta` writes this crate's (`schema::DERIVATION`, revision 1), a reconciliation gives an existing store the column's default 1, and an owner at a later revision re-derives what changed then records it with `set_derivation`, which never lowers it. Raising `DERIVATION` owes that re-derivation.
