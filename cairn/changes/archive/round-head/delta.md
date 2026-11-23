---
cairn: delta
change: round-head
---

## ADDED Requirements

### Requirement: A resumed round lists its head first
A run resuming an open round SHALL first ask for the round's head when the connector lists heads (`PimdirRemote::heads`). The head is a delta head from the round's checkpoint, or for a band round from the source's checkpoint, when the connector lists delta heads; else a band head from `round_started_at`, when it lists band heads; else none. It is checked against the run's scope and merged as a page before the last.

A delta head stamps its members and records its checkpoint as the round's, with the cursor kept. A band head stamps nothing and records nothing. Neither derives an `Add` or infers an absence. `HeadRejected` restarts the round over the run's scope, and `NoHead` resumes the round (pimdir SYNC §5, vectors/sync/62 to 69).

#### Scenario: An arrival during an open round
- GIVEN a round left open on checkpoint h1000 and cursor t1
- WHEN the next run's head lists a new member with checkpoint h1001, and the last page lands
- THEN the member is named and stamped, it survives the close, and the round closes on h1001

## MODIFIED Requirements

### Requirement: A listed member carries its meta
Appended: a head is answered as a page, `PimdirRemoteSnapshot::delta` for a delta head and `band_head` for a band head, or as `PimdirEnumerated::HeadRejected` or `NoHead`.
