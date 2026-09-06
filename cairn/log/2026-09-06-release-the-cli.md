---
cairn: log
change: release-the-cli
date: 2026-09-06
---

# The CLI ships

tests.yml was this repository's only workflow. The `pimdir` binary built and was never released, and the crate had no way to reach crates.io, which is the release neverest 0.2 is waiting on.

The shared pimalaya/nix workflows do both, and they want a flake exposing `cross-<target>` outputs over a derivation whose `result/bin/<project>*` and `result/share` they collect. This repository had a flake exposing a devshell alone, so the work was the two nix files under it, not the YAML.

## What landed

- **default.nix and package.nix**, and `default = ./default.nix` on the flake. Modelled on neverest's, with two differences the repository forces. The crate is io-pimdir and the binary is `pimdir`, and the workflow's `project` input is the binary glob, so it reads `pimdir`. And the binary sits behind `cli`, which is not a default, so the derivation names its own feature set rather than inheriting one.
- **`cli,vendored` unless told otherwise.** A released binary carries the SQLite it was built with, so it depends on nothing where it lands, and the static-musl and mingw targets need no cross pkg-config to reach a system one. `release-on-demand.yml` takes a `features` input for anything else, defaulting to the same pair.
- **The rpath a non-vendored build needs.** With `--argstr features cli` the build linked and then died at 127, `libsqlite3.so: cannot open shared object file`: rustc hands the linker sqlite's `-L` and nix's wrapper writes no rpath for it, so the binary, and the test binary before it, loaded nothing. `RUSTFLAGS = "-C link-arg=-Wl,-rpath,${lib.getLib sqlite}/lib"` under `lib.optionalAttrs (!vendored)` settles it, and the vendored build carries none of it.
- **The share payload.** postInstall runs the built binary for the manual pages, the completion scripts of five shells and the twelve JSON Schemas, which is what the release attaches beside the binary.
- **releases.yml** on tags and master, publishing the crate from the x86_64-linux job, and **release-on-demand.yml** for one target and one feature set.
- **README.** A pre-built binary section beside the cargo one, saying that the released binaries hold their own SQLite and that `cargo install --features cli` links the machine's.

## Verification

- `nix-build default.nix` (the flake path cannot see untracked files): `pimdir v0.4.1 +client +vendored +cli`, `ldd` naming no libsqlite3, and bin/ and share/{completions,man,schemas} laid out where the workflow copies from.
- `nix-build default.nix --argstr features cli`: `pimdir v0.4.1 +cli +client`, `ldd` resolving libsqlite3.so to the store, which is the case that failed before the rpath.
- `nixfmt --check` on all four nix files.

Capabilities moved: `cli`.
