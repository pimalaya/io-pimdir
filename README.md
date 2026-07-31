# io-pimdir

A [pimdir](https://github.com/pimalaya/pimdir) store for Rust: an
[io-replica](https://github.com/pimalaya/io-replica) `ReplicaStorage` backed by a
SQLite index plus a content-addressed blob directory.

io-replica is the I/O-free replica engine; it owns the model and the sync logic
and yields storage `Wants`. io-pimdir is the store that services them, persisting
the engine's placements and objects as a pimdir store (`pimdir.db` + `objects/`).
The pimdir spec is the on-disk contract, so a store written here is readable by
any other conformant implementation (a native Android SQLite store, for example).

## Layout

The crate is split so the logic stays I/O-free and the I/O sits behind a feature
gate, matching pimdir's "operations are canonical" model:

- `codec` — pure, `no_std` encodings between the io-replica model and the store
  columns (flag JSON, the `level`/`status` integer maps).
- `sql` — the canonical pimdir schema and statements, inlined from the spec.
- `client` (feature `client`, default) — `PimdirStore`, which runs the statements
  against SQLite (`rusqlite`, bundled) and the blob files.

The crate is `no_std` with `alloc`; `std` only enters behind the `client`
feature, where the SQLite driver lives.

## Usage

```rust
use io_pimdir::PimdirStore;
use io_replica::client::ReplicaStorage;

let mut store = PimdirStore::open("/path/to/store")?;
let loaded = store.load(&"INBOX".into())?;
// drive io-replica's coroutines, servicing WantsLoad/Write/LookupObject with `store`
```

## Status

Early. Single-source stores (one remote per collection) work end-to-end; the
multi-source hub is not yet modelled here. Schema version 1.

## License

Licensed under either of [Apache License 2.0](./LICENSE-APACHE) or
[MIT license](./LICENSE-MIT) at your option.
