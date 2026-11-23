---
cairn: change
id: the-text-output-is-plain
status: landed
created: 2026-11-11
---

# The text output is plain

## Why

pimalaya-cli 0.4, published on crates.io on 2026-11-11, removed its `table` module and the comfy-table dependency, and ships `columns::Columns` and `fields::Fields` for the family's plain-text rule, which Himalaya and Ortie adopted the same day (Himalaya's change plain-text-output). pimdir's `cli` feature still required pimalaya-cli 0.3 with `table`: `collection list`, `item list`, `queue list` and the collection counts of `store info` drew comfy-table boxes sized to the terminal, `store info` and `item show` printed hand-rolled ` - label: value` bullets, every string from the store reached the terminal as stored (a subject's bidi override or escape sequence included), and `--help` wrapped to the terminal's width through clap's `wrap_help`.

## What

Approved by the owner on 2026-11-11.

- **Lists through `Columns`.** An uppercase header, one item per line, columns two spaces apart, an empty cell `-`, no border, no colour, no truncation. The free-text column reads last: `collection list` ends with NAME, `item list` with TITLE (LINK ID under `--retained`), `queue list` with TARGET then, under `--parked`, ERROR, and the collection counts of `store info` with COLLECTION; the other columns keep their order. An item's flags join with `,`, so a line splits on blanks up to its last column.
- **Records through `Fields`.** `store info` prints one aligned `Label: value` line per fact, `-` when absent, then its collection counts after one blank line. `item show` prints each placement as blocks one blank line apart: the item's fields, its summary row under the store's own column names then its addresses by role, and one block per binding, the conflict lines still only when they apply.
- **Sanitized.** Every string these lists and records print from the store goes through `sanitize::text`, a control or bidi character printing as U+FFFD.
- **Help unwrapped.** clap loses `wrap_help`, so `--help` prints each paragraph on one line, as pimalaya-cli 0.3.2 made the family default.
- **pimalaya-cli 0.4.** Both requirements move to `0.4`, the `table` feature goes, src/main.rs and src/cli/json_schema.rs import from the now public `clap::commands::{completion, json_schema, manual}` modules, and `cargo update` takes the semver-compatible bumps.

## Scope / non-goals

The `--json` output and the JSON Schemas do not change. The sentences (`queue status`, the receipts of `item restore`, `item purge`, `item erase`, `queue cancel`), `check`, `gc`, `export`, `quarantine empty` and the search verbs (`index`, `search`, `count`, `show`, `thread`, `tag`) keep their text: the search verbs print their hits as hand-rolled lines and stay outside this change. rusqlite stays on 0.41, 0.42 being a separate move.
