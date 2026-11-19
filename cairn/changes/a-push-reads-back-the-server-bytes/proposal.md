---
cairn: change
id: a-push-reads-back-the-server-bytes
status: active
created: 2026-10-18
---

# A content push reads back what the server stored

## Why

An accepted `Add` or `Update` of a mutable kind rebases the placement on the body the engine pushed, at the revision the connector reports (pimdir SYNC §4, §5). The store then holds its own optimistic bytes as the base. A server that stores something else (a CardDAV server re-serialising a card, a calendar server dropping a property it does not support, normalising a `TZID`) hands back a revision the base already carries, so no enumeration ever reads the difference: the store believes the server holds a property it dropped, and the next merge diffs against bytes nobody holds.

Borrowed from an external engine: after a write, re-read through the same path a sync reads by, so the store holds the server's bytes and a silently dropped property is caught rather than masked, and the cursor advances so the change is not delivered again.

## What

- After a run's accepted content pushes of a mutable kind, the engine reads those members back by handle, as an upgrade fetches (`Full`), before the run's last write, and lands each as a pull at the reported revision: the server's body becomes the base and, when the item had not moved since, the shared body.
- A body read back that differs from the one pushed is reported (an event naming the item), so a consumer can tell "saved" from "saved, but the server changed it".
- A body read back that differs from the pushed one by a property the push carried is not pushed again: the server refused it, and pushing it each run would loop.
- Immutable kinds are untouched: a message's body is its hash and its identity.

## Open questions

- The cost: one fetch per pushed body. A connector whose write answers the stored body (a DAV `PUT` with `Prefer: return=representation`) could hand it with the outcome instead, `Accepted { revision, object }`.
- Whether the read-back goes through a delta from the checkpoint (which also advances it) or a fetch by handle, for a connector that lists deltas.
- How a consumer is told, and whether `Manual` should keep the pushed body as a divergence instead of adopting the server's.

## Ruled (2026-11-19)

The owner ruled it after v1, additive: a push outcome MAY carry the stored body (`Accepted { revision, object }` or a read-back the engine asks for), an optional field SYNC §4 gains in a minor after sync-v1.0.0; SYNC §5 does not make the server's body the base a MUST before the tags, so no frozen behaviour or vector moves. Accepted and deferred: nothing is implemented before sync-v1.0.0. The local body still holds what a server dropped meanwhile, so nothing is lost.
