---
cairn: tasks
change: release-the-cli
---

# Tasks

- [x] default.nix and package.nix; `default = ./default.nix` on the flake.
- [x] `cli,vendored` as the derivation's feature set, overridable by the workflow's `features` input.
- [x] The rpath a non-vendored build needs.
- [x] postInstall: manual pages, five completion scripts, twelve JSON Schemas.
- [x] .github/workflows/releases.yml and release-on-demand.yml.
- [x] README: pre-built binary beside cargo.
- [x] CHANGELOG under `### Added`.
- [x] Fold the delta into cairn/spec/cli.md; log; land.
