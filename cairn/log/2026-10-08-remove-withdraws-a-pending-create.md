---
cairn: log
change: remove-withdraws-a-pending-create
date: 2026-10-08
---

# A remove withdraws a pending create in its own write

pimdir SYNC §7: a `Remove` on a pending create withdraws it, the binding going and the item retained. `PimdirMutate` tombstoned it instead, so the provisional binding stayed until a sync dropped it, for good in a collection no sync visits (an on-device address book); a `Remove` of the copy the hub offers a source lacking the item even bound it. Pimalaya Android worked around both in its bridge.

## What landed

- **The mutation** (`mutate`): a `Remove` leaving no base (a conflict still settles into one first) writes the tombstone and then a `Deleted` drop of its handle, one batch: the item is retained when nothing else binds it, and every other binding pushes the delete otherwise (SYNC §9).
- **The load** (`seam`): a `Handles` load naming a provisional handle the source does not bind returns the copy the hub offers under it; it answered nothing, and the `Remove` failed `UnknownHandle`.
- **Tests**: vectors/sync/51 (new upstream, failing before); `mutate::tests::remove_withdraws_a_pending_create`; tests/engine/hub.rs covers an added create removed, an offered copy removed (the holder pushes the delete) and a re-staged create removed beside a based binding, each failing before.

## Verification

`cargo fmt`, `cargo clippy --all-targets --all-features` and `cargo test --all-features` green; `nix flake check` in pimdir green with vector 51.
