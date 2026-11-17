---
cairn: delta
change: the-vendored-requirement-says-on
---

## ADDED Requirements

## MODIFIED Requirements

### Requirement: SQLite is bundled unless `vendored` is turned off
The `client` feature SHALL take `rusqlite` with no features of its own. A `vendored` feature, on by default since 0.5.1, SHALL forward to `rusqlite/bundled`, so `cargo install io-pimdir --features cli` builds SQLite from source and needs none on the machine, mirroring what `vendored` does for OpenSSL across the org. A consumer building with `--no-default-features` and without `vendored` SHALL link the SQLite the machine provides through pkg-config, as the Nix builds do, so a store this crate writes is the one the system's `sqlite3` reads; CI proves that link on its own (`cargo test --no-default-features --features cli`). No other feature SHALL imply either choice.


## REMOVED Requirements
