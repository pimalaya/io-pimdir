---
cairn: delta
change: capabilities
---

# Delta

## ADDED Requirements

### Requirement: A producer's enqueue is gated on the declared capabilities

`enqueue` refuses with `Unsupported`, naming capability, source and detail, an action a declared source concerned does not support (pimdir STORAGE §15.6, Annex B.1), reading a calendar write's resources from the blob store; an undeclared source gates nothing. An intent with no resolvable performer is `NoPerformer` or `Ambiguous`.

#### Scenario: A scheduled event on a source that notifies nobody

- **GIVEN** a calendar source declaring `calendar.scheduling` `none`
- **WHEN** a producer enqueues an event naming an attendee with no `SCHEDULE-AGENT`
- **THEN** it is refused with `calendar.scheduling`, and the same event marked `SCHEDULE-AGENT=NONE` is queued

### Requirement: The drain parks what a declared source does not support

### Requirement: An intent's candidates are read at its anchor

The candidates are the account's sources whose row at the anchor collection, its own or else the source-wide one, has some support: a provider's reply declared on its calendars and an iMIP reply declared source-wide are both candidates on those calendars, the second alone elsewhere.
