---
cairn: log
change: cli-and-packaging
date: 2026-09-04
---

# The CLI on the org contract, the crate packaged cleanly

The `pimdir` binary caught up with what every other Pimalaya CLI honours, and the crate's packaging, CI and cairn history were put in order in the same pass. No verb changed what it does; what changed is the shape of what it prints and how the crate ships.

## What landed

- **The JSON contract** (capability `cli`). Every output type carries `#[serde(rename_all = "camelCase")]` and derives `JsonSchema`, the nested rows and the `item restore` status enum included, and a `json-schema` subcommand (pimalaya-cli's `JsonSchemaCommand`) lists the twelve data commands by their dash-joined path.
- **The `check` findings are the CLI's own rows.** The library's diagnostics types derive no schema, so three small structs mirror them: a duplication `store-compaction` had removed and this change takes back deliberately, so the CLI's contract cannot move when a library field does.
- **One schema version.** The reader refuses any stamp but the one this build services and exposes no pragma read, so the two figures `store info` printed were always one figure.
- **Singular toolkit verbs.** `completion`, `manual` and `json-schema`, each with its plural as a hidden alias; the verb groups gained hidden plural aliases too. The help footer carries the bug tracker and the sponsoring links.
- **One handle per verb.** `StoreFlags::blobs` opened a second `PimdirReader` behind the one the verb held, and `write_source` a third; both take the held reader now, and `item restore` resolves its source before dropping it.
- **The MSRV.** `rust-version` is 1.88, which the let chains in src/sync.rs need; `fs4` stays, since `std::fs::File::lock` is 1.89.
- **Features.** `cli` pulls pimalaya-cli through `dep:` with the terminal features on the dependency line, so the accidental public `pimalaya-cli` feature is gone. `cargo tree -e features` shows two copies, `build` on the host side and `prompt`, `table`, `terminal` on the target side, so the build-dependency kept its name: renaming it would have bought nothing and needed a build.rs edit.
- **Packaging.** `exclude` keeps .github/, cairn/ and tests/proptest-regressions/ out of the package, the `fs4` and `rusqlite` dev-dependencies went since the `client` feature already provides them to every integration test, and deny.toml lost the `allow-org` no git source matched.
- **CI.** The sibling pimdir checkout only (io-replica is no dependency any more), `cargo deny check`, fmt, clippy on every target, the whole suite, the no_std core's unit tests without default features, and each spec suite (`spec_drift`, `summaries`, `vectors_sync`, `objects`) run on its own and failed when it reports the checkout missing. The `conventions` suite the old job named had become `summaries`, so the vectors had stopped being proven.
- **Docs.** The `client` module shows its feature badge on docs.rs; the README says three layers of which two are gated; SECURITY.md supports 0.4.x; every pub item of the binary carries a one-line first paragraph and the clap first paragraphs fit two lines.
- **Cairn structure.** Twenty-one delta files carried `cairn: change` and carry `cairn: delta`; `duplicate-link-id-freeze` is `superseded` with each task credited or dropped; `store-algorithm-audit`'s twenty-five boxes are reconciled with its log and its addendum; the `engine-merge` and `vendored-spec-sql` deltas carry the requirement text they folded; every other unchecked box on a landed change says why; every log entry carries `date:`; every landed change is archived.

## Verification

- `cargo check --features cli`, `cargo clippy --all-features --all-targets` and `cargo doc --no-deps --all-features` clean for the CLI files; `cargo deny check` clean; `cargo fmt`.
- `pimdir --help`, `pimdir completion --help` and `pimdir json-schema --dir` run against the built binary.

Capabilities moved: `cli`.
