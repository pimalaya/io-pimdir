---
cairn: log
change: the-query-split-by-domain
landed: 2026-11-21
---

# The search features split

The first half of the-query-split-by-domain, uncommitted: features `mail`, `contact`, `calendar` and `indexer` beside `search`, which now reads the index alone over `client`. The query language and the coverage build with no feature. search/extract.rs keeps the core (`PimdirExtract`, `EXTRACTION`, `EXTRACTOR`, the summary read) and dispatches to extract/mail.rs, contact.rs and calendar.rs, each gated by its domain; a body of a domain left out is read from its summary. `FLOATING_ZONE` moved from `occurrence` to `coverage`. The summary reads `item` needs moved from client/search/refresh.rs to client/search/summary.rs.

Found while splitting, against the draft: a reader is not extractor-free, since `item` and the snippets re-extract the bodies they print, so Calendula and Cardamum take their domain beside `search`; and `indexer` cannot imply `search` (so `client`), since the Android app runs the refresh coroutine over its own SQLite without `client`. `PimdirIndex::open`, `refresh` and `erase` need both.

A CI step runs clippy on `mail`, `contact`, `calendar`, `search`, `indexer`, `search,mail` and `search,indexer` alone. Capabilities moved: spec/search.md ("Search compiles by domain", the card matcher under `contact`).

Left: blake3 and encoding_rs behind `client`, `search::mail`, `PimdirQueryPredicate::name()`, the SEARCH additions, the release.
