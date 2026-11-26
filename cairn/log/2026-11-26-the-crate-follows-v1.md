---
cairn: log
change: the-crate-follows-v1
landed: 2026-11-26
---

# The crate follows v1

spec/ is pimdir at the freeze commit spec/REVISION names, storage-v1.0.0, sync-v1.0.0 and search-v1.0.0, released together from storage-draft-19, sync-draft-16 and search-draft-20 with no rule moved. The crate is 1.0.0, rehearsed on a scratch copy of both trees before landing (pimdir cairn/changes/the-freeze-plan, checklist.md step 3).

## What moved

- **The names the crate records.** `sql::FORMAT_VERSION` `storage-v1`, `sync::SYNC_VERSION` `sync-v1`, `sql::search::FORMAT_VERSION` `search-v1`, written by init_store_meta.sql and init_index_meta.sql; `sql::search::FORMAT_DRAFT` became `FORMAT_MAJOR`, the index's versions ordered as SEARCH §2 orders them, drafts below releases (store.md, search.md; tests/store_version.rs, tests/search.rs, tests/indexer_alone.rs).
- **The open contract judges as a release.** `schema::check` asks `PimdirStoreVersion::opens` as `PimdirFormat::Release`, each refusal mapped to the error naming its cause (`Foreign`, `VersionMismatch`, `HashAlgo`, else `Version`), and a store above `sql::VERSION` is judged by `store_meta` alone: a reader and a producer open it while their floors allow, an owner refuses it, its schema's text unchecked (STORAGE §4.2). The rehearsal found the draft-era check refusing such a store as `Damaged` and every refusal collapsed into `Version`; tests/store_version.rs `a_later_store_opens_for_a_reader_and_a_producer_its_floors_allow` and tests/queue.rs `a_later_store_is_refused_by_an_owner_alone` fail without the fix (store.md "A store of another version is refused").
- **README.md's conformance** is `storage-v1.0.0 owner, sync-v1.0.0 engine, search-v1.0.0 indexer (canonical) and query client` (spec-fidelity.md).
- **CI's Tests workflow** runs in parallel jobs: the suite against pimdir, the generator with the system link, the no_std core and the consumer sets, and one clippy job per feature set, where one job ran all of them in a row (19 to 24 minutes).

## Left

- `PimdirFormat::Draft` is run by no open since 1.0.0 and kept for the judgement's tests; it is crate-internal, so removing it moves no API.
