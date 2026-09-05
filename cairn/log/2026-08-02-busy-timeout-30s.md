---
cairn: log
change: busy-timeout-30s
date: 2026-08-02
---

# Raise the write busy timeout to 30s for same-source worker fan-out

neverest's one-source sync now fans its spine across several same-source store
handles (one per connection/worker) to overlap network while the writes serialise
on the store's single-writer lock. A first sync's per-mailbox meta insert can be a
large write that holds the lock for a moment; a burst of them contending could
exceed the previous 5s busy timeout and trip `PimdirError::Busy`. Raised
`busy_timeout` 5s → 30s so contending large writes wait it out rather than fail.
No other behaviour changes; correctness of the serialised writes is unchanged
(`BEGIN IMMEDIATE`).

Spec updated: `store` ("A write batch is one transaction": the busy timeout is
generous (30s) to accommodate same-source worker fan-out).
