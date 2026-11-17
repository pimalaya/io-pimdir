---
cairn: change
id: a-rekey-knows-every-old-copy
status: landed
created: 2026-11-17
---

# A rekey knows every old copy

## Why

CLI family audit of 2026-11-15, PIM-7 and PIM-9, open at dcf49e2 (src/rekey.rs unchanged since c40e01d). A rekey recognised an old minted copy only when its key equalled the hint minted over the handle it held now, but a carry keeps the key: a second rekey of a collection holding a `dup:` copy, a copy staged beside a held identity, and a lone minted copy were dropped `Deleted` and pulled fresh, losing body and pending state, and on a shared collection made the other source push a `Remove`. neverest's NEV-4 (keying contacts from their whole body) rekeys and waits on it. Beside it the rebuild was quadratic: `key_of` scanned every old placement per member and the hub's `absorb_drop` walked every item per drop.

## What

- src/placement.rs: `PimdirLinkId::minted_from`, the keys a key may be minted from, longest first.
- src/rekey.rs: `PimdirRekey::copies` indexes the old copies by the identity the listing names once; `key_of` reads that index, carries a lone minted copy before minting its own.
- src/hub.rs: `absorb` keeps an index of the batch's handles, `absorb_drop` and `absorb_delivery` look the binding up, checked to bind it still.
- Tests: tests/engine/rekey_copies.rs (the skeptic's four cases, failing on dcf49e2), tests/engine/rekey_scale.rs (ignored, measuring; release, 16,000 members: coroutine 5,274 ms to 65 ms, store write 2,181 ms to 587 ms).
- PIM-13 (three recognitions of a minted key) is not merged: the sync's two (`take_pending_create`, `disagreeing_create`) recognise only the mint over the provisional handle, which SYNC §6 asks for, a different question from the rekey's.
