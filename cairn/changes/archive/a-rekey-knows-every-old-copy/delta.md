---
cairn: delta
change: a-rekey-knows-every-old-copy
---

## ADDED Requirements

### Requirement: A rebuild knows every old copy of an identity by its key
A rebuild SHALL find the old copies of a member's identity by the key each carries, never by the handle it holds now: the placement keyed by the hint, and each keyed by a key minted from it over whatever handle (`PimdirLinkId::minted_from`), a rekey carrying the key and not the handle. A minted key whose `#` could end several keys goes to the longest one the listing names. Among the copies the member takes the one its revision or body tells, else the bare key while an old row holds it or no copy is left, else the first remaining copy in handle order, so a copy whose bare sibling is gone is carried too, and only then a mint of its own. A second rekey therefore carries a copy the first rekey carried, a copy staged beside the identity and delivered, and a lone minted copy, with their bodies and pending state, and no other source of the collection is told to remove one.

#### Scenario: The copies a first rekey or a copy left
- GIVEN one mailbox holding a `Message-ID` twice, hydrated, rekeyed once, a flag then staged on the minted copy; a `Copy` staged beside the identity and delivered; a minted copy whose sibling was deleted
- WHEN the server renumbers and the collection is rekeyed
- THEN every copy is carried (`rekeyed` counts it, `pulled` and `dropped` do not), with its body and staged flag, nothing retained, and a second source mirroring the mailbox keeps both messages on its server (tests/engine/rekey_copies.rs)

### Requirement: A rebuild grows with the collection
A rebuild SHALL index the old copies by identity once (`PimdirRekey::copies`) rather than scan every old placement per listed member, and the hub's absorb SHALL find the binding a drop or a delivery names through an index of the batch's handles, checked to bind it still, rather than walk every item per drop; so a rekey of a whole collection, every handle dropped then upserted, costs in the engine and in the store's write what the collection's size does, not its square.

#### Scenario: Measured
- GIVEN mailboxes of 2,000 to 16,000 distinct messages, renumbered
- WHEN each is rekeyed
- THEN the coroutine's and the write's times grow about as the collection does (tests/engine/rekey_scale.rs, ignored, run with `--release --ignored`)

## MODIFIED Requirements

## REMOVED Requirements
