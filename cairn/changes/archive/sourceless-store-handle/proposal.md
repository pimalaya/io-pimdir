---
cairn: change
id: sourceless-store-handle
status: landed
created: 2026-08-25
---

# An operation with no side does not name one

## Why

`PimdirStore::open(dir, source)` binds a handle to one source, and most of what a store does genuinely is "act as this side": `load` projects the shared item through that side's binding, so one item reads Clean to one source and Dirty to another; `write` folds that side's decisions back; `drain` stages a queued action for it; `rekey` rebuilds its handle space. None of those mean anything without a side.

Purge is not one of those. It removes an item every source has already dropped, retained with no bindings left, and it reads `self.source` nowhere. Neither does a queue cancellation, an orphan sweep, or any read. The constructor demands a source anyway, so every caller has to invent one:

- `owner_any_source` takes whichever source it finds first, for `item purge` and `queue cancel`. The value is passed in and never read.
- `read`, used by every read-only command, passes the literal string `pimdir` (`FALLBACK_SOURCE`).
- To find that first source, the CLI runs `distinct_sources`, a `SELECT DISTINCT source FROM bindings`: a full scan of the largest table beside `items`, on a store where the answer is then discarded.

A read-only handle inventing a source name is the shape of the problem. The scan is only its most visible cost.

## What

Split the handle by what an operation actually needs.

- **Source-bound**, unchanged: `load`, `write`, `write_rekeyed`, `drain_collection`, and the queue-apply path behind it.
- **Source-less**: `purge`, `purge_retained_before`, `revive_item`, `drop_action`, the whole client read surface, and the coming `gc` and `check`.

Concretely, `PimdirStore::open(dir)` takes no source and yields a handle carrying none, with `for_source(source)` producing the source-bound one beside the existing `for_account(account)`. The source-bound operations move to that type, so a caller that never names a source cannot reach them, and one that does cannot forget.

`FALLBACK_SOURCE` is deleted: nothing needs to invent a name once nothing asks for one that has no meaning.

`distinct_sources` survives for `store info` and `export`, where listing the store's sources *is* the answer the user asked for, and where a scan is what they asked for too.

## Scope / non-goals

- The `sources` table keeps its meaning ("has a checkpoint"). An earlier draft of this proposed making it authoritative for "is known to this store" so the scan could read it instead; that is unnecessary once nothing scans to pick a source it will not use.
- No change to what a source *is*, to bindings, or to the projection.
- `item restore` stays source-bound and keeps refusing a multi-source store without `--source`: it stages a creation *for* a side, so the question is real there.
