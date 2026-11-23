---
cairn: change
id: an-ambiguous-intent-awaits-confirmation
status: landed
created: 2026-10-18
---

# An intent whose outcome is unknown awaits confirmation

## Why

An intent is at-least-once and deduplicating it is the performer's (pimdir STORAGE §15.5): it is acknowledged once performed, and a failure leaves it pending or parks it. One outcome fits neither: a mail submission whose connection dropped after the message data was sent and before the server answered. The message may have gone. Left pending, the next drain sends it again; parked, it reads as failed though it may have been delivered.

What the queue already has is kept: append order serialises the actions on one item and lets an offline create precede its edit, and every pushed change carries an idempotency key (SYNC §4). Borrowed from an external engine's outbox: an ambiguous outcome is a state of its own, distinct from failed, and is never retried blindly.

## What

- A third outcome for a performed intent beside acknowledged and parked: **unconfirmed**, the row kept with a reason saying the effect may have happened.
- A drain skips an unconfirmed row: no retry, no attempt counted.
- It leaves the state by a decision: the performer confirms it (the sent copy found in the Sent collection, or the provider's own record), which acknowledges it; or the user retries it, which makes it pending again; or withdraws it.
- A producer following the row (`load_action`) reads the state and shows it.

## Open questions

- Whether the state is a column of `queue` or a reserved `error` form, so an older owner reads it as parked rather than pending.
- Which intents can end unconfirmed: `submit` surely; `collection-create` is idempotent by name (Annex B.2) and needs none.
- Whether a performer may confirm automatically by finding the message, and by what match (`Message-ID` in the account's sent collection).

## Landed in substance (2026-11-19)

The owner ruled on 2026-11-19 (pimdir the-lines-before-v1) to close this change as landed: the state arrived through pimdir draft-10 and draft-11 (audit-2026-11-12 O7 and F16), as a parked row under the reserved `error` prefix `unconfirmed: ` rather than a column, which answers the first open question (an older owner reads it as parked); `submit` is the intent Annex B.2 names, and a performer confirms by `acknowledge_action`. Evidence in cairn/log/2026-11-19-an-ambiguous-intent-awaits-confirmation.md.
