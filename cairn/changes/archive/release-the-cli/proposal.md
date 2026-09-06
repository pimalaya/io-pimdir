---
cairn: change
id: release-the-cli
status: landed
created: 2026-09-06
---

# The CLI ships

The `pimdir` binary has been buildable for weeks and releasable never. tests.yml is the repository's only workflow, so there is no way to cut a tag into binaries and no way to publish the crate, which is the release neverest 0.2 waits on.

Every other Pimalaya CLI delegates that to the shared workflows in pimalaya/nix, which want a flake exposing `cross-<target>` outputs and a derivation whose `result/bin/<project>*` and `result/share` they collect. This repository has a flake exposing a devshell alone, and neither default.nix nor package.nix.

Two things make it not a copy of neverest's. The binary is `pimdir` while the crate is io-pimdir, and the workflow's `project` input is the *binary* glob, so it is `pimdir`. And the binary sits behind `cli`, which is deliberately not a default, so the derivation cannot rely on the default feature set the way a repository whose binary is its default can.

## What

- default.nix and package.nix, and `default = ./default.nix` on the flake, so `nix build .#default` and `nix build .#cross-<target>` produce the binary the shared workflow copies.
- The derivation builds `cli,vendored` unless told otherwise: a released binary carries its own SQLite rather than needing one on the machine it lands on, which is also what makes the static and mingw targets buildable without a cross pkg-config.
- A non-vendored build stays supported and gains the rpath it needs: rustc hands the linker sqlite's `-L` and nix's wrapper writes no rpath, so without it the binary loads nothing at run time.
- postInstall generates the manual pages, the completion scripts for five shells and the twelve JSON Schemas into share/, which is the payload the release attaches beside the binary.
- releases.yml on tags and master, release-on-demand.yml for one target and one feature set.

## Not in scope

No install.sh: neverest and himalaya carry one and this can have one when it is worth a curl-to-shell, which an operator tool arguably is not. The `version` in default.nix and package.nix is maintained by hand, as in every sibling.
