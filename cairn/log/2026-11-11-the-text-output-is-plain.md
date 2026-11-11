---
cairn: log
change: the-text-output-is-plain
date: 2026-11-11
---

# The text output is plain

The `pimdir` binary moved to pimalaya-cli 0.4 (published 2026-11-11), whose `table` module and comfy-table dependency are gone, and its text output follows the family's plain-text rule as Himalaya and Ortie did the same day. The capability moved is cli (cairn/spec/cli.md, new requirement "The text output is plain text").

## What landed

- **Lists** (src/cli/collection.rs, item.rs, queue.rs, store.rs). `collection list`, `item list`, `queue list` and the collection counts of `store info` print through `columns::Columns`, the free-text column last: NAME, TITLE (LINK ID under `--retained`), TARGET then ERROR under `--parked`, COLLECTION. The other columns keep their order; an item's flags join with `,`; an absent cell is an empty one `Columns` prints as `-`.
- **Records** (src/cli/store.rs, item.rs, summary.rs). `store info` prints through `fields::Fields`, one fact per line (the former ` - format: ..., schema version ...` bullets split into `Format version`, `Schema version`, `Read version` and so on, sources joined with `,`), then its counts after one blank line. `item show` prints each placement as `Fields` blocks one blank line apart: the item, its summary row (`summary::fields` replaces `summary::lines`), one per binding with the conflict lines only when they apply.
- **Sanitized.** Each string these outputs print from the store goes through `sanitize::text`; a subject's U+202E reached the terminal as is before.
- **Dependencies.** pimalaya-cli `0.4` in [build-dependencies] (`build`) and [dependencies] (`prompt`, `terminal`), `table` dropped; clap's `wrap_help` dropped, so help prints unwrapped as pimalaya-cli 0.3.2 made the default; src/main.rs and src/cli/json_schema.rs import from `clap::commands::{completion, json_schema, manual}`. `cargo update` within semver: pimalaya-cli 0.3.1 to 0.4.0, dirs 6 to 7, jiff 0.2.39 to 0.2.43 and patch bumps (cc, itoa, serde_spanned, smallvec, syn 3, toml, toml_datetime, winnow); comfy-table, terminal_size, git2, libgit2-sys, libz-sys, jobserver, secrecy, zeroize, the anstream family, defmt, indexmap and toml_writer left the lockfile. `cargo deny check` stays clean.
- **Tests.** Display tests pin each shape: collection.rs `the_list_prints_plain_columns_ending_with_the_name`, item.rs `a_live_list_ends_with_the_title_and_a_retained_one_with_the_link_id` and `a_placement_prints_its_fields_its_summary_then_its_bindings`, queue.rs `a_parked_list_ends_with_the_target_then_the_error`, store.rs `the_summary_prints_fields_then_its_collections`. The `--json` output of the converted commands and every JSON Schema were diffed byte for byte against the previous binary on a scratch store: unchanged.

## Open

- The search verbs (`search`, `thread`, `tag`, `show`) and `check` still print hand-rolled lines, and print titles, snippets and summary text from the store unsanitized.
- rusqlite 0.42 is available and not taken.
