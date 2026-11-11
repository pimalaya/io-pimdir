---
cairn: log
change: wide-lists-print-as-records
date: 2026-11-11
---

# Wide lists print as records

The `pimdir` binary's wide lists print through pimalaya-cli's `records::Records` (pimalaya-cli change a-wide-list-prints-as-records, the same day), the way `git log` prints commits, and the search verbs and `check` stop printing store strings raw. The capability moved is cli (cairn/spec/cli.md, requirement "The text output is plain text" rewritten, its scenarios "A collection list" replaced and "An item list", "A search hit with a crafted subject" and "Threads" added).

## What landed

- **The rule.** `Records` when a row carries more than one value of unbounded length, `Columns` when every cell but the last is short and bounded. Classified: `collection list` (id, name), `item list` (flags, sort key, title, link id), `queue list` (producer, collection, target, error), `search`, `search --threads`, `thread` and `tag` (title, collections, snippet) are records; the collection counts of `store info` (two counts then the id) stay columns.
- **Records** (src/cli/collection.rs, item.rs, queue.rs, search.rs). `collection list` titles each collection by its id, the name a field rather than the body since it is a one-line label. `item list` titles each item by its seq, its raw flags as state words, its title as the body. `queue list` titles each action by its id. `search` and `thread` print a hit through `HitRow::record`: its seq, Account, Kind, Title, Collections, Sort key, Matched when a text term matched, its snippet as the body. `search --threads` prints one record per thread, `-` for an item no thread holds, its hits as the body in headerless `Columns` lines (`SEQ  KIND  TITLE`), a record holding no nested records. `tag` prints one Placement field per placement, its note on the line under. The lines after a list (next page, tag tallies, coverage, undecodable collections) follow one blank line.
- **Fields.** `thread` prints the thread's fields, then its hits. `show` moved to `Fields`, its placements one field each and its people, attachments, place and org as multi-line values, then its body and note after a blank line each. `check` stays sentences: its findings are capped lists under a heading sentence, not one record of labelled facts. By the rule its dangling rows (row, target) would be records and its other lists columns; left as they are.
- **Sanitized.** The search verbs printed titles, snippets, extracted texts, collections, link ids, kinds and the coverage's media types and bounds raw, and `check` its hashes, paths, rows, targets and collections: on a scratch store, the previous binary sent a subject's OSC 52 clipboard write and an objects/ entry's name to the terminal as is. Each now passes `sanitize::text`, a multi-line text line by line. `item list`'s next page token and `queue list`'s undecodable collections too.
- **Build.** pimalaya-cli comes from the local checkout through `[patch.crates-io]` (`records` is not released yet), so Cargo.lock drops its registry source; the release pin replaces both before the commit.
- **Tests.** Each shape is pinned and fails against the previous Display code: collection.rs `the_list_prints_one_record_per_collection`, item.rs `a_list_prints_one_record_per_item_its_flags_as_state`, queue.rs `a_parked_list_prints_one_record_per_action`, search.rs `a_search_prints_one_record_per_hit_its_snippet_as_the_body`, `threads_print_one_record_each_their_hits_as_the_body`, `a_thread_prints_its_fields_then_its_hits`, `a_tag_prints_one_record_per_hit_and_a_field_per_placement`, `show_prints_fields_then_the_text_line_by_line`, check.rs `the_findings_print_what_the_store_names_sanitized`. The `--json` output of 18 runs (every converted command, `item show`, `store info`, `count`, `check`) and all 25 JSON Schemas were diffed byte for byte against the previous binary on a scratch store: unchanged.

## Open

- Sentences still print store strings raw: `queue status` (collection, error), the `item restore` and `item erase` receipts (collection), and `store info`'s Store id and Format version.
- A tab in `show`'s body prints as U+FFFD, `sanitize::text` treating it as a control character.
