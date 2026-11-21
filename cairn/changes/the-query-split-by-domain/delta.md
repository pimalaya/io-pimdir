---
cairn: delta
change: the-query-split-by-domain
---

# Delta

## ADDED Requirements

- The grammar and coverage compile with no feature.
- Features `mail`, `contact`, `calendar`, each gating its extraction and matcher; `search::mail`.
- The `indexer` feature: the I/O-free refresh over the three domains, no `client`; with `search`, `PimdirIndex::open`, `refresh`, `erase`.
- `PimdirQueryPredicate::name()`.
- The SEARCH additions: `header:`, instants, `min`/`h`/`mo`, `unserved-field`, sort direction and the `Sender` key.

## MODIFIED Requirements

- The `search` feature: reading the index alone, over `client`, no domain (spec/search.md "Search compiles by domain", folded 2026-11-21).

## REMOVED Requirements
