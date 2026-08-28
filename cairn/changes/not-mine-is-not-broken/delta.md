---
cairn: change
change: not-mine-is-not-broken
---

# Delta

## MODIFIED Requirements

### Requirement: Producers append, only the owner pops
The store SHALL support the pimdir action queue: any process may act as a
producer whose sole write is the single enqueue transaction (ensure_collection,
at most one object upsert pinning a pre-written blob, one queue insert). Only the
owner SHALL read-and-remove queue rows: each pending action is applied to items
and bindings and its row deleted in the same transaction, so application is
exactly-once and never partially visible. Failing actions accumulate `attempts`;
permanently failing actions are parked with `error` set, skipped without blocking
later actions, queryable, and never silently deleted.

An action the owner cannot apply at all (a kind it does not recognise, or one it
recognises but lacks the capability to perform) is **skipped and left pending**,
never parked, so another owner can perform it. An action the owner cannot apply
**as the source it is draining** SHALL be treated the same way: an existing
item's action resolves that item's binding for the draining source, and a source
holding no binding for it has nothing to mutate, which says nothing about
whether another source can. Parking it would be terminal, no drain retrying a
parked row and no verb clearing one, so the first source to reach an action it
cannot place would destroy it for the source that could. Such a row SHALL be
left pending with its `attempts` untouched and counted as skipped.

#### Scenario: A source that holds no binding leaves the row alone
- GIVEN a queued action against an item bound to one source
- WHEN another source drains that collection first
- THEN the action is skipped, the row stays pending and unmarked, and the
  binding's own source applies it on its turn
