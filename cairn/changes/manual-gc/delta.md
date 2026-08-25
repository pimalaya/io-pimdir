---
cairn: change
change: manual-gc
---

# Delta

## ADDED Requirements

### Requirement: A store never collects itself
No write SHALL reclaim. `write`, `write_rekeyed`, the queue drain, a cancelled action, and both purges maintain the refcounts exactly as before and delete nothing: an object at refcount zero is unreferenced, not deleted, and its body stays. That is what lets a consumer index a body in one batch and attach it in a later one, which spec §14 invites and a sweep at the end of every write silently broke, taking the bytes with it.

`PimdirStore::collect_garbage` is the collector: it drops the object rows at refcount zero and unlinks every blob file the index does not name, which is those rows' own bodies and the orphans a crash left, reporting the rows, the files and the bytes. It takes the store's staging lock exclusively, and runs on an owning handle, which already holds the owner lock: those two are what let it reclaim with no grace window, since neither an owner nor a producer can be mid-write while it sweeps. A period-prefixed temporary file belongs to a writer that has not committed and is left alone.

Unreferenced objects accumulate until someone collects. A consumer that wants that bounded schedules the verb.

### Requirement: The store repairs what it can recompute
`recompute_refcounts` settles every object's count from the four columns that pin one (spec §7), returning how many disagreed; `clear_dangling_bindings` deletes the bindings whose item is gone, returning how many. Both recover a fact the store already holds. Every other dangling row stays reported and untouched: an item whose object row is missing is still the item, and a queue row whose body is missing is still an intent somebody expressed, so deleting either would destroy data rather than repair it.

### Requirement: The collector is a verb
`pimdir gc` reclaims: object rows at refcount zero with their bodies, plus orphan blob files, reporting what it freed. It takes the owner role, so it never runs beside a sync, and reports a producer mid-append as such rather than waiting on it.

## MODIFIED Requirements

### Requirement: A write batch is one transaction
Unchanged except for reclamation, which leaves it: zero-refcount objects are no longer collected inside the transaction and no blob file is unlinked after the commit. The refcount maintenance, the diffed save, the incremental cross-collection correctness, the retention of an item no source holds, `BEGIN IMMEDIATE` and the busy timeout are all as they were.

### Requirement: `check` is diagnosis, `--fix` is repair
`pimdir check` reports what should not happen: object rows whose body is missing, refcount drift, dangling rows, and the ambiguous bindings it already listed. Orphan blob files are reported too, pointing at `pimdir gc`. `--fix` repairs rather than reclaims — it recomputes the drifted refcounts and clears the dangling bindings — so it needs neither a grace period nor a confirmation, and `--grace` and `--yes` are gone from the verb.

### Requirement: Purge reports what it retired
`item purge` and the store's two purge operations report the rows they retired, not bytes reclaimed. A purge releases the references a retained item held; the bytes are freed by the collector, which is what reports them.

## REMOVED Requirements

None.
