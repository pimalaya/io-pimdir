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
- [x] Counts of system flags alone over a scope of kinds and accounts by one pass over `items_by_sort` when it costs less than the cheapest flag set (`PimdirPlan::scans_flags`): `is:unread` 126 to 69 ms (notmuch 13), `kind:mail is:unread` 201 to 54, `is:read` 167 to 60, `not is:unread` 462 to 57; a store mostly read keeps walking its few unread rows (log 2026-11-21-flag-counts-in-one-pass)
- [x] Text counts off the full-text index once it is current: one read transaction, no store seek per text hit, text terms alone combined as bitmaps per account; `enron` 506 to 107 ms, `california` 60 to 21, `power california` 125 to 44 (log 2026-11-21-text-counts-off-the-index). Left: the seek from a body to its placement, about 0.2 µs a matched row; FTS5's own `and` answers `power california` in 6 ms but is exact per hit only where a hit's placements name one document
- [x] Text counts match whole documents: the index records the public ids whose placements name two or more documents (`split`, pimdir SEARCH §3, search-draft-16 in place), a text-only count matches one FTS5 expression and evaluates those per placement; `power california` 43 to 13 ms (notmuch 5.5), `california not gas` 47 to 17 (6.4), none split on Enron, index +6.7 MB (log 2026-11-21-text-counts-match-whole-documents). Left: the seek from a body to its placement
- [x] System flags read from the store's marks (decided 2026-11-21, pimdir e2b7f49): the unread count at 518k from 572 to 116 ms, not the 45 expected, `is:flagged` from 28 to 45 (log 2026-11-21-system-flags-from-the-store)

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
