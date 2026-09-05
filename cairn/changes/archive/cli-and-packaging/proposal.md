---
cairn: change
id: cli-and-packaging
status: landed
created: 2026-09-04
---

# The CLI meets the org contract, and the crate packages cleanly

## Why

The `pimdir` binary grew verb by verb while the engine was being folded in, and it drifted from the contract every other Pimalaya CLI honours. Its `--json` keys are snake_case where the org standard is camelCase, no command describes its output with a JSON Schema, and `completions` and `manuals` are plural where every other product spells them singular.

Two of its helpers open a second `PimdirReader` behind the handle the verb already holds, which cli.md forbids, and cli.md itself still claims a read-only diagnostic connection that the compaction change removed.

The packaging drifted the same way. CI still checks out io-replica, which is no longer a dependency, and loops over a `conventions` suite that became `summaries`, so a green run proves nothing about the vectors. The `rust-version` says 1.87 while the source uses let chains, which need 1.88.

An optional build-dependency leaks an accidental public `pimalaya-cli` feature, `cargo package` ships the cairn history and the proptest regressions, and two dev-dependencies duplicate what the `client` feature already pulls.

Under cairn/, twenty-one delta files carry the wrong type, a superseded change still reads as landed, a landed audit keeps twenty-five unchecked boxes, and two deltas list headings with no body.

## What

- **The CLI contract.** Singular `completion` and `manual` with hidden plural aliases; `#[serde(rename_all = "camelCase")]` on every output type, nested row and status enum; `JsonSchema` on all of them and a `json-schema` subcommand listing every command's output, mirroring himalaya's registry.
- **One schema version.** `store info` prints the version the reader verified on open, since the reader refuses any other stamp and exposes no pragma read.
- **One handle per verb.** `StoreFlags::blobs` and `write_source` take the handle the verb holds rather than opening another.
- **Documented carve-outs.** The export's `format_version` 2, and `item export` refusing `--json` on stdout, stated as the one exception to "every command renders as JSON".
- **Packaging.** `rust-version = "1.88"`; `cli` pulls pimalaya-cli through `dep:` with the terminal features on the dependency line, so the build script's copy keeps only `build`; an `exclude` keeping cairn/, .github/ and tests/proptest-regressions/ out of the package; the duplicated dev-dependencies removed; deny.toml without the unmatched `allow-org`; the `client` badge on docs.rs.
- **CI.** The sibling pimdir checkout, the whole suite, then each spec suite run on its own and failed when it reports the checkout missing; clippy with every target, `fmt --check`, the no_std core's unit tests without default features, and `cargo deny check`.
- **Docs and cairn structure.** The missing one-liners and the over-long first paragraphs in src/main.rs and src/cli/; the README's layer count; SECURITY.md on 0.4.x; every delta.md on `cairn: delta`; `duplicate-link-id-freeze` superseded with its tasks explained; `store-algorithm-audit`'s tasks reconciled with its log; the two skeletal deltas given their requirement text; every landed change archived; `date:` on every log entry.

## Not in scope

`fs4` stays: `std::fs::File::lock` needs 1.89 and the crate moves to 1.88 only. CHANGELOG.md is rewritten from the three concurrent reports rather than here. The `engine-conformance` and `store-conformance` changes land beside this one and are not archived by it.
