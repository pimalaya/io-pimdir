---
cairn: tasks
change: operator-cli
---

# Tasks

- [x] `Cargo.toml`: `[[bin]] pimdir` with `required-features = ["cli"]`, a `cli`
      feature outside `default` pulling `client`, clap, humantime and the
      pimalaya-cli terminal/table/prompt pieces, plus the release profile.
- [x] `build.rs`: bake feature, target and git metadata for `--version`, a
      no-op without the `cli` feature.
- [x] `src/main.rs`: the clap parser, the global store/source/log/json flags,
      the printer, `manuals` and `completions`.
- [x] `src/cli/`: shared store opening and role handling, then one module per
      verb group (collection, item, queue, store, check, export) and the
      read-only diagnostic connection.
- [x] Reads open read-only; item mutations enqueue then drain when the owner
      role is free; `Busy` reports as "another writer holds the store lock".
- [x] Destructive verbs (`item purge`, `queue cancel`, `check --fix`) confirm
      unless `--yes`, and refuse to prompt when output is not a terminal.
- [x] Listings never truncate silently: every paged listing reports what it
      omitted and the cursor to continue from.
- [x] `export` writes a manifest, per-collection JSONL and the blob copies;
      `import` deliberately left out (it would interpret item content).
- [x] `README.md` reshaped into the hybrid lib+CLI form (Installation and a
      Usage that redirects to `--help`).
- [x] fmt + clippy clean on the library alone and with `--features cli`.
- [x] Fold `delta.md` into the new `cairn/spec/cli.md`; log; land.
