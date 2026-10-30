---
cairn: delta
change: an-automatic-settle-keeps-what-it-discards
---

## ADDED Requirements

### Requirement: An overruling edit keeps what it discards
An `Edit` stating `overrule`, a settle by a rule of the engine's or a connector's (a one-way mode restating the body its authority holds), SHALL keep each diverging body it does not adopt, the binding's `conflict_object` then the item's, each once, as a retained item under `dup:<link>#<edited handle>`, minted again over a held key, its summary derived from the body, reported `Overruled` on `PimdirMutateReport::events`, before any body the collection's policy keeps in the same write (pimdir SYNC §7, §9). The mutation stages `PimdirWriteOp::Overrule { collection, handle }` before its upsert, and the write reads the conflict there, the projection carrying no item conflict body. An overruling edit restating the shared body over the item's conflict clears it, the source's `shared_object` moving to that body. It SHALL be refused, staging nothing, as `PimdirError::Unread` (`conflict-unread`) while the binding's or the item's conflict holds no diverging body. Without `overrule` an edit is a person's choice and keeps nothing.

#### Scenario: The vectors
- vectors/sync 100 (a binding conflict), 101 (refused over an unread body), 102 (an item conflict), 103 (no overrule keeps nothing), tests/vectors_sync.rs; tests/overrule.rs `an_overruling_edit_over_an_unread_item_conflict_is_refused`
