---
cairn: log
change: a-read-verb-takes-the-index-as-it-stands
landed: 2026-11-17
---

# A read verb takes the index as it stands

CLI family audit of 2026-11-15, PIM-3, open at dcf49e2: `pimdir show 1` opened the index as the indexer does, creating index.db where none was and wiping one another extractor revision built; a plain `pimdir count` refreshed in `UTC` over the `Europe/Paris` a sync engine indexed in, re-expanding every calendar item.

## What landed

- src/cli/search.rs: `show`, and `search`, `count`, `tag` and `thread` given `--refresh-timeout 0s`, open the index through `PimdirIndex::open_query`; `--floating-zone` of `index` and of the one-shot answers defaults to the zone the index records, read before an open rebuilds an index of another extraction.
- Test: tests/search_cli.rs `a_read_verb_leaves_the_index_as_it_stands`.

## Capabilities moved

cli ("A read verb takes the index as it stands", added).

In the working tree on io-pimdir dcf49e2 (0.15.1), uncommitted, no version bumped; CHANGELOG.md [Unreleased] names it.
