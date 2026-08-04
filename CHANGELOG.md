# Changelog

All notable changes to this project are documented in this file. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0/).

## [Unreleased]

### Added

- Initial pimdir store: a SQLite index plus a content-addressed, two-level-sharded blob directory, implementing io-replica's storage seam (load, lookup_objects, write) for one source.
- no_std core reusable without the SQLite client: the canonical schema and statements (sql) and the model-to-column encodings (codec).
- Store-global public ids (seq): one per message, shared across every collection it is filed in, monotonic and never reused.
- Streaming blob ingest and read, so a large body is never held whole; a byteless object write indexes a body already streamed to its content-addressed path.
- Incremental, cross-collection-correct reference counting with blob garbage collection inside the write transaction; a crash leaves at worst an orphan blob, never a row without its body.
- Single-writer serialisation via BEGIN IMMEDIATE and a generous busy timeout, so several same-source handles overlap network while their writes serialise.
- An availability-aware, paginated client read surface (list_items, get_item, count_items, distinct_sources, seq_for_link) projecting the store as a local backend.
