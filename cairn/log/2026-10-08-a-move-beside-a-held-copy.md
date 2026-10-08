---
cairn: log
change: a-move-beside-a-held-copy
date: 2026-10-08
---

# A move beside a held copy relocates rather than deletes

pimdir's `a-move-beside-a-held-copy` (still draft-04) pairs a key with the create minted over its provisional handle for the origin, the destination and the landing, and leaves the remove's `link_id` out when its destination is such a create. Before it, a move of a body-less member into a collection already holding its identity derived a plain delete, and nothing could deliver the create: the message was lost on the server.

## What landed

- **Vendored spec** (`spec-fidelity`): `origin_for_link` and `destination_for_link` match the minted form, the bare key first, and the destination returns the create's handle.
- **The move** (`sync`, `hub`, `mutate`): a tombstone's destination names the pending create's handle, the store's and the one `Move` stages; `Remove::link_id` is `None` when that handle is not the tombstone key's provisional one. The landing by a minted key (2eb6d42) is now stated.
- **Tests**: vectors/sync/48, 49 and 50 run green; tests/engine/membership.rs covers a cold move beside a held copy source first and target first, failing without either half; the `LocalMoveCold` property drops its held-target exclusion and checks the copies of the link are kept, excluding only a target already holding the once-minted key; the fake remote gives a second copy of one origin a fresh handle; tests/roundtrip.rs reads the destination under the create's handle.

## Verification

`cargo fmt`, `cargo clippy --all-targets --all-features` and `cargo test --all-features` green.

Capabilities moved: `sync`, `hub`, `mutate`, `spec-fidelity`.
