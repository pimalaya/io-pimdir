---
cairn: tasks
change: as-fast-as-notmuch
---

## Phase 1, the bench

- [x] The Maildir connector and the import, index and query runs as an ignored test or example, the corpus fetched by a script, never vendored (bench/, 7847427)
- [x] The quiet-machine protocol: CPU, bytes written (`/proc/self/io`), wall; the eatmydata split; results table per tier (bench/README.md, scripts/summary.sh)
- [ ] The notmuch reference line recorded with its version

## Phase 2, queries

- [x] src/search/plan.rs in the I/O-free core: scope, term sets once, smallest-set driver, probes, count apart (performance-audit-2026-11-05 F1), 1098b16
- [x] A page in the store's order with probes for a broad driver, a keyset cursor; threads grouped for the page alone, 1098b16
- [x] Coverage once per query, cached per index handle on data_version (the per-collection stale set left); `COUNT_UNINDEXED` on `items_by_changed`; the temporal axis in one statement (F8), cac005d
- [ ] Every search vector green; the bench's query table against the gates

## Phase 3, writes and the index

- [ ] Prototype the-write-path-before-the-freeze 1 to 4, numbers to the owner
- [ ] Profile the index refresh (0.5 ms a message), then batch size, FTS5 merges, parallel extraction, summary text rewritten only when it changed (F6)
- [x] The no-op sync's fixed cost per collection: QPSG on every connection, no write for a delta that moved nothing, the bench connector skipping folders by mtime; 5.0 s to 0.49 s at 517k (log 2026-11-21-a-sync-that-finds-nothing); `list_owing_bindings` and `list_lacking_items` left to the owner
- [ ] The import gates

## Phase 4, the spec's additions

- [ ] an-item-keeps-local-state
- [x] the-query-reserves-its-later-forms (io-pimdir cebe50d, pimdir f22d990, log 2026-11-21)
- [ ] a-query-reads-across-domains, form by form

- [ ] Spec folded into cairn/spec/search.md and store.md, CHANGELOG, log entry per landed phase
