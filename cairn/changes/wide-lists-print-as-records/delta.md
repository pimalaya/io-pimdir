---
cairn: delta
change: wide-lists-print-as-records
---

## ADDED Requirements

## MODIFIED Requirements

### Requirement: The text output is plain text
Without `--json`, a list SHALL print through pimalaya-cli's `records::Records` when its rows carry more than one value of unbounded length, and through `columns::Columns` when every cell but the last is short and bounded, with no border, no colour and no truncation either way.

As records, each item SHALL print as one block, one blank line apart: its title alone on its first line, its state words after it in parentheses and comma-separated when it has any, its fields as aligned `Label: value` lines, `-` for an absent value, then its free text after a blank line, each line indented by four spaces. `collection list` SHALL print each collection under its id with its Name, Kind, Role, Generation, Live, Coverage and Retained; `item list` each item under its seq, its raw flags as state words, its Level and Sort key (Level, Retained at, Retained by and Link id under `--retained`), its title as the body; `queue list` each action under its id with its Created, Producer, Collection, Action, Tries, Target and, under `--parked`, Error; `search` each hit under its seq with its Account when the store has several, Kind, Title, Collections, Sort key and, when a text term matched, Matched, its snippet as the body; `search --threads` each thread under its id, `-` for an item no thread holds, with its Account, Messages, First and Last, its hits as the body, one `SEQ  KIND  TITLE` line each; `tag` each hit under its seq with its Account, Kind, Title and one Placement per placement naming its collection, the flags the tag gives it and its status, the reason it is not staged or stays in the store on the line under. What a list prints after its items (the next page, the tag's tallies and notes, the coverage, the collections whose queue could not be decoded) SHALL follow one blank line.

As columns, a list SHALL print an uppercase header row, then one item per line, its cells two spaces apart, an empty cell as `-`, the free-text column last: the collection counts of `store info` (LIVE, RETAINED, COLLECTION).

A record SHALL print through `fields::Fields`, one aligned `Label: value` line per field, `-` for an absent value, a multi-line value continuing under its first line: `store info`, followed by its collection counts after one blank line; each placement of `item show` as blocks one blank line apart, the item's fields, its summary row under the store's own column names then its addresses by role, and one block per binding; `thread`, the thread's Thread, Account, Messages, First and Last, then its hits after a blank line as `search` prints them; `show`, the item's Seq, Account, Kind, Title, one Placement per placement, Read from, Status, People, Attachments, Place and Org, then its body and its note, each after a blank line. `check` SHALL print its findings as sentences heading capped lists.

A string these outputs print from the store (an id, a name, a title, a flag, a link id, a sort key, a summary value, a handle, a revision, a payload, an error, a snippet, an extracted text, a media type, a date, a hash, a path) SHALL pass through `sanitize::text`, a multi-line text line by line so its breaks survive, so a control or bidi character prints as U+FFFD. `--help` SHALL print each paragraph on one line, never wrapped to the terminal's width. The `--json` output SHALL NOT change with the text.

#### Scenario: A collection list
- GIVEN a store holding INBOX, whose name carries a bidi override, covered since 2026-01-01, and a collection whose id carries an escape sequence and which states no kind, role or coverage
- WHEN `pimdir collection list` runs
- THEN it prints one record per collection, its id as the title and its Name, Kind, Role, Generation, Live, Coverage and Retained aligned under it, `-` for what the second does not state, the override and the escape as U+FFFD (src/cli/collection.rs `the_list_prints_one_record_per_collection`)

#### Scenario: An item list
- GIVEN a live page of two items, the first flagged `\Flagged` and with a flag carrying a BEL, titled with a bidi override, the second with neither flags nor title, then a next page
- WHEN `pimdir item list` prints it
- THEN the first reads `3 (\Flagged, $Junk�)`, its Level and Sort key, a blank line and its title indented by four spaces, the second `1` with no state and no body, then a blank line and the `--page-token` to continue (src/cli/item.rs `a_list_prints_one_record_per_item_its_flags_as_state`)

#### Scenario: A search hit with a crafted subject
- GIVEN a hit whose title carries a bidi override and whose snippet carries an OSC 52 clipboard write, and a partial coverage whose media type carries an escape sequence
- WHEN `pimdir search` prints it
- THEN the hit reads its seq, its fields, a blank line and its snippet indented by four spaces, then after a blank line the next page and the coverage, every control and bidi character as U+FFFD (src/cli/search.rs `a_search_prints_one_record_per_hit_its_snippet_as_the_body`)

#### Scenario: Threads
- GIVEN a thread of two hits, one titled with an escape sequence, and a contact no thread holds
- WHEN `pimdir search --threads` prints them
- THEN the thread reads its id, its Messages, First and Last, then its hits as `SEQ  KIND  TITLE` lines indented by four spaces, and the contact `-` with one message, `-` for its dates and its one hit line (src/cli/search.rs `threads_print_one_record_each_their_hits_as_the_body`; `thread` in `a_thread_prints_its_fields_then_its_hits`, `tag` in `a_tag_prints_one_record_per_hit_and_a_field_per_placement`, `show` in `show_prints_fields_then_the_text_line_by_line`, `check` in src/cli/check.rs `the_findings_print_what_the_store_names_sanitized`)

#### Scenario: An item shown with a diverged binding
- GIVEN a placement holding a mail summary whose subject carries an escape sequence, bound by a source in conflict
- WHEN `pimdir item show` prints it
- THEN it prints the item's fields, a blank line, the summary's columns and addresses with the escape as U+FFFD, a blank line, then the binding's fields ending with `Conflicted at revision` and `Conflict object` (src/cli/item.rs `a_placement_prints_its_fields_its_summary_then_its_bindings`)

## REMOVED Requirements
