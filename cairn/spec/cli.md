---
cairn: spec
capability: cli
status: current
---

# Operator CLI

The `pimdir` binary: the operator front-end over a pimdir store. It ships from
this crate behind the `cli` feature and is built on the same public library
surface a consumer uses, plus a read-only diagnostic connection for the
questions the library has no reason to answer.

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

### Requirement: Reads open the store through the reader role
Every inspection verb SHALL open the store through the reader role
(`PimdirReader`), so inspecting a store while a sync is running is always safe
and can never take the write lock away from the owner. A verb that only reads
SHALL NOT open the store read-write "just in case".

Those reads SHALL NOT overlay the pending queue: an operator inspects the store
as it stands, and reads what is queued through `queue list`, where a pending row
shows as a row rather than as a fact about an item.

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
else the store's own source when it has exactly one, else a refusal: naming the
candidates when there are several, asking for the flag when the store has synced
none. A store syncing several sources SHALL NOT be guessed at, since creating an
item for the wrong side would push it to the wrong server, and a store with no
source at all offers no side to act as.

What follows the enqueue is not a source question but an ownership one:
`item restore` SHALL append its action first and take the owner role only to
apply it, reporting the action as queued when another process owns the store
rather than failing. The action is in the queue by then, and the owner holding
the store is the one that will drain it.

### Requirement: Terminal operations take the owner role directly
Purge, queue cancellation, repair and collection have no action kind and
cannot be queued: they SHALL take the owner role directly, without naming a
source, and fail if the role is unavailable. When another process owns the
store, or another writer holds its write lock, the CLI SHALL report it as a
plain sentence naming the likely cause (a running sync) and never as a raw SQL
or debug error dump.

### Requirement: Destroying data is confirmed
`item purge` and `queue cancel` SHALL confirm interactively before destroying
anything, stating what will be destroyed (how many items, how many bytes)
whenever the store can tell. `--yes` SHALL skip the confirmation. When the output
is not a terminal, or `--json` is set, the CLI SHALL refuse to proceed without
`--yes` rather than prompt into a pipe.

`gc` and `check --fix` SHALL NOT ask. A collection destroys nothing a caller
could want: it takes what the index says nothing references, under the locks that
prove nobody is mid-write, and a store that asks before doing its routine
housekeeping teaches an operator to answer yes without reading. A repair destroys
nothing at all.

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
  it, retained placements included, each with the bindings its sources hold it
  under.
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
- `check`: object rows whose body is missing, refcount drift, dangling
  references and orphan blob files, with `--fix` repairing the drift and the
  dangling bindings, plus an informational count of the minted keys each
  collection holds, which is a fact rather than a problem.
- `gc`: reclaim the object rows nothing references, their bodies, and the orphan
  blob files a crash left, reporting what it freed.
- `export`: a portable dump of the store.

Every command SHALL render as JSON under `--json`, write logs to stderr only,
and carry its own `--help` text.

### Requirement: Diagnostics are a library read
`check` and the object figures of `store info` (object count, bytes, refcount
drift, dangling references, orphan blobs) SHALL come from the library's
diagnostics surface, on the handle the verb already holds. No verb opens a
second connection to the index, and none bypasses the library API.

That surface was the operator tool's own read while the library only
*maintained* those invariants; it belongs to the library now that the library
also repairs them, since a repair whose findings a caller cannot read is the
worse seam.

The drift check SHALL count exactly what the write path maintains: an item's
body, an item's conflict copy, each source's stored base, and each queue row
pinning a body. `--fix` SHALL repair only what the store can recompute from what
it already holds: the drifted counts, and the bindings whose item is gone. Every
other dangling row SHALL be reported and left alone, since a wrong repair is
worse than a reported inconsistency, and reclaiming is not repairing: a body is
`gc`'s to take.

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

### Requirement: A binding is readable, and `item show` prints it
The library SHALL expose every source's binding of one item, keyed by source:
the handle that source addresses it by, the base the last sync agreed on
(flags, body and revision, and whether a base exists at all), and the exception
marker `conflicted` with the revision observed when it was recorded.

`item show` SHALL print one block per binding under each placement it names, in
text and under `--json`. The exception line SHALL be printed only when it
applies, so a diverged binding stands out from the ordinary ones beside it
rather than being one more line that is usually empty.

This is what makes a duplicated identity actionable. `check` counts the minted
keys a collection holds, and the next question is always which resource each
copy came from; the binding is the only thing that says so, and without it the
only answers were the server and the database file.

`item list` SHALL NOT carry bindings. Its rows are a page served by one query,
and a per-row binding lookup would make a listing cost what a listing must not.
The verb that names one item is the one that can afford to say everything about
it.

#### Scenario: A minted copy names the resource it came from
- GIVEN a source holding one identity under two handles, the second filed under a minted key
- WHEN `item show` names each of the two items
- THEN each prints the binding of its own handle

#### Scenario: Each source reports its own view
- GIVEN two sources holding one item, one of them diverged from its own remote
- WHEN `item show` names that item
- THEN each source prints its own handle and base, and only the diverged one prints a conflict
