---
cairn: spec
capability: spec-fidelity
status: current
---

# Spec fidelity

spec/ holds two things. The pimdir specification's store schema, vectors and payload schemas (spec/migrations/storage/, spec/vectors/, spec/payloads/), a byte-for-byte copy at a pinned revision: the tables and columns of that schema are the format's contract, and the vectors and payloads what a conforming implementation answers. And this crate's own SQL and tooling, which the specification no longer binds: the statements (spec/queries/), the index schema (spec/migrations/search/) and the test generator (spec/generator/). The `sql` module is generated at build time from both, so a consumer holding its own SQLite binding runs the crate's statements by name over the format's schema and nothing is transcribed. The suites proving the crate against the format read the vectors from the same copy. The spec lives in a separate repository, so nothing about the build notices when one moves and the other does not. This capability is what notices.

### Requirement: The vendored copy is the specification's, byte for byte
spec/migrations/storage/, spec/vectors/ and spec/payloads/ SHALL be identical to the specification's directories of the same name at the revision spec/REVISION names (its full commit id, one line; at 1.0.0 the commit releasing storage-v1.0.0, sync-v1.0.0 and search-v1.0.0), file by file and both ways (`VENDORED` in tests/spec_drift.rs). tests/spec_drift.rs SHALL compare them with that revision read from the git history of a pimdir checkout beside this one, whatever branch its working tree is on, and SHALL check spec/vectors/ and spec/payloads/ against spec/vectors/manifest.json without any checkout: every listed file present under its SHA-256, none present and unlisted (but the manifest and the README), and each case file holding the number of cases the manifest counts, the sync cases included. Re-vendoring copies the three directories at one revision, deleting a file the revision lacks, and writes it to spec/REVISION; nothing under them is edited here.

### Requirement: The statements, the index schema and the generator are the crate's own
spec/queries/ (one file per statement under queries/storage/read/, queue/ and owner/, and queries/search/), spec/migrations/search/0001_init.sql and spec/generator/ SHALL be edited here, as the crate's code is, and are compared with nothing upstream: pimdir binds no statement, no index layout and no generator since pimdir 692af79 (its cairn/changes/a-smaller-pimdir, D5). A statement follows the store schema it reads, so a change to the vendored migration lands with the statements it moves, in one commit. queries/signatures.json, the listing of every statement's parameters, list slots and answered columns, sits beside the profile directories and is no statement: build.rs reads the profile directories of queries/storage/ and queries/search/ alone. The package leaves spec/generator/, spec/vectors/ and spec/payloads/ out; spec/generator/ is the crate's test tooling, the path dev-dependency `pimdir-generator` the generated tests (tests/generated/) run. The drafts the copy carries are named in README.md and CHANGELOG.md, and a re-vendoring from later drafts renames them there.

#### Scenario: The sibling checkout is on another revision
- GIVEN a pimdir checkout beside this one whose working tree is on a revision other than spec/REVISION
- WHEN tests/spec_drift.rs runs
- THEN it compares spec/ with the files at spec/REVISION, not with the working tree, and passes while the copy is exact

#### Scenario: A vector edited by hand
- tests/spec_drift.rs `the_vendored_vectors_are_the_manifests`, whatever sits beside this checkout

### Requirement: The constants are generated, never written
build.rs SHALL emit one constant per statement file, named after the file in upper case, documented by the file's leading comment, plus one per migration, `MIGRATIONS` listing them in order and `VERSION` as their count, and `sql::CANONICAL` indexing the statements. `sql::OWN` indexes the statements written inline in src/sql.rs, and `sql::all()` both. A statement file is therefore never substituted: the file's text is the constant's.

### Requirement: An inline statement serves no profile
`sql::OWN` SHALL hold only what the operator tool asks that no profile of the standard needs: the consistency diagnostics behind `pimdir check` (`LIST_OBJECT_HASHES` for the orphan-file diff among them) and the figures behind `store info` (`OBJECT_SIZE`, `COUNT_RETAINED_BEFORE`), and the one repair the diagnostics justify, `DELETE_DANGLING_BINDINGS`. A statement a reader, a producer or an owner runs is a file under spec/queries/storage/, never an inline constant, so a consumer binding `io_pimdir::sql::NAME` (the Android app, neverest's `LOAD_CHECKPOINT` and `UPSERT_CHECKPOINT`) finds every one in `sql::CANONICAL`.

#### Scenario: A statement is added
- GIVEN a new file under spec/queries/storage/
- WHEN the crate builds
- THEN it carries the file's constant, with no Rust edit

### Requirement: No statement file is silently absent
Every statement file under spec/queries/storage/'s profile directories (read/, queue/, owner/), and with `search` under spec/queries/search/, SHALL have a constant reachable through `sql::all()` (`sql::search` for the second).

### Requirement: Every statement prepares against the schema
Every constant `sql::all()` yields SHALL prepare against a database created from `sql::MIGRATION_0001`, every statement file under spec/queries/storage/ against the vendored migration, and with `search` every search statement against an index created from spec/migrations/search/ with the store attached, a file holding several (`optimize_text`) statement by statement. pimdir's own checks load the store schema alone (checks/schema.sh, checks/floor.sh), so this is the only place a statement is ever prepared.

### Requirement: The vectors are read from the vendored copy
Every suite reading the specification's files (tests/spec_drift.rs, tests/objects.rs, tests/summaries.rs, tests/payloads.rs, tests/vectors_sync.rs, tests/search.rs) SHALL read them from spec/, and none SHALL skip: a vendored file missing fails the suite reading it. A runner SHALL check every case of its file, a failure being the case's own, and settle them against tests/pending_vectors.txt (tests/pending/mod.rs): a case listed there with the phase of the-crate-follows-storage-draft-08 fixing it may fail; a listed case passing, an unlisted one failing and a listed one its suite does not hold each fail the suite, so the list only shrinks, and a release needs it empty (decision 3). A case the crate believes wrong in the standard is listed too, with its reason, never worked around: the number before it is then the standard's batch expected to restate it. tests/payloads.rs judges each payload case by its `outcome`, valid, malformed or unknown (`codec::payload_outcome`), the file at format `v` 2, and hands the codec each payload as the file writes it, never as a parsed value printed again: an integer `v` is written with neither a fraction nor an exponent (STORAGE §15.3), and `1e0` read into a value prints `1.0`, another spelling than its case states. tests/vectors_sync.rs reads the sync cases at `v` 6 (a `SetFlags`'s `base`, a `Remove`'s `if_match` and `resolves`, the `Disagreed` event, an `Abandon`), seeding the references a case's store states. Only the comparison with the specification needs a pimdir checkout; without one holding the pinned revision beside this one (a consumer building from the registry has no reason to hold it) it compares nothing and prints one line saying so, never a failure.

#### Scenario: No pimdir checkout
- GIVEN no pimdir checkout beside this one
- WHEN the suites run
- THEN every vector suite runs against spec/vectors/, and tests/spec_drift.rs alone reports that it compared nothing

#### Scenario: The pending list is empty
- tests/pending_vectors.txt named no case from phase 6 to pimdir 0cfa8a1. At e4faa21 the list named one case the standard had to restate, sync case 48, whose store stated no mail summary for the row its expectation had hold a bodiless message elsewhere. At ebd60b6, which restates it, every case passes and the list is empty; at c834ee1 it stays so, over the 13 objects cases, the 71 summaries cases, the 73 payloads, the 212 queries at format `v` 4 on the store at `v` 2, and the 154 sync cases at format `v` 6. At d8800fb, the drafts 11, it stays empty over the 13 objects cases, the 73 summaries cases, the 75 payloads, the 220 queries and the 158 sync cases, no format moving. At e443a65 it stays empty, sync case 159 the one case added. At daec31a it names, with their reason, the 30 `label:` and `property:` cases of queries.json and the 7 steps of refresh.json's "an item keeps local state", which wait on the query planner (as-fast-as-notmuch, phase 2), the 102 payloads passing. At 8b1523d, objects keyed by an integer, it is empty, no case moving. At e2b7f49, system flags read from the store, it stays empty, refresh.json at `v` 4 adding a case of three steps, two of them lagging. At f295ed7, storage-draft-17 and search-draft-17, it stays empty, refresh.json at `v` 6 pinning answers alone. A case goes back on it only with its reason, and leaves when a re-vendored case passes

#### Scenario: An unlisted failure
- GIVEN a scratch copy of the tree whose objects runner fails one case
- WHEN tests/objects.rs runs
- THEN the case's panic is caught, the next cases run, and the settle fails the suite naming the case (proved at phase 7 of the-crate-follows-storage-draft-08)

### Requirement: The crate states its conformance
README.md SHALL state the crate's conformance as STORAGE §17, SYNC §12 and SEARCH §12 ask, each part's version and the profile met: `storage-v1.0.0 owner, sync-v1.0.0 engine, search-v1.0.0 indexer (canonical) and query client`. The owner holds the reader's and the producer's profiles, each a handle of its own; the crate is no connector, which a consumer writes. Its departure from a MUST is listed there with it (STORAGE §18's access list on Windows).

### Requirement: Object naming is checked against the format's vectors
The crate SHALL check its object names against the specification's vectors/objects.json as vendored (STORAGE §16): every body under BLAKE3, the one algorithm, fed whole and fed in pieces, together with the shard path each name derives, one shard level (`objects/<hash[0:2]>/<hash>`). A store whose two writers name the same body differently reports nothing: it silently never deduplicates and never finds the blob the other side wrote. `PimdirBlobs::path` is public for the same reason, STORAGE §14 inviting a consumer to stream a body straight to it; it checks the name first and answers `InvalidInput` for a malformed one.
