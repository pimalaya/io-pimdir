---
cairn: change
id: wide-lists-print-as-records
status: landed
created: 2026-11-11
---

# Wide lists print as records

## Why

The change the-text-output-is-plain (2026-11-11) moved the `pimdir` lists to pimalaya-cli's `columns::Columns`. A `Columns` row never truncates and never asks the terminal its width, so a row carrying several values of unbounded length (an id, a name, a sort key that is a contact's name, a title, a target, an error) wraps and loses its alignment as soon as one value is long. pimalaya-cli then gained `records::Records` (its change a-wide-list-prints-as-records, same day): one block per item the way `git log` prints commits, with the family rule choosing it over `Columns`. The owner approved adopting it here on 2026-11-11, after Himalaya's envelope shape (the id as title, flags as state words, properties as fields, free prose as an indented body).

The search verbs stayed outside the first change: `search`, `search --threads`, `thread`, `tag` and `show` printed hand-rolled lines, and `check` its findings, with every title, snippet, extracted text, collection, link id, path and coverage value from the store unsanitized, so a crafted subject's escape sequences (an OSC 52 clipboard write, a screen clear) or a bidi override reached the terminal.

## What

Approved by the owner on 2026-11-11.

- **The rule.** A list prints as `Records` when its rows carry more than one value of unbounded length, and stays `Columns` when every cell but the last is short and bounded.
- **`collection list`**: one record per collection under its id, the fields Name, Kind, Role, Generation, Live, Coverage, Retained. The name is a field, not the body: it is a one-line label, and the body is kept for free prose.
- **`item list`**: one record per item under its seq, its flags as state words (raw, never interpreted), the fields Level and Sort key (Level, Retained at, Retained by and Link id under `--retained`), its title as the body.
- **`queue list`**: one record per action under its id, the fields Created, Producer, Collection, Action, Tries, Target, and Error under `--parked`.
- **`search`**: one record per hit under its seq, the fields Account (when the store has several), Kind, Title, Collections, Sort key and Matched (when a text term matched), its snippet as the body. `thread` prints the thread's fields (Thread, Account, Messages, First, Last), a blank line, then its hits as `search` prints them.
- **`search --threads`**: the listed items are threads, so one record per thread under its id (`-` for an item no thread holds), the fields Account, Messages, First and Last, its hits as the body, one `SEQ  KIND  TITLE` line each: a record cannot nest records.
- **`tag`**: one record per hit under its seq, the fields Account, Kind, Title and one Placement per placement (`COLLECTION: FLAGS (STATUS)`, the not-staged or kept-in-the-store note on its next line).
- **`show`**: `fields::Fields` (Seq, Account, Kind, Title, one Placement per placement, Read from, Status, People, Attachments, Place, Org, a multi-line text continuing under its label), then the body and the note, each after a blank line.
- **`check`**: stays sentences heading capped bullet lists, which is no record of labelled facts, so no straight `Fields` conversion; only sanitized.
- **Trailers.** What a list prints after its items (the next page, the tag tallies and notes, the coverage, the undecodable collections) follows one blank line.
- **Sanitized.** Every store string these outputs print passes through `sanitize::text`, a multi-line text line by line so its breaks survive: titles, snippets, extracted texts, kinds, collections, link ids, accounts, thread ids, dates, the coverage's media types and bounds, check's hashes, paths, rows, targets and collections, the undecodable collections of `queue list` and `item list`'s next page token.

## Scope / non-goals

The `--json` output and the JSON Schemas do not change. `store info` and `item show` already print `Fields` and stay; the collection counts of `store info` (LIVE, RETAINED, COLLECTION) stay `Columns`, every cell but the last being a count. The sentences (`index`, `count`, `queue status`, `queue cancel`, the receipts of `item restore`, `item purge`, `item erase`, `item export --output`, `gc`, `export`, `quarantine empty`) keep their text. The build points pimalaya-cli at its local checkout through `[patch.crates-io]` until the release carrying `records` is pinned.
