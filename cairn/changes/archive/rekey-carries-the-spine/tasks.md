---
cairn: tasks
change: rekey-carries-the-spine
---

- [x] Reproduce: a rekey batch leaves the binding on the voided handle, ambiguous
- [x] Collect the `Superseded` drop handles per collection in `apply_ops`
- [x] Thread them through `save_hub_diff` into `save_bindings_diff`
- [x] Replace the binding (delete + insert) rather than repoint it, per SPEC §10
- [x] Test: a rekey carries the binding over, clean, with no ambiguity
- [x] Test: the licence is per handle, so a duplicate in the same batch still freezes
- [x] Give `tests/queue.rs`'s generation test the assertion it was missing
