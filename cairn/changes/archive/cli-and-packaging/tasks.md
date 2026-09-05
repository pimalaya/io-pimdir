---
cairn: tasks
change: cli-and-packaging
---

# Tasks

- [x] CI: sibling pimdir checkout, full suite, per-suite skip detection for `spec_drift`, `summaries`, `vectors_sync` and `objects`, clippy, fmt, no_std unit tests, `cargo deny check`.
- [x] Cargo.toml: `rust-version = "1.88"`, `cli` on `dep:pimalaya-cli`, `exclude`, dev-dependencies without `fs4` and `rusqlite`; deny.toml without `allow-org`.
- [x] CLI: singular `completion` and `manual` with hidden plural aliases; camelCase and `JsonSchema` on every output type; `json-schema` subcommand; `store info` on one verified schema version; `blobs` and `write_source` on the held handle.
- [x] Docs: one-liners and short first paragraphs in src/main.rs and src/cli/; `doc(cfg)` on the `client` module; README layers and search claim; SECURITY.md on 0.4.x.
- [x] Cairn: delta types, `duplicate-link-id-freeze` superseded, `store-algorithm-audit` reconciled, the two skeletal deltas filled, landed changes archived, `date:` on every log entry.
- [x] Fold the delta into cairn/spec/cli.md; log; land.
