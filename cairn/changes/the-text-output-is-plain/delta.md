---
cairn: delta
change: the-text-output-is-plain
---

## ADDED Requirements

### Requirement: The text output is plain text
Without `--json`, a list SHALL print through pimalaya-cli's `columns::Columns`: an uppercase header row, then one item per line, its cells in columns two spaces apart, an empty cell as `-`, with no border, no colour and no truncation. The free-text column SHALL read last: `collection list` ends with NAME, `item list` with TITLE (LINK ID under `--retained`), `queue list` with TARGET (then ERROR under `--parked`), and the collection counts of `store info` with COLLECTION. An item's flags SHALL join with `,`, so a line splits on blanks up to its last column.

A record SHALL print through pimalaya-cli's `fields::Fields`, one aligned `Label: value` line per field, `-` for an absent value: `store info`, followed by its collection counts after one blank line, and each placement of `item show` as blocks one blank line apart, the item's fields, its summary row under the store's own column names then its addresses by role, and one block per binding.

A string these lists and records print from the store (an id, a name, a title, a flag, a link id, a summary value, a handle, a revision, a payload, an error) SHALL pass through `sanitize::text`, so a control or bidi character prints as U+FFFD. `--help` SHALL print each paragraph on one line, never wrapped to the terminal's width. The `--json` output SHALL NOT change with the text.

#### Scenario: A collection list
- GIVEN a store holding INBOX, whose name carries a bidi override, covered since 2026-01-01, and an address book named `Personal contacts` stating no role
- WHEN `pimdir collection list` runs
- THEN it prints `ID  KIND  ROLE  GEN  LIVE  COVERED  RETAINED  NAME` aligned over one line per collection, the address book's kind, role and coverage as `-`, each name last and the override as U+FFFD (src/cli/collection.rs `the_list_prints_plain_columns_ending_with_the_name`)

#### Scenario: An item shown with a diverged binding
- GIVEN a placement holding a mail summary whose subject carries an escape sequence, bound by a source in conflict
- WHEN `pimdir item show` prints it
- THEN it prints the item's fields, a blank line, the summary's columns and addresses with the escape as U+FFFD, a blank line, then the binding's fields ending with `Conflicted at revision` and `Conflict object` (src/cli/item.rs `a_placement_prints_its_fields_its_summary_then_its_bindings`)

## MODIFIED Requirements

## REMOVED Requirements
