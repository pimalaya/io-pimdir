---
cairn: change
id: system-sqlite-by-default
status: landed
created: 2026-09-06
---

# The system SQLite by default, the source build behind `vendored`

## Why

The `client` feature pins `rusqlite` with `bundled`, so every build of this crate compiles its own SQLite and links it statically, with no way to say otherwise. That is the wrong default for a library a distribution packages: a packager wants one SQLite on the machine, patched once, and a store written by the `pimdir` binary readable by the `sqlite3` the same system ships.

Every other native dependency in the org is already handled the other way round. `pimalaya-stream` links the system OpenSSL and offers `vendored`, off by default, for the platforms and the static targets that need a source build, and io-imap, io-smtp, himalaya and neverest forward that feature by name. SQLite is the only native library in the ecosystem that cannot be chosen.

The test layout drifted at the same time: five modules keep their unit tests in a sibling `<module>/tests.rs`, where every other file in this crate and every other Pimalaya repository keeps them in the `#[cfg(test)] mod tests` of the module itself.

## What

- **`vendored`**, a new feature, off by default, forwarding to `rusqlite?/bundled`. The `client` feature takes `rusqlite` with no features, so the default build links the system SQLite through pkg-config.
- **The devshell** carries `sqlite` in `buildInputs` and on `LD_LIBRARY_PATH`, the way neverest's carries `openssl`, so the test binaries a devshell builds find the library at run time.
- **The unit tests move back into their modules**: src/hub.rs, src/mutate.rs, src/rekey.rs, src/sync.rs and src/upgrade.rs each carry their own `mod tests`, and the four directories that held nothing else are gone.

## Not in scope

The consumers forward the feature in their own repositories; neverest's is the only one this pass touches, since it is the release in flight.
