---
cairn: tasks
change: the-text-output-is-plain
---

# Tasks

- [x] Cargo.toml: pimalaya-cli `0.4` in [build-dependencies] and [dependencies], `table` dropped, clap's `wrap_help` dropped; `cargo update`
- [x] src/main.rs, src/cli/json_schema.rs: import from `clap::commands::{completion, json_schema, manual}`
- [x] src/cli/collection.rs: `collection list` through `Columns`, NAME last
- [x] src/cli/item.rs: `item list` through `Columns`, TITLE or LINK ID last, flags joined with `,`; `item show` through `Fields` blocks
- [x] src/cli/summary.rs: `fields` replaces `lines`
- [x] src/cli/queue.rs: `queue list` through `Columns`, TARGET then ERROR last
- [x] src/cli/store.rs: `store info` through `Fields`, its collection counts through `Columns`
- [x] `sanitize::text` on every string from the store these outputs print
- [x] Unit tests pinning each text shape; the `--json` output and schemas diffed unchanged on a scratch store
- [x] CHANGELOG, delta folded into cairn/spec/cli.md, log, land
