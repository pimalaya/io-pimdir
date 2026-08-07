---
cairn: change
change: operator-cli
---

# Delta

All of these land in the new `cairn/spec/cli.md` capability file; the `store`
capability is untouched.

## ADDED Requirements

### Requirement: The CLI is an operator tool, never a client
The `pimdir` binary SHALL be to a store what `sqlite3` is to a database: an
inspection, repair and recovery tool. It SHALL NOT interpret item content. It
prints `seq`, `link_id`, flags, level, the object hash and the **raw** meta
string, and exports bodies as raw bytes. Rendering a message, a vCard or an
event is a per-kind consumer's job (himalaya, cardamum), never this tool's,
because a pimdir store is kind-agnostic and one binary cannot render every kind
it may hold.

### Requirement: The CLI ships behind a feature and never reaches library consumers
The binary SHALL be declared with `required-features = ["cli"]`, and the `cli`
feature SHALL NOT be part of `default`. A library consumer SHALL therefore
never compile clap or any terminal dependency. The command modules SHALL hang
off the binary crate root, not off `lib.rs`, so the library's public API is
unchanged by the presence of the CLI.

### Requirement: Reads open the store read-only
Every inspection verb SHALL open the store through the read-only role, so
inspecting a store while a sync is running is always safe and can never take
the write lock away from the owner. A verb that only reads SHALL NOT open the
store read-write "just in case".

### Requirement: Item mutations enqueue, then drain when the owner role is free
An item mutation requested from the CLI (today: `item restore`) SHALL be
appended to the action queue through the producer role, exactly as any other
non-owner process does. The CLI SHALL then attempt to take the owner role and
drain that collection itself, reporting **applied** when the item is live again
and **queued, applies at next sync** when the owner role was unavailable. The
mutation is never lost either way, and the CLI never applies a mutation behind
the queue's back.

The outcome SHALL be read from the store, not from the drain's own counters: a
drain reports what it did to the whole collection's queue, so only the item
itself proves that this action landed. A drain that ran without reviving the
item SHALL report neither success nor silence, but point at the parked-action
listing.

`item restore` SHALL rebuild the item from the retained row's own values (link
id, flags, meta, object hash) as an `add` action: the retained row still holds
them, and the store's revive branch adopts them back onto the same row. The
body is already indexed, so the enqueue references the existing object rather
than re-writing a blob.

### Requirement: The write source is resolved before anything is enqueued
A queued mutation is staged for one source, so the CLI SHALL resolve which
source it writes as before appending anything: the `--source` flag when given,
else the store's own source when it has exactly one, else a refusal naming the
candidates. A store syncing several sources SHALL NOT be guessed at, since
creating an item for the wrong side would push it to the wrong server.

### Requirement: Terminal operations take the owner role directly
Purge, queue cancellation and orphan-blob reclamation have no action kind and
cannot be queued: they SHALL take the owner role directly and fail if it is
unavailable. When another writer holds the store's write lock, the CLI SHALL
report it as a plain sentence naming the likely cause (a running sync) and
never as a raw SQL or debug error dump.

### Requirement: Destroying data is confirmed
`item purge`, `queue cancel` and the orphan-blob sweep SHALL confirm
interactively before destroying anything, stating what will be destroyed (how
many items, how many bytes) whenever the store can tell. `--yes` SHALL skip the
confirmation. When the output is not a terminal, or `--json` is set, the CLI
SHALL refuse to proceed without `--yes` rather than prompt into a pipe.

The orphan sweep carries one further guard: a body is written to the blob store
*before* the row referencing it, so a file that has just appeared may belong to
a write in flight. The sweep SHALL therefore leave orphans younger than a grace
period alone, and that period SHALL be operator-visible.

### Requirement: A listing never truncates silently
Every paged listing SHALL state what it omitted and the cursor to continue
from, in both the text and the JSON output. An operator SHALL never be able to
mistake a page for the whole set. This is what makes "the queue is empty" and
"this collection holds nothing else" trustworthy answers. A listing that could
not read part of its data (an undecodable queue payload) SHALL name what it
left out rather than return a shorter list.

### Requirement: The verb surface
The CLI SHALL expose, at minimum:

- `collection list`: id, kind, name, generation, live count and retained count.
- `item list`: a collection's live items, or its retained ones with
  `--retained`, keyset-paged with `--after` and `--limit`.
- `item show`: one item by its public `seq`, across every collection that holds
  it, retained placements included.
- `item export`: an item's body as raw bytes on stdout, or to a file.
- `item restore`: revive a retained item (see the enqueue-then-drain
  requirement above).
- `item purge`: destroy retained items, one `seq`, everything older than a
  human duration, or all of them. A live item SHALL be refused: purge empties
  the trash, it does not delete synced data.
- `queue list`: pending actions, or parked ones with `--parked`.
- `queue cancel`: drop one queue row by id.
- `store info`: schema version, sources, per-collection live and retained
  counts, object count and bytes live versus retained.
- `check`: orphan blob files, refcount drift and dangling references, with an
  optional sweep of the orphans.
- `export`: a portable dump of the store.

Every command SHALL render as JSON under `--json`, write logs to stderr only,
and carry its own `--help` text.

### Requirement: Diagnostics may read the raw index
`check` and the object figures of `store info` (object count, bytes, refcount
drift, dangling references, orphan blobs) SHALL be allowed to query the SQLite
index directly, read-only, instead of going through the library API. They are
diagnostics *about* the store's internal consistency, so exposing them as a
library API would publish invariants the library maintains rather than
observes. No other verb may bypass the library API.

The drift check SHALL count exactly what the write path maintains: an item's
body, an item's conflict copy, each source's stored base, and each queue row
pinning a body. Drift and dangling rows SHALL be reported and never repaired
automatically, since a wrong repair is worse than a reported inconsistency;
orphan blob files are the one exception, being reclaimable without a guess.

### Requirement: The dump carries metadata and bytes, nothing derived
`export` SHALL write a manifest describing the store and its collections, one
JSON-lines file of items per collection, and a copy of the blob tree. Items
SHALL be dumped exactly as stored, and bodies byte for byte. Collection files
SHALL be numbered and mapped in the manifest rather than named after collection
ids, since an id may hold anything a mailbox name may hold.

### Requirement: Import is not the CLI's job
Converting a Maildir, m2dir or vdir tree into a store (`cairn/spec/import.md`)
SHALL NOT be implemented here. Every one of its steps reads item content (a
`Message-ID` out of RFC 822 bytes, Maildir flag letters, a vCard `UID`), which
the never-interpret-content requirement forbids. It belongs in a per-kind
importer. A future `pimdir import` MAY restore a dump this tool's `export`
produced, since that reads only the store's own metadata.

## MODIFIED Requirements

None.

## REMOVED Requirements

None.
