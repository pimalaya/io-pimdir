---
cairn: change
id: not-mine-is-not-broken
status: landed
created: 2026-08-28
---

# An action this source cannot place is skipped, not parked

## Why

The queue is the whole store's and records no source. An owner drains it as one
of its sources, and staging an existing item's action resolves that item's
binding **for the draining source**: no binding, no handle, nothing to mutate.

That was treated as a park, which is terminal. A parked row carries an `error`,
every later drain filters it out, and nothing in the crate ever clears one: the
only exit is `queue cancel`, which throws the action away. So the first source
to reach an action it could not place destroyed it for the source that could.

It is not hypothetical. A neverest account syncing mail, contacts and calendar
drains once per source in name order, so `caldav` reached every action himalaya
queued against `imap/INBOX` before `imap` did, and parked all of them. The item
was there, with a live `imap` binding, and the flag change was lost anyway.

The spec already draws the line this crossed: an action the owner cannot apply
**at all** parks, one it cannot apply **here** is "skipped and left pending, so
another owner can perform it". A missing binding for the draining source is the
second case, and it was being answered as the first.

## What

- `stage_action` returns a `PimdirRefusal` rather than a bare park reason:
  `Park(String)` for what will never stage, `Skip` for what this source cannot
  place.
- A missing binding is `Skip`. The drain's transaction rolls back as before, so
  the row is left exactly as found, still pending, its `attempts` untouched, and
  counts as `skipped` in the drain report.
- Every other refusal is unchanged and still parks: an undecodable payload, an
  item that is gone, a link id already taken, a mutation the engine refuses.

## Not in scope

**Unparking.** Rows parked by the old behaviour stay parked, `queue cancel`
being the only exit. Nothing here clears an `error`, and a verb that did would
be its own change.

**Who drains what.** That a consumer drains collections belonging to another of
its sources is the consumer's to fix, and neverest does. This makes the crate
safe when it happens.
