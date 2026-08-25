---
cairn: change
id: rekey-carries-the-spine
status: landed
created: 2026-08-25
---

# A handle-space rebuild froze every item of its collection

## Why

`duplicate-link-id-freeze` gave the write a floor: a binding pins one handle, and a write that would repoint it keeps the bound handle and records the incoming one instead, which freezes the item. That is right for what it was built for, a source holding one identity under two handles, where repointing destroyed the evidence at the write.

It is wrong for the one case where repointing is correct. A rekey drops the whole old spine and upserts every item under its new handle, in one batch (pimdir SPEC §12), and `save_bindings_diff` compares the hub before and after the whole batch: the two halves collapse into "same source, different handle", the floor keeps the old handle and records the new one as ambiguous, and the engine then derives nothing for the item in either direction.

So an IMAP `UIDVALIDITY` bump does not renumber a collection, it **freezes** it: every item bound to a handle the server has just voided, every item ambiguous, and no way back, since the only thing that clears an ambiguity is the source reporting the recorded handle gone and the recorded handle is the live one.

Reproduced against the working tree: a two-op batch (`DropPlacement { reason: Superseded }` plus an upsert under the new handle) leaves `handle=u1 status=Ambiguous ambiguous=[101]`. The crate's own test drives exactly that batch and asserts only the generation, which is why it passed. neverest recorded it as a known blocker and named this repository as the fix.

## What

The hub diff cannot tell the two apart from the rows, and it does not have to: the ops carry the answer. `apply_ops` collects the handles this batch dropped with `ReplicaDropReason::Superseded`, per collection, and hands them to `save_hub_diff` and `save_bindings_diff`.

A binding whose handle moved off a superseded one is **replaced** rather than repointed, which is what pimdir SPEC §10 already says a legitimate rebind is: the row is deleted and the new one inserted. `UPDATE_BINDING` could not have done it in any case, since it writes every column but `handle`, deliberately.

The licence is per handle. Superseding `u1` says nothing about `u9`, so a rebuild batch that also carries a genuine second copy still freezes that one, and the data-loss floor stays intact inside the one operation that legitimately repoints.
