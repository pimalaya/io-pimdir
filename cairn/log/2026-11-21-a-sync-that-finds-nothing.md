---
cairn: log
change: as-fast-as-notmuch
landed: 2026-11-21
---

# A sync that finds nothing

Phase 3's "no-op sync's fixed cost per collection" of as-fast-as-notmuch. On the a135cac store of the 517k Enron tier (3,311 Maildir folders, 3,312 collections), core 12, warm cache, a sync with nothing changed took 5.0 s wall, 2.2 s user and 0.33 s system, with 3,325 `fsync`s (io-pimdir 78483f0; 3.7 s in results-2026-11-21's run). notmuch's no-op `new` takes 0.07 s; the gate is within ten times.

## Where it went

Split with temporary timers in the bench (getrusage around each yield) and a callgrind profile of the run, CPU in ms:

- the bench connector listing every folder: 410 (`cur/` and `new/` read, the sidecar read and written again, 21 MB);
- the engine's first load (`Handles` naming none): 560;
- the delta load (`Owing`): 1,170;
- the write of each delta's checkpoint: 110, and one `fsync` each, about 1.2 s of wall;
- `prepare_sync`: 130; `ensure_collection`: 40; the folder walk: 17; process setup: 2.

callgrind put 55% of all instructions in `sqlite3Reprepare`: every cached load statement was compiled again at every step. SQLite marks a statement to recompile on rebinding when the planner compared one of its parameters with a partial index's literal condition (`collection = :collection` beside `items_deleted`'s `deleted = 1`, the column fixed by constant propagation): 18 µs instead of 3 for `load_mail_summaries_by_link`, 31 instead of 2 for `list_superseded_bindings`.

## What landed

- Every connection turns on the query planner stability guarantee (`SQLITE_DBCONFIG_ENABLE_QPSG`, `prepare_connection`): 2.5 s of CPU to 1.05 s, 5.0 s wall to 3.2 s. No canonical statement binds a value a partial index's condition could match and no store carries `sqlite_stat4`, so no plan moves; the query table, rerun on the same store, has no count slower and its pages and threaded pages 5 to 60% faster, the same recompiling gone.
- A delta leaves out `SetCheckpoint` when its checkpoint is the one the source holds, and the engine yields no last write that holds no op: a delta that found nothing takes no transaction and no `fsync`.
- `load` with `Handles` naming none reads the sync state and the refusals alone (560 ms to 17 at that point), and every load runs in one read transaction, one snapshot and one set of locks (35 ms of the wall).
- The bench connector (bench/src/main.rs, bench/README.md) skips a folder whose `cur/` and `new/` modification times are those of its last listing, as `notmuch new` does, the times kept in its state dir's `%mtimes` once the folder's sync landed and only when 2 s old; a delta finding nothing hands back its checkpoint, as IMAP's `HIGHESTMODSEQ` or a JMAP state that did not move. It no longer rewrites the sidecars of folders that did not change.

Together: 5.0 s wall, 2.6 s CPU and 3,325 `fsync`s to 0.49 s, 0.48 s and none, seven times notmuch's 0.07 s. The incremental steps of incr.sh, run on a hardlinked copy of the tree under noop-work (corpus/x1 untouched), still answer: 1,000 new messages land (1.3 s), 1,000 flag renames move `\Flagged` from 1,419 to 2,419 (0.67 s), 100 deletions take the folder from 1,387 to 1,287 (0.54 s), and the no-op after them is 0.53 s; a fresh import of 10 folders lists every one and the next no-op skips them.

Capabilities moved: sync.md ("A delta that moved nothing writes nothing"), seam.md (the load's snapshot and its empty selection), store.md ("A cached statement keeps its plan"). Checks: src/sync.rs `a_delta_finding_nothing_under_the_same_checkpoint_writes_nothing` and `a_delta_pulling_under_the_same_checkpoint_writes_the_pull_alone`, src/client.rs `a_cached_statement_is_not_compiled_again_per_binding`, each failing against 78483f0; `a_delta_finding_nothing_under_a_new_checkpoint_lands_it` guards the other side.

## Left for the owner

What is left is SYNC §10's delta load: `list_owing_bindings` and `list_lacking_items` walk every binding and every item of the collection (0.25 s and 0.23 s over the 517k bindings measured alone, 0.43 s of the 0.51 s of CPU now, 84%), to answer nothing on a no-op. The load grows with what moved; these two statements grow with the collection. Both are canonical, so the change is pimdir's. Two shapes to weigh:

- an owing mark kept by the store: a column on `bindings` set by the triggers that already watch items, flags and bases, with a partial index `(collection, source) WHERE owes = 1`, and a count of a collection's live bodied items no source binds, so both reads seek what is there;
- a stamp per source and collection of the change feed's position (`items.changed`) when its last delta load found nothing owing and nothing lacking, both reads skipped while no item of the collection changed since; cheaper to write, but a binding's base moving without its item's stamp would have to move it too.

Either takes the no-op to about 0.08 s, near notmuch.
