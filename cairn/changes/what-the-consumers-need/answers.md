# Answers to the app lanes

One line per request in <app>/cairn/changes/follows-storage-draft-17/needs-io-pimdir.md, answered by the io-pimdir lane. Each says accepted (the task and drop that carry it), declined (why) or deferred past 0.20.0. A drop's pushed hash is in drops.md once the owner has pushed it.

Written at planning (2026-11-23, about 15:50) from the four files that existed then, confirmed at C0 (16:52) against all six, calendula's and carillon's added. A line's id is the app's prefix and the number of its own needs file.

## himalaya

- HM1 Whether a writer folds a system flag's case into `items.marks`: deferred. It is a STORAGE §13 ruling for the owner (D13), not made mid-migration; sources send `\Seen` and nothing is lost.
- HM2 `pending_actions` lists a row it cannot decode: accepted, as CD1 (task 2.11, D12).
- HM3 R4, R1 and K4 fixed: accepted, drop 2 (tasks 2.1, 2.2, 2.10). R4 and R1 each get a pimdir vector. K4's check is an io-pimdir test (tests/drain_copy.rs, tests/drain_agrees.rs), because the vectors have no drain verb.
- HM4 The release with `client`, `search`, `mail`, `indexer` and `vendored` as at 7f16b27: accepted (tasks §5, §6). No feature changes; AN1 only adds `sql::search` under `indexer`.
- HM5 A `mid:` term driven by `summary.uid` rather than a scan: deferred to a 0.20.x. It is a query-path speed-up with a workaround (`envelope search mid:`), and no 0.20.0 fix touches the query path (D7).

## neverest

- NV1 R5, R6, R6b and K4 fixed: accepted, drop 2 (tasks 2.7 to 2.10). R6 (R6a) is a rule change, D10, whose default is to land the pending create as a page does, so the "no second APPEND" test can hold.
- NV2 An owner write registering a source: deferred. It is additive and the code passes at 7f16b27; it can land in a 0.20.x.
- NV3 The bound-placements and owed-body reads: deferred, as performance with a workaround.
- NV4 The release: accepted (tasks §5, §6).

## cardamum

- CD1 `pending_actions` lists a row it cannot decode (G1, STORAGE §15.4): accepted, drop 2 (task 2.11), in a shape that keeps the signature (D12). If only a breaking shape works, it is deferred.
- CD2 `#[non_exhaustive]` on `PimdirAction`: deferred to the 1.0 API review (pimdir the-freeze-plan tasks.md:67). Marking one enum now breaks every consumer's exhaustive match mid-migration, while your `SetProperty` arm already covers it.
- CD3 `list_summaries` 25 to 35% slower a row than 0.19.1: deferred to a 0.20.x. It is the cost of the one `summary` table and `summary_value` (storage-draft-17), correct and with no workaround needed; a faster read is a query-path change, out of 0.20.0 (D7).
- CD4 Counts of a collection's live and bodied items: deferred to a 0.20.x, additive (a new reader statement), with the walk as workaround.

## calendula

- CL1 `pending_actions` per row: accepted, as CD1 (task 2.11, D12).
- CL2 A calendar profile in spec/generator: deferred. spec/generator is test tooling outside the package, so it needs no release; it waits until after drop 3. calendula's bench/seed.rs stands meanwhile.
- CL3 A domain left out says so: accepted as documentation, drop 2 (with task 2.12): the `search` feature's doc and `PimdirIndex::item` and the snippet read state that a kind whose domain feature is off carries no snippet and reads its summary. A typed "domain not built" error is a behaviour change, out.

## carillon

- CR1 A change feed scoped to one collection: deferred. An index on `items(collection, changed)` is a storage schema change, and storage-draft-17 does not move in this migration.
- CR2 A reader holds a store moved aside: noted, no code. A rustdoc sentence on `PimdirReader` says a long-lived reader opens the store again to see one synced anew (drop 2, with task 2.12); STORAGE §8 wording is pimdir's, left to the owner.
- CR3 Generator events with a `VALARM`: deferred, as CL2.

## android

- AN1 `sql::search` under `indexer`: accepted, drop 1 (task 1.3). It blocks the app's release build.
- AN2 `DELIVER_BINDING` and `LIST_GARBAGE_OBJECTS`: accepted as a doc fix. pimdir cairn/changes/a-smaller-pimdir/consumers.md gains both (task 2.14).
- AN3 `DERIVATION` public outside `client`: accepted, drop 2 (task 2.12). It is additive and closes audit A5.
- AN4 A SQLite 3.18 plan check of io-pimdir's statements: deferred, none in 0.20.0. It stays owed, as the 2026-11-22 log says, and your own asserts remain the guard.
