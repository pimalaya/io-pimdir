---
cairn: change
id: an-automatic-settle-keeps-what-it-discards
status: landed
created: 2026-10-30
---

# An automatic settle keeps what it discards

## Why

pimdir sync-draft-05 and storage-draft-05 gained a rule the same day (its log an-automatic-settle-keeps-what-it-discards, 2026-10-30): neverest's one-way mode settles a target's conflict with an `Edit` restating the store's body, and SYNC §7 made the diverging body the base and kept nothing, so the push collected it. An `Edit` settling a conflict by a rule of the engine's or a connector's now states `overrule`: it keeps each diverging body it does not adopt, the binding's `conflict_object` then the item's, as a retained item keyed over the edited handle, reported `Overruled`, before any body the collection's policy keeps in the same write, and is refused, staging nothing, while the conflict holds a body nobody has read. Vectors 100 to 102 fail here ("events differ", "the run failing differs"); 103 passes.

## What

- `PimdirMutation::Edit { .., overrule: bool }`; the harness reads `overrule` from a vector's mutation.
- The mutation stages `PimdirWriteOp::Overrule { collection, handle }` before its upsert; the projection carries no item conflict body, so the write, which reads the hub, decides what is kept and what refuses.
- The hub keeps, before the upsert applies, the binding's `conflict_object` then the item's, each once and neither the body adopted, as `PimdirOverruled`; the write derives the kept body's summary from its blob. An overruling edit restating the shared body over the item's conflict clears it, the source's agreement point moving to that body.
- The write refuses an overrule over a binding or an item conflict holding no diverging body as `PimdirError::Unread` (`conflict-unread`).
- Migration: neverest's `keep_store_bodies` states `overrule: true` and fetches a placement's diverging body before settling it; every other `Edit` literal names `overrule: false`.
