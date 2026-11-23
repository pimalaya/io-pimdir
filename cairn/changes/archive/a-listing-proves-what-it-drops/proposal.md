---
cairn: change
id: a-listing-proves-what-it-drops
status: landed
created: 2026-10-30
---

# A listing proves what it drops

## Why

pimdir spec edit 3 (plan, 2026-10-30) makes completeness a rule rather than a convention: only absences from a closed complete round in scope propagate (SYNC.md:147 resolved against :270); `complete`, `last` and the cursor are MUSTs; an engine refuses a non-last page without a cursor (a-page-without-a-cursor-is-refused, landed 2026-10-30); a round dropping more than 50% of a scope of at least 20 bound members parks and reports; a server collection is never deleted from an absence unless enabled, and then only when empty or fully reconciled in the run; a two-source vector.

What io-pimdir still lacks:

- no mass-absence guard: a round finding most of a large scope absent drops it all, and the hub pushes `Remove` to every other source;
- a paged delta (Gmail history, JMAP `hasMoreChanges`) handing its final checkpoint early: the delta page's `last` and cursor are ignored and its checkpoint lands (src/sync.rs:557, :711-717, :777-780; review-io-pimdir.md S12, code reading);
- `delete_collection` (src/client.rs:608) removes a collection with its queued rows; the rule for a collection a server stops listing is the connector's today (neverest `collection.delete`).

## What

- Mass-absence guard: a round's last page that would drop more than half of a scope of at least 20 bound members drops nothing, keeps the round's stamps, and reports it (a `PimdirSyncReport` field and a `PimdirSyncEvent`), as the spec states it; a later round within the bound closes normally. The thresholds as pimdir fixes them.
- A delta page with `last` false is refused like a round page, or followed through its cursor, as the spec decides; its checkpoint lands only with its last page.
- A collection's deletion from an absence: what the library owes (the run's reconciliation state a connector reads before deleting, and `delete_collection` refusing or parking queued rows, spec edit 10).
- The pimdir vectors for the guard, the delta page and the two-source case, run by tests/vectors_sync.rs.
