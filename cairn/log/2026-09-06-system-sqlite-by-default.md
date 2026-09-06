---
cairn: log
change: system-sqlite-by-default
date: 2026-09-06
---

# The system SQLite by default, the source build behind `vendored`

The crate stopped compiling its own SQLite on every build. Nothing the store does changed; what changed is which library it links and where the unit tests live.

## What landed

- **`vendored`** (capability `store`). A new feature, off by default, forwarding to `rusqlite?/bundled`. `client` takes `rusqlite` with no features, so the default build resolves SQLite through pkg-config and a packager patches one library rather than every crate that embeds one. This is the shape `pimalaya-stream` already uses for OpenSSL, which io-imap, io-smtp, himalaya and neverest forward by name; SQLite had been the one native dependency with no say in it.
- **The devshell.** shell.nix carries `sqlite` in `buildInputs` and on `LD_LIBRARY_PATH`: pkg-config finds the library at build time from the first, and the test binaries a devshell builds find it at run time from the second, since nix's linker wrapper writes no rpath outside a derivation.
- **The unit tests moved home.** src/hub.rs, src/mutate.rs, src/rekey.rs, src/sync.rs and src/upgrade.rs each carry their own `#[cfg(test)] mod tests` again, and the four directories that held nothing but a tests.rs are gone. src/sync/ stays for join.rs. Not a test changed; every other module in the crate, and every other Pimalaya repository, already kept them this way.

## Verification

- `cargo build --features cli` (system SQLite, `libsqlite3-sys` without the bundled build) and `cargo build --all-features` (vendored) both clean.
- `cargo test --features cli`: 26 binaries, all green, 261 unit tests among them; `cargo clippy --all-features --all-targets` clean; `cargo fmt`.
- neverest builds and runs against both, its `vendored` feature forwarding to this one.

Capabilities moved: `store`.
