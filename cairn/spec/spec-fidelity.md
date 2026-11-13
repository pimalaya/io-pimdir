---
cairn: spec
capability: spec-fidelity
status: current
---

# Spec fidelity

The `sql` module is generated at build time from spec/, a byte-for-byte copy of the pimdir specification's migrations/ and queries/ at a pinned revision, so a consumer holding its own SQLite binding runs the format's own statements by name and nothing is transcribed. The suites proving the crate against the format read the vectors from the same copy. The spec lives in a separate repository, so nothing about the build notices when one moves and the other does not. This capability is what notices.

### Requirement: The vendored copy is the specification's, byte for byte
spec/migrations/, spec/queries/, spec/vectors/ and spec/payloads/ SHALL be identical to the specification's directories of the same name at the revision spec/REVISION names (its full commit id, one line; pimdir c834ee1 at storage-draft-10, sync-draft-10 and search-draft-10, the fourth and last batch of those drafts, the search batch, master, untagged), file by file and both ways. tests/spec_drift.rs SHALL compare them with that revision read from the git history of a pimdir checkout beside this one, whatever branch its working tree is on, and SHALL check spec/vectors/ and spec/payloads/ against spec/vectors/manifest.json without any checkout: every listed file present under its SHA-256, none present and unlisted (but the manifest and the README), and each case file holding the number of cases the manifest counts, the sync cases included. Re-vendoring copies the four directories at one revision, deleting a file the revision lacks, and writes it to spec/REVISION; nothing under spec/ is edited here. queries/signatures.json, the specification's listing of every statement's parameters, list slots and answered columns, is vendored with queries/ and is no statement: build.rs reads the profile directories of queries/storage/ and queries/search/ alone. The package leaves spec/vectors/ and spec/payloads/ out. The drafts the copy carries are named in README.md and CHANGELOG.md, and a re-vendoring from later drafts renames them there.

#### Scenario: The sibling checkout is on another revision
- GIVEN a pimdir checkout beside this one whose working tree is on a revision other than spec/REVISION
- WHEN tests/spec_drift.rs runs
- THEN it compares spec/ with the files at spec/REVISION, not with the working tree, and passes while the copy is exact

#### Scenario: A vector edited by hand
- tests/spec_drift.rs `the_vendored_vectors_are_the_manifests`, whatever sits beside this checkout

### Requirement: The constants are generated, never written
build.rs SHALL emit one constant per statement file, named after the file in upper case, documented by the file's leading comment, plus one per migration, `MIGRATIONS` listing them in order and `VERSION` as their count, and `sql::CANONICAL` indexing the statements. `sql::OWN` indexes this crate's own statements, and `sql::all()` both. A canonical statement is therefore never substituted: the specification's text is the constant's.

### Requirement: A statement of the crate's own serves no profile
`sql::OWN` SHALL hold only what the operator tool asks that no profile of the standard needs: the consistency diagnostics behind `pimdir check` (`LIST_OBJECT_HASHES` for the orphan-file diff among them) and the figures behind `store info` (`OBJECT_SIZE`, `COUNT_RETAINED_BEFORE`), and the one repair the diagnostics justify, `DELETE_DANGLING_BINDINGS`. A statement a reader, a producer or an owner needs to meet the standard is upstreamed to the specification, never kept here: `collection_sources` went up on 2026-09-05, the day it was written.

#### Scenario: A statement is added upstream
- GIVEN a new file under the specification's queries/storage/
- WHEN the two directories are copied over
- THEN the next build carries its constant, with no Rust edit

### Requirement: No canonical statement is silently absent
Every statement file under the specification's queries/storage/ profile directories (read/, queue/, owner/) SHALL have a constant reachable through `sql::all()`.

### Requirement: Every statement prepares against the schema
Every constant `sql::all()` yields SHALL prepare against a database created from `sql::MIGRATION_0001`, and every statement the vendored copy carries SHALL prepare against the vendored migration, a file holding several (`optimize_text`) statement by statement. The specification repository holds no toolchain, so this is the only place its SQL is ever loaded.

### Requirement: The vectors are read from the vendored copy
Every suite reading the specification's files (tests/spec_drift.rs, tests/objects.rs, tests/summaries.rs, tests/payloads.rs, tests/vectors_sync.rs, tests/search.rs) SHALL read them from spec/, and none SHALL skip: a vendored file missing fails the suite reading it. A runner SHALL check every case of its file, a failure being the case's own, and settle them against tests/pending_vectors.txt (tests/pending/mod.rs): a case listed there with the phase of the-crate-follows-storage-draft-08 fixing it may fail; a listed case passing, an unlisted one failing and a listed one its suite does not hold each fail the suite, so the list only shrinks, and a release needs it empty (decision 3). A case the crate believes wrong in the standard is listed too, with its reason, never worked around: the number before it is then the standard's batch expected to restate it. tests/payloads.rs judges each payload case by its `outcome`, valid, malformed or unknown (`codec::payload_outcome`), the file at format `v` 2, and tests/vectors_sync.rs reads the sync cases at `v` 5 (a `SetFlags`'s `base`, a `Remove`'s `if_match` and `resolves`, the `Disagreed` event). Only the comparison with the specification needs a pimdir checkout; without one holding the pinned revision beside this one (a consumer building from the registry has no reason to hold it) it compares nothing and prints one line saying so, never a failure.

#### Scenario: No pimdir checkout
- GIVEN no pimdir checkout beside this one
- WHEN the suites run
- THEN every vector suite runs against spec/vectors/, and tests/spec_drift.rs alone reports that it compared nothing

#### Scenario: The pending list is empty
- tests/pending_vectors.txt named no case from phase 6 to pimdir 0cfa8a1. At e4faa21 the list named one case the standard had to restate, sync case 48, whose store stated no mail summary for the row its expectation had hold a bodiless message elsewhere. At ebd60b6, which restates it, every case passes and the list is empty; at c834ee1 it stays so, over the 13 objects cases, the 71 summaries cases, the 73 payloads, the 212 queries at format `v` 4 on the store at `v` 2, and the 154 sync cases at format `v` 6. A case goes back on it only with its reason, and leaves when a re-vendored case passes

#### Scenario: An unlisted failure
- GIVEN a scratch copy of the tree whose objects runner fails one case
- WHEN tests/objects.rs runs
- THEN the case's panic is caught, the next cases run, and the settle fails the suite naming the case (proved at phase 7 of the-crate-follows-storage-draft-08)

### Requirement: The crate states its conformance
README.md SHALL state the crate's conformance as STORAGE §17, SYNC §12 and SEARCH §12 ask, each part's version and the profile met: `storage-draft-10 owner, sync-draft-10 engine, search-draft-10 indexer (canonical) and query client`. The owner holds the reader's and the producer's profiles, each a handle of its own; the crate is no connector, which a consumer writes. Its departure from a MUST is listed there with it (STORAGE §18's access list on Windows).

### Requirement: Object naming is checked against the format's vectors
The crate SHALL check its object names against the specification's vectors/objects.json as vendored (STORAGE §16): every body under BLAKE3, the one algorithm, fed whole and fed in pieces, together with the shard path each name derives, one shard level (`objects/<hash[0:2]>/<hash>`). A store whose two writers name the same body differently reports nothing: it silently never deduplicates and never finds the blob the other side wrote. `PimdirBlobs::path` is public for the same reason, STORAGE §14 inviting a consumer to stream a body straight to it; it checks the name first and answers `InvalidInput` for a malformed one.
