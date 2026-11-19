---
cairn: tasks
change: a-push-reads-back-the-server-bytes
---

- [x] Rule when (2026-11-19: after v1, an additive optional body on the outcome)
- [ ] Decide the read path (a fetch by handle, a delta, or the body carried by the outcome) and the event
- [ ] pimdir: SYNC §4 (an outcome MAY carry the stored body), §5 (push discipline: the base is the server's body), a sync vector where the server drops a property
- [ ] src/sync.rs: read back accepted content pushes of a mutable kind before the last write; land them as pulls; report a changed body; never repush a property the server dropped
- [ ] src/remote.rs: the outcome's optional body
- [ ] Test: a server dropping a property leaves the store holding its body, an event, and no push on the next run
- [ ] Spec: sync.md, seam.md; fold, log, CHANGELOG
- [ ] neverest: DAV connectors ask for the stored representation where the server offers it
