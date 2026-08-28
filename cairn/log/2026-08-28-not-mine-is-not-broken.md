---
cairn: log
change: not-mine-is-not-broken
landed: 2026-08-28
---

# A source that cannot place an action no longer destroys it

Staging an existing item's queued action resolves that item's binding for the
**draining** source. No binding meant no handle, and that was answered with a
park: `error` set, filtered out by every later drain, and no verb in the crate
clears one. So the first source to reach an action it could not place made the
action unappliable for the source that could.

It cost a real flag change. A neverest account syncing mail, contacts and
calendar drains once per source in name order, so `caldav` reached what himalaya
had queued against `imap/INBOX` first and parked it, while the item sat there
with a live `imap` binding. Every mail action that account queued died the same
way.

**Refusal split** (`client.rs`): `stage_action` returns a `PimdirRefusal` in
place of a bare park reason. `Park(String)` is what will never stage, an
undecodable payload, an item that is gone, a link id already taken, a mutation
the engine refuses. `Skip` is a missing binding for the draining source: the
drain's transaction rolls back as it already did, so the row is left exactly as
found, still pending, `attempts` untouched, and counted as `skipped` in the
report.

That is what the spec asked for all along: an action the owner cannot apply at
all parks, one it cannot apply *here* is skipped and left pending. A missing
binding is the second case and was being answered as the first. The requirement
now says so in as many words, the two readings having been distinguishable only
by intent.

Covered by `an_action_the_draining_source_cannot_place_is_skipped_not_parked`:
a source with no binding drains first and reports `(applied, skipped, parked) =
(0, 1, 0)` with an empty parked list, then the owning source drains and reports
`(1, 0, 0)` with the flags actually cleared. On the old behaviour the second
drain sees nothing, the row having been parked.

Rows parked by the old behaviour stay parked: nothing here clears an `error`,
and `queue cancel` remains the only exit.

Verified: 15 unit and 57 integration tests green, fmt and clippy clean.

Spec updated: `store` (MODIFIED: "Producers append, only the owner pops", the
"cannot apply here" case extended to a source that holds no binding).
