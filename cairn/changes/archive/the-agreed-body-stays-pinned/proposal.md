---
cairn: change
id: the-agreed-body-stays-pinned
status: landed
created: 2026-10-30
---

# The agreed body stays pinned

## Why

pimdir spec edit 4 (plan, 2026-10-30): the agreed (shared) body stays pinned until every binding has reconciled with it (SYNC §9), and a `prefer-*` policy keeps its losing body pinned and reports it.

io-pimdir drops a local edit when the shared item holds no body (src/hub.rs:467-494; report-io-pimdir.md, blocker 6; review-io-pimdir.md B4, code reading): with `shared = None`, `diverged` needs `shared.is_some()` and the fast-forward branch needs `!hub_moved`, so neither fires and the editing source projects `Clean`. Files always arrive bodiless, DAV does when it omits the body. The same spot reverses a per-source `PreferRemote` on a bodiless pull (S14). neverest works around the divergence in memory (driver.rs:3478-3866), and diffs `items.conflicted` around every pass because a source restating the shared body never clears it (driver.rs:3557-3570).

## What

- The shared body is pinned while a binding has not reconciled with it, so the hub always has the agreement point it merges against; a bodiless pull no longer empties it.
- The hub with no shared body reads a local edit as a divergence (`manual` records it), never `Clean`.
- `prefer-local` and `prefer-remote` keep the losing body pinned and report it (an event and a report count), as the spec states it.
- `items.conflicted` cleared when a source restates the shared body.
- neverest's in-memory workaround can go (neverest's change, once this lands).

Each needs a failing hub vector or property case first.
