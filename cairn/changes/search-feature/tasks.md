---
cairn: tasks
change: search-feature
---

# Tasks

Deferred to the 2027 NLnet plan, Task 2 "pimdir Notmuch"; the plan is pimdir cairn/changes/search-milestones.

## M1, the index

- [x] `search` feature on `client`, with mail-parser 0.11 (`full_encoding`, as himalaya) as its optional dependency; vendored migrations/search and queries/search, tests/spec_drift.rs extended (ical and vcard join with M2's extraction)
- [x] FTS5 through `vendored`; a clear error when the system SQLite lacks it
- [x] Open index.db with pimdir.db attached read-only; rebuild on a version or tokenizer mismatch; `index.lock`
- [x] Refresh from the change feed, bounded batches, key reconciliation, cursor in the completing transaction
- [x] One-shot verbs refresh up to the cursor first, within a time bound, staleness reported in coverage; a session (REPL, MCP) follows the feed by calling `refresh` (the session itself is Task 1's)
- [x] Mail extraction through mail-parser, with chunk boundaries; `encrypted`, `unparseable`; summary text for bodiless placements
- [x] Query parser, exposed AST, compiler; an unknown field refused with a structured error; hits with placements, a snippet and an address, coverage
- [x] CLI `index`, `search`, `count`, `show`; cli.md amended from never-interprets to never-renders
- [x] `--json` for these verbs and their JSON Schema (hits, coverage, errors)

## M2, cross-domain

- [x] vCard and iCalendar extraction (vcard-rs 0.5, ical-rs 0.6); the role fields
- [x] `kind:`, `with:` (answered since M1, `with:` from the summaries' address rows), `person:`, `linked:`
- [x] Occurrences within the horizon; the roll; the per-item bound; the tzdb version; `horizon_left` in coverage

## M3, threads and dates

- [x] `message` and `thread` tables, `thread:`, threaded listing (`PimdirIndex::threads`, `search --threads`, `pimdir-search-threads`); an index built before threads rebuilt through this crate's `index_build`
- [x] Relative dates (`today`, `2w`, ranges) against `PimdirSearchClock`; `unsupported-field` removed, every field answered

## M4, this crate's part

- [x] CLI `tag` through the queue (`PimdirIndex::tag`, `PimdirSearchTagPlan::stage`, `--dry-run`); `thread` (`PimdirIndex::thread`); their `--json` and JSON Schema (`pimdir-tag`, `pimdir-thread`)

## Coverage

- [x] A `PimdirSearchCoverage` with the local, temporal and remote axes, the conservative roll-up and `is_complete()`
- [x] Filled by every search answer and by the store's `LIKE` reads (through `PimdirReader::search_coverage`) (temporal axis from the coverage rows); those reads are kept for consumers without the feature
- [ ] pimdir: SEARCH §8.2 states the temporal axis beside the bodiless count; a vector with a range below a collection's coverage
- [ ] Consumers: himalaya and android cairn/changes/a-search-says-what-it-covered

## Conformance and landing

- [x] tests/search.rs over vectors/search (every case, none skipped)
- [x] Spec: a search capability file (cairn/spec/search.md); cli.md modified
- [ ] Fold, log, CHANGELOG (M1 logged 2026-10-27-search-index-m1; again at each milestone)
- [ ] Undocumented extensions (from a-query-only-index): state each in search.md or drop it; the 16 own statements under src/sql/search.rs upstreamed where pimdir takes them
- [ ] Typed `kind` and status outputs (`PimdirSearchHit::kind: String`, the CLI's `Status(String)`), if the API pass agrees
