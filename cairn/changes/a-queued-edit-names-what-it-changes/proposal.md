---
cairn: change
id: a-queued-edit-names-what-it-changes
status: landed
created: 2026-10-30
---

# A queued edit names what it changes

## Why

pimdir spec edit 4 (plan, 2026-10-30), the merge half beside the-agreed-body-stays-pinned:

- an optional `if_match` on the queued `update` payload, the owner parking the row when the item's body is no longer the one the producer edited. Today a producer's `update` written over a body a sync changed since it read it lands silently over the server edit (frontends check `--if-match` against the overlay before staging, unfog backend.rs:763-788, but the window between staging and drain is unguarded);
- `SetFlags` on the connector seam becomes add and remove sets (`PimdirChangeKind::SetFlags`), so a server flag change between enumeration and push is not overwritten: today it replaces the set absolutely, silent last-writer-wins on flags (SYNC.md:126; pimdir algorithm-audit-2026-09-04);
- the spec states the push window left unguarded; the seam docs say the same.

## What

- `codec`: `PimdirAction::Update` gains `if_match: Option<PimdirHash>`, encoded only when present (`v` unchanged as the spec decides); the drain parks an `update` whose item's body is not `if_match`, the error naming both hashes.
- `change`: `PimdirChangeKind::SetFlags { handle, add, remove }` in place of the absolute set; the engine derives both from its three-way merge; connectors apply them as deltas (IMAP `+FLAGS`/`-FLAGS`, JMAP patch). **Breaking** for every connector.
- The queued `set-flags` action stays absolute unless the spec changes it.
- The vectors pimdir adds for both.
