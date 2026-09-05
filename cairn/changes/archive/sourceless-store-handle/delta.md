---
cairn: delta
change: sourceless-store-handle
---

# Delta

## ADDED Requirements

### Requirement: A handle names a source only where an operation acts as one
`PimdirStore::open(dir)` and `PimdirStore::open_read_only(dir)` SHALL take no source. The handle they return serves what an operation means for the store as a whole: every client read, retention and its purges, the queue rows a cancellation or an acknowledgement removes, and the collection generations. None of those consult a source, and none SHALL require one to be named.

`for_source(source)` SHALL yield the source-bound handle (`PimdirSourceStore`), which carries the io-replica storage seam (`load` / `lookup_objects` / `write`), the rekeyed write and the queue drain: the operations that mean "as this side". It SHALL dereference to the source-less handle, so the store-wide surface stays reachable through it, and a caller that named no source SHALL NOT be able to reach the source-bound one.

No API SHALL invent a source name to satisfy a constructor. A store an operator reads, sweeps and purges therefore records no source, and `distinct_sources` on it stays empty.

## MODIFIED Requirements

### Requirement: A reader can open the store read-only
`PimdirStore::open_read_only(dir)` SHALL open an existing store with
`SQLITE_OPEN_READ_ONLY`: it never creates the schema (that is the owner's
opening write), and refuses a schema version other than the current one with
the version error. The returned handle exposes the full read surface; any write
through it fails at the SQLite layer, whether the write is reached directly or
through the source-bound handle it yields.

### Requirement: Several sources share one store
A store MAY be opened as several source handles (`"left"`, `"right"`, …) over the
same files, each made by binding an open store to a source; each services the seam
for its own source, and the shared database is the multi-source hub. `load_hub`
reads a collection's whole hub (every source's bindings) for a consumer that
projects each side.

### Requirement: The write source is resolved before anything is enqueued
A queued mutation is staged for one source, so the CLI SHALL resolve which source it writes as before appending anything: the `--source` flag when given, else the store's own source when it has exactly one, else a refusal: naming the candidates when there are several, asking for the flag when the store has synced none. A store syncing several sources SHALL NOT be guessed at, since creating an item for the wrong side would push it to the wrong server, and a store with no source at all offers no side to act as.

### Requirement: Terminal operations take the owner role directly
Purge, queue cancellation and orphan-blob reclamation have no action kind and cannot be queued: they SHALL take the owner role directly, without naming a source, and fail if the role is unavailable. When another writer holds the store's write lock, the CLI SHALL report it as a plain sentence naming the likely cause (a running sync) and never as a raw SQL or debug error dump.

## REMOVED Requirements

None.
