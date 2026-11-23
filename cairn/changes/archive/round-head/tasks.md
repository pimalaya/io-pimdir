---
cairn: tasks
change: round-head
---

- [x] `remote.rs`: heads, head listing and answers, documented for connectors (the band-head relisting on a round's last page, fresh pages after a delta head)
- [x] `sync.rs`: the head planned when resuming, merged as a page before the last, stamped and recorded by kind, rejection and absence handled; unit tests
- [x] `change.rs`, `client/write.rs`: `SetRoundCursor` with no cursor; spec/ re-vendored (`set_round_cursor`)
- [x] `client/run.rs`: `sync` passes the remote's heads
- [x] tests/vectors_sync.rs: `heads`, `head`, `round_started_at`; vectors 62 to 69 pass
- [x] tests/scoped_sync.rs: an arrival during a round through the std runner
- [x] Fold, log, CHANGELOG
