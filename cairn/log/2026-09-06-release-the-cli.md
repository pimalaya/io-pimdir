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
- **The rpath a non-vendored build needs.** With `--argstr features cli` the build linked and then died at 127, `libsqlite3.so: cannot open shared object file`: pkg-config hands the linker the library and no rpath, so the binary, and the test binary before it, loaded nothing. `env.NIX_LDFLAGS = "-rpath " + lib.getLib sqlite + "/lib"` settles it, which is what comodoro already does for libdbus: the wrapper reads it, so it covers every link the derivation makes rather than clobbering the `RUSTFLAGS` buildRustPackage sets itself. Guarded on `!vendored` alone: every released target vendors, so Windows never reaches it.
- **The share payload.** postInstall runs the built binary for the manual pages, the completion scripts of five shells and the twelve JSON Schemas, which is what the release attaches beside the binary.
- **releases.yml** on tags and master, publishing the crate from the x86_64-linux job, and **release-on-demand.yml** for one target and one feature set.
- **README.** A pre-built binary section beside the cargo one, saying that the released binaries hold their own SQLite and that `cargo install --features cli` links the machine's.

## Verification

- `nix-build default.nix`: `pimdir v0.4.1 +client +vendored +cli`, `ldd` naming no libsqlite3, and bin/ and share/{completions,man,schemas} laid out where the workflow copies from.
- `nix-build default.nix --argstr target x86_64-w64-mingw32 --arg isStatic true`: the mingw cross build of the release default is green, which settles the bundled SQLite on the hardest target of the matrix.
- `nix flake show`: `cross-aarch64-darwin`, `cross-aarch64-linux`, `cross-armv6l-linux`, `cross-armv7l-linux`, `cross-i686-linux`, `cross-x86_64-darwin`, `cross-x86_64-linux`, `cross-x86_64-windows` and `default`, one per entry of the shared workflow's matrix.
- `nixfmt --check` on all four nix files; the workflows parse as YAML.
- The non-vendored path was proven at `RUSTFLAGS`, where a `--argstr features cli` build went from exit 127 (`libsqlite3.so: cannot open shared object file`) to an `ldd` resolving the store's libsqlite3. It was then rewritten to `NIX_LDFLAGS` for comodoro's reasons and **not** rebuilt: CI is where that runs.

Capabilities moved: `cli`.
