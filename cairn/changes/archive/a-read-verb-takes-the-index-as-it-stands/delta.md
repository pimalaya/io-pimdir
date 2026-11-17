---
cairn: delta
change: a-read-verb-takes-the-index-as-it-stands
---

## ADDED Requirements

### Requirement: A read verb takes the index as it stands
`pimdir show` SHALL open the index for queries alone (`PimdirIndex::open_query`), and so SHALL `search`, `count`, `tag` and `thread` given `--refresh-timeout 0s`: none creates index.db, nor rebuilds one standing under another extractor, which another release's indexer keeps; an index missing is `index-missing`. A refresh given no `--floating-zone` (`index`, and the one-shot answers) SHALL fold in the zone the index records, read before an open rebuilds an index of another extraction, `UTC` for a new one, so a one-shot answer never resets the zone a sync engine indexed in and re-expands every calendar item.

#### Scenario: Read verbs on a store with no index, then on another extractor's
- GIVEN a store holding one message and no index.db
- WHEN `show 1` and `count --refresh-timeout 0s` run
- THEN both fail `index-missing` and index.db is not created
- WHEN `index --floating-zone Europe/Paris` runs, then `count date:2026-08-02` twice beside a plain `index`
- THEN the message of 23:00Z counts on 2 August each time, nothing re-expanded
- WHEN the index records another extractor and `show 1` and a `0s` count run
- THEN both answer and the index keeps its extractor until a refresh rebuilds it (tests/search_cli.rs `a_read_verb_leaves_the_index_as_it_stands`)

## MODIFIED Requirements

## REMOVED Requirements
