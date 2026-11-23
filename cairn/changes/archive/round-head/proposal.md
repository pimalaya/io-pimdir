---
cairn: change
id: round-head
status: landed
created: 2026-10-13
---

# A round lists its head first

## Why

pimdir `round-head` (pimdir ab30f24, SYNC §4, §5; vectors/sync/62 to 69). A first round over a large mailbox stays open for many runs: a Gmail Inbox of 20,000 messages, paced below its quota, fills in about a quarter of an hour. Every run resumes the round, the remaining pages list older mail, and no delta runs before it closes, so a message arriving meanwhile shows only once the round has closed. MOA saw an invoice arriving at 10:03 shown at 10:18.

## What

- `remote`: `PimdirHeads { delta, band }` (`NONE`, `DELTA`, `BAND`, `BOTH`), stated by `PimdirRemote::heads` (default `NONE`) as `scope_bound` is; `PimdirHead::Delta(checkpoint)` and `PimdirHead::Band { since }`; `PimdirListing::Head`; `PimdirEnumerated::HeadRejected` and `NoHead`; `PimdirEnumerate::head`; `PimdirRemoteSnapshot::band_head`.
- `sync`: `PimdirSync::heads`, which `PimdirSourceStore::sync` sets from the remote. A run resuming an open round lists its head before the round's next page:
  - from the round's checkpoint, or for a band round from the source's, when the connector lists delta heads;
  - else from `round_started_at`, when it lists band heads;
  - else none.
  
  The head is checked against the run's scope and merged as a page before the last: its candidates are its members and vanished handles, it derives no `Add`, and it infers no absence. A delta head stamps its members and records its checkpoint as the round's. A band head does neither. The round's cursor stays. `HeadRejected` restarts the round over the run's scope, never a band. `NoHead` resumes the round.
- `change`: `PimdirWriteOp::SetRoundCursor::cursor` becomes `Option<PimdirCursor>`, `None` keeping the cursor a page landed; the vendored `set_round_cursor` coalesces it.
