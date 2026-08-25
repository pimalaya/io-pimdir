---
cairn: log
change: rekey-carries-the-spine
date: 2026-08-25
---

# A UIDVALIDITY bump renumbers a collection instead of freezing it

`duplicate-link-id-freeze` gave the write a floor: a binding pins one handle, and a write that would repoint it keeps the bound handle and records the incoming one, which freezes the item. Right for what it was built for, wrong for the one case where repointing is correct.

A rekey drops the whole old spine and upserts every item under its new handle, in one batch (pimdir SPEC §12), and `save_bindings_diff` compares the hub before and after the whole batch. The two halves collapsed into "same source, different handle", the floor kept the old handle, and the engine then derived nothing for the item in either direction. So an IMAP `UIDVALIDITY` bump did not renumber a collection, it froze it: every item bound to a handle the server had just voided, and no way back, since the only thing that clears an ambiguity is the source reporting the recorded handle gone and the recorded handle was the live one.

Reproduced before the fix: the two-op batch left `handle=u1 status=Ambiguous ambiguous=[101]`. The crate's own test drove exactly that batch and asserted only the generation, which is why it stayed green; neverest had recorded it as a known blocker and named this repository as the fix.

## What landed

- **`apply_ops` collects the batch's `Superseded` drop handles**, per collection, and hands them to `save_hub_diff` and `save_bindings_diff`. The hub diff cannot tell a rebuild from a duplicate out of the rows, and it does not have to: the ops carry the answer, and `ReplicaDropReason` is what the engine emits it in.

- **A superseded rebind replaces the row rather than repointing it**, which is what pimdir SPEC §10 already called the legitimate case: the binding is deleted and the new one inserted. `UPDATE_BINDING` could not have done it in any case, writing every column but `handle`, deliberately.

- **The licence is per handle.** Superseding `u1` says nothing about `u9`, so a rebuild batch carrying a genuine second copy still freezes that one, and the data-loss floor stays intact inside the one operation that legitimately repoints. Tested both ways.

- `tests/queue.rs`'s generation test now asserts the item came with the epoch.

## Verification

Test-first: both new tests were written against the old code and watched fail on the bound handle, then made to pass. 104 tests green, `cargo clippy --all-targets --all-features` clean, `cargo fmt`.

The rule is now written down on both sides: pimdir SPEC §12 states it as the format's, and io-replica's new `cairn/spec/rekey.md` states the engine's half. It had been carried by code in three repositories and by prose in none, which is why a two-line hub diff could quietly contradict it.

Capabilities moved: `store`.
