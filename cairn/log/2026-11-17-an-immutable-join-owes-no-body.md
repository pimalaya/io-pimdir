---
cairn: log
change: an-immutable-join-owes-no-body
landed: 2026-11-17
---

# An immutable join owes no body

CLI family audit of 2026-11-15, PIM-6, open at dcf49e2. A mail member naming a message another source holds, joined at the write on a page before the last or listed by both sources before either hydrated it, got a base naming no body and projected `Dirty` (hub.rs compared bodies whatever the kind, where SYNC §3 rule 4 compares them for a mutable kind only), so the engine derived an `Update` every run, which neverest's IMAP connector rejects, and no flag change ever pushed for that message. On a first pairing the flag winner also depended on the page position, neither being the union SYNC §5 gives.

## What landed

- src/hub.rs: `PimdirHub::immutable`; the projection compares no body for an immutable collection, the absorb adopts the shared body as the base of such a binding whatever its own named, and joins the flags of a source binding the item for the first time as a union.
- src/client/write.rs: `read_hub` sets `immutable` for `message/rfc822`. Bindings already written with a body-less base project `Clean` now, so no store rebuild is needed.
- src/sync.rs: a member landing a `Created` placement the store offers this source lands it with the union of both flag sets.
- Tests: tests/engine/join_immutable.rs (the skeptic's scenarios, with a first pairing in both page positions).

## Capabilities moved

hub ("An immutable binding owes no body", "A source binding an item for the first time joins its flags as a union", added), sync ("An offered create is landed with the union of its flags", added).

In the working tree on io-pimdir dcf49e2 (0.15.1), uncommitted, no version bumped; CHANGELOG.md [Unreleased] names it.
