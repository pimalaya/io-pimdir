---
cairn: log
change: the-crate-follows-draft-15
landed: 2026-11-20
---

# Keep local

pimdir sync-draft-15 lands the owner's D2 (pimdir cairn/log/2026-11-20-keep-local.md): `KeepLocal`, a source policy for a source the store is authoritative over, under which a change made on that remote alone is a divergence, never pulled. spec/vectors re-vendored from the pimdir working tree: the sync format moves to 9 for the run policy `keep-local`, and vectors 173 to 175 join it.

- src/sync.rs: `PimdirConflictPolicy::KeepLocal`. On the content axis, a remote revision the base does not hold over a placement holding a body, with no local edit, is resolved as a both-changed case under it; `resolve_conflict` settles it as `PreferLocal`: the member's carried body kept by an overruling write and the base moved onto it before the store's body is pushed gated on the observed revision, a member carrying no body or a source that may not push content recorded as `Manual` records it. A placement holding no body pulls as any.
- src/upgrade.rs `settle`: `KeepLocal` settles a fetched diverging body as `PreferLocal` does, and as `Manual` without content pushes.
- tests/vectors_sync.rs reads format 9 and maps `keep-local`. Vectors 174 (the carried body kept, the store's pushed over it) and 175 (a body-less member recorded, nothing pulled or pushed) fail with the content axis pulling the remote-only change as before, and pass; 173 (an upgrade reading its base keeps the item conflict, the case of cairn/log/2026-11-20-a-base-read-keeps-the-item-conflict.md as a two-source vector) fails with that fix disabled, and passes.
- cairn/spec/sync.md (the policy requirement, the vector format at `v` 9) and cairn/spec/hub.md name them.
- spec/generator re-vendored and spec/REVISION moved to pimdir 467b534, the generator covering every read, the queue, the drain, the overlay and the capabilities for the differential tests (pimdir the-freeze-plan, section 3). spec/vectors is ahead of that commit by this draft's sync vectors until pimdir commits them, `the_vendored_copy_is_the_spec_byte_for_byte` failing locally until spec/REVISION names that commit.

Breaking: an exhaustive `match` on `PimdirConflictPolicy` needs the new arm (neverest matches it).

Capabilities moved: sync, hub.
