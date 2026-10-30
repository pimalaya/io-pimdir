---
cairn: tasks
change: an-automatic-settle-keeps-what-it-discards
---

- [x] Vectors 100, 101, 102 green (failed before: "events differ" for 100 and 102, "the run failing differs" for 101); 103 green
- [x] `PimdirMutation::Edit::overrule`, `PimdirWriteOp::Overrule`, the hub's keep before the upsert, the summary derived from the kept body
- [x] `PimdirError::Unread` for an item conflict holding no diverging body; tests/overrule.rs `an_overruling_edit_over_an_unread_item_conflict_is_refused` (fails with the check removed: the overrule applied)
- [x] Spec: mutate.md folded; log; CHANGELOG (breaking: the `Edit` literal, the `PimdirWriteOp` variant)
