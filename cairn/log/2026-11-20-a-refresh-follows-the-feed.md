---
cairn: log
change: the-crate-follows-draft-15
landed: 2026-11-20
---

# A refresh follows the feed

pimdir's vectors/search/refresh.json (pimdir cairn/log/2026-11-20-a-refresh-follows-the-feed.md, search-draft-15) re-vendored under spec/vectors with its schema and the restated manifest.

- tests/search.rs `every_refresh_vector_answers` runs every step: the cursor read, the write through the owner, the feed above the cursor against `changed`, a refresh, then each query's hits as a set on the refreshed index and on one built from a backup of the store. `build` became `build_store` over any store of store.json's shape, its item writing `put`, shared with the runner.
- tests/pending/mod.rs and tests/pending_vectors.txt name the `refresh` suite; it lists no case.
- The proof: with `stamp_item` under an unchanged item disabled in src/client/write.rs, step 4 (a bodiless summary changed) fails on the feed ("the items the feed lists"), and with the feed check skipped, on the refreshed index ("\"subject:final\": refreshed"). No code of the crate moved: it already followed STORAGE §4.5.

Capabilities moved: search.
