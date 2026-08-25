---
cairn: change
id: manual-gc
status: active
created: 2026-08-25
---

# A store never collects itself

## Why

Every write transaction sweeps. `collect_garbage` runs inside `write`, `write_rekeyed`, `apply_queued`, `drop_action`, `purge` and `purge_retained_before`: it deletes every object row at refcount zero and unlinks its blob after the commit.

That makes a rule the format states elsewhere impossible to obey. SPEC.md §14 invites a consumer to stream a body straight to its sharded path and index it without holding it whole, and `STORE_OBJECT` inserts at refcount zero because references come from placement pointers only. So a consumer that stores bodies in one batch and attaches them in a later one has them deleted at the end of the first, silently, bytes included. §5 permits the sweep and §14 invites the pattern it breaks; the crate's own tests already note it.

Two verbs also disagree about what reclamation is. The automatic sweep runs with no grace and no confirmation, while `check --fix` reclaims orphan blob *files* behind a grace window and a TTY prompt. The routine operation is the unguarded one and the exceptional verb is the careful one, which is backwards.

The alternative the audit offered was a grace window on the automatic sweep. Nix suggests the better shape: a store that never collects itself, and a collector that runs when asked.

## What

- **No write collects.** `collect_garbage` leaves `write`, `write_rekeyed`, `apply_queued`, `drop_action`, `purge` and `purge_retained_before`. Refcounts are still maintained exactly as now; only the reclamation goes. An object at refcount zero is simply unreferenced, and stays until someone collects.
- **`pimdir gc`**, the collector: object rows at refcount zero with their blobs, plus orphan blob files (a file no row references). Reports what it freed. This is the routine, expected-to-find-something verb.
- **No grace window.** `gc` takes the exclusive owner lock (`single-owner-lock`), so it cannot run while a sync holds the store, and a producer's blob-write-and-enqueue is atomic against it under the shared lock. The lock is the GC root; a timer was standing in for one.
- **`check` becomes diagnosis, `--fix` becomes repair.** It reports what should never happen (object rows whose blob is missing, refcount drift, dangling rows, and the ambiguous bindings it already lists), and `--fix` repairs drift by recomputing refcounts from the pointer columns, which §7 permits and nothing does today, and clears dangling rows. It reclaims nothing, so it needs neither grace nor prompt.
- **`purge` and `purge_retained_before` report rows retired**, not bytes reclaimed: the bytes are the collector's to report now. An output-type change with a `json_schema.rs` entry.

## Scope / non-goals

- Unreferenced objects accumulate until someone collects. That is the bargain, and it is the one nix makes; a client that wants it bounded schedules the verb.
- Consumers should surface it. neverest and himalaya are where a user would expect to reach it, and neither exposing it is how a store grows quietly.
- The refcount discipline itself does not change: this removes a sweep, not the counting.
