# Changelog

All notable changes to this project are documented in this file. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0/).

## [Unreleased]

## [0.1.0] - 2026-08-06

### Added

- Initial pimdir store: a SQLite index plus a content-addressed, two-level-sharded blob directory, implementing io-replica's storage seam (load, lookup_objects, write) for one source.
- no_std core reusable without the SQLite client: the canonical schema and statements (sql) and the model-to-column encodings (codec).
- Store-global public ids (seq): one per message, shared across every collection it is filed in, monotonic and never reused.
- Streaming blob ingest and read, so a large body is never held whole; a byteless object write indexes a body already streamed to its content-addressed path.
- Incremental, cross-collection-correct reference counting with blob garbage collection inside the write transaction; a crash leaves at worst an orphan blob, never a row without its body.
- Single-writer serialisation via BEGIN IMMEDIATE and a generous busy timeout, so several same-source handles overlap network while their writes serialise.
- An availability-aware, paginated client read surface (list_items, get_item, count_items, distinct_sources, seq_for_link) projecting the store as a local backend.
- The action queue table and collections.generation as part of the draft v1 schema, with user_version and store_meta.version kept in agreement and a store stamped with a higher schema version refused on open (the spec is a draft: draft stores are recreated, never migrated).
- The action queue (spec §14): PimdirProducer (the single enqueue transaction any non-owner process may run, pinning a pre-written body against garbage collection) and the owner's drain (drain_collection applies each action and deletes its row in one transaction, parking permanently failing actions), plus queued_collections, pending_actions (the read-your-writes overlay) and parked_actions.
- The action payload codec in the no_std core: PimdirAction (add, set-flags, remove, move, copy, update, addressing items by public seq) with a strict, versioned JSON round-trip.
- Collection generations (spec §15): the handle-space epoch on PimdirCollection and generation(), bumped atomically with a rebuild batch by write_rekeyed().
- Read-only store open (open_read_only): opens an existing store with SQLITE_OPEN_READ_ONLY, never creates anything, refuses any other schema version, and exposes the full read surface for frontend processes that must be unable to write.

[unreleased]: https://github.com/pimalaya/io-pimdir/compare/v0.1.0..HEAD
[0.1.0]: https://github.com/pimalaya/io-pimdir/compare/root..v0.1.0
