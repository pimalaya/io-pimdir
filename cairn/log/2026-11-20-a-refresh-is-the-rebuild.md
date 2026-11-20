---
cairn: log
change: the-crate-follows-draft-15
landed: 2026-11-20
---

# A refresh is the rebuild

The generated suite's G9 and G10 (tests/generated/index.rs, a refresh after each owner write against a rebuild), decided by the lead toward exactness and stated in pimdir search-draft-15 (pimdir cairn/log/2026-11-20-a-refresh-is-the-rebuild.md).

- G9, src/search/refresh.rs: a body a placement of the page left (a dropped row, a gone placement) or moved off (a repointed one) is a `PimdirIndexOp::DropUnplaced` once the page placed what it holds, which src/client/search/refresh.rs applies as `DropObject` when `object_placed` finds no placement naming it; one put earlier in the run is put again should a later page name it. `PimdirIndexGonePlacement` carries the placement's `hash`, which `placements_gone` now answers.
- G10: every mail placement a batch folds is touched, so its thread is re-keyed and re-bounded from its members' sort keys as they stand (src/search/thread.rs).
- tests/search.rs reads refresh.json at `v` 2: a query's `threads`, a step's `objects` against the `object` rows of the refreshed and of the rebuilt index. The second and third cases fail with `DropUnplaced` emitted as nothing and the touch condition of before ("object rows: refreshed"; "\"thread:basic-1@example.org\": refreshed") and pass.
- tests/generated: G9 and G10 lifted (index.rs), and G4, G5 and G6, which the generator now follows, lifted too (overlay.rs, drain.rs); index.rs follows the generator's `bindings` list.
- spec/ re-vendored: spec/REVISION and spec/generator at pimdir f56aaf6, spec/queries and spec/vectors from its working tree (`object_placed`, `placements_gone`'s `hash`, refresh.json 2).

Capabilities moved: search.

## The hostile zone and the generator's new expectations

- pimdir vectors/search/refresh.json's fourth case files event-zoned-hostile.ics (G8): tests/pending_vectors.txt lists it, and tests/search.rs runs no step of a case it lists (`pending::is_listed`), the case exhausting memory rather than failing an assertion; it stays listed until the crate takes the ical-rs that bounds the zone walk, moving `EXTRACTOR` then. checks/implementation.sh fails while it is listed.
- `PimdirReader::pending_arrivals` answers `load_pending_arrivals`, which the generator's oracle states; tests/generated/reader.rs maps it, and lists `load_conflict`, an owner read no reader call wraps, as unwrapped.
