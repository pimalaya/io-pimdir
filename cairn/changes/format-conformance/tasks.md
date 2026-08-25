---
cairn: tasks
change: format-conformance
---

- [x] `items_retained` to `(collection, seq)`, with a reshape check on open
- [x] Test: an index whose columns moved is rebuilt rather than left alone
- [x] `tests/objects.rs` against `vectors/objects.json`, whole and streamed
- [x] `PimdirBlobs::path`, so the shard path is checkable and reachable
- [x] `every_canonical_statement_prepares`, verified by injecting a typo
- [x] Drop the stale "deliberate substitution" claims about `LIST_RETAINED_PAGE`
- [x] CI here and in pimdir, both asserting the suites did not skip
- [x] Restore the `Cargo.toml` note about the io-replica path dep
