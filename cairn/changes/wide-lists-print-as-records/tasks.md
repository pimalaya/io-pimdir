---
cairn: tasks
change: wide-lists-print-as-records
---

# Tasks

- [x] Cargo.toml: pimalaya-cli from the local checkout through `[patch.crates-io]` (replaced by the release pin before the commit)
- [x] src/cli/collection.rs: `collection list` through `Records`, the name a field
- [x] src/cli/item.rs: `item list` through `Records`, flags as state words, the title as the body, the next page token sanitized
- [x] src/cli/queue.rs: `queue list` through `Records`, the undecodable collections sanitized
- [x] src/cli/search.rs: `search`, `search --threads`, `thread` and `tag` through `Records` (`HitRow::record`), `show` through `Fields`, the coverage sanitized
- [x] src/cli/check.rs: every hash, path, row, target and collection sanitized
- [x] Unit tests pinning each new shape, each failing against the previous Display code
- [x] The `--json` output of every converted command and every JSON Schema diffed unchanged against the previous binary on a scratch store
- [x] CHANGELOG, delta folded into cairn/spec/cli.md, log, land
