---
cairn: log
change: as-fast-as-notmuch
landed: 2026-11-21
---

# The owing rows are marked

pimdir's the-owing-rows-are-marked (storage-draft-15 edited in place) vendored: spec/migrations/storage/0001_init.sql, spec/queries/storage/owner/list_owing_bindings.sql and list_lacking_items.sql, byte copies; vectors/, payloads/ and queries/signatures.json do not move. `bindings.owes` and `items.lacks` are kept by the schema's triggers, so no crate code moved: the delta load (`PimdirLoadSelection::Owing`, src/client/write.rs) runs the two statements as before and they now seek `bindings_owing` and `items_lacking`. A store created before it is refused as damaged, its schema text differing from the canonical one (src/client/schema.rs). spec/REVISION still names the revision before it, until the owner commits pimdir.

tests/statement_scale.rs times `list_lacking_items` beside `list_owing_bindings`.

## Measured

On the 517k tier, core 12, the same afternoon, against this crate built without the change: the no-op sync 0.12 s wall and 0.10 to 0.11 s user, from 0.39 to 0.43 s and 0.35 to 0.36 s (0.49 s in 2026-11-21-a-sync-that-finds-nothing, an earlier day; notmuch 0.07 s); 1,000 flag changes 0.19 s from 0.48 s; the import, under eatmydata in alternating pairs, about 4% to 11% more CPU (user +4.9% and +10% in the two pairs no I/O burst disturbed, wall +4.0% and +7.7%). pimdir's log entry of the same name holds every run, the statements' own times and where the import's cost goes, including the SQLite temporary files a statement firing a writing trigger journals into, which this crate leaves on the default `temp_store`.
