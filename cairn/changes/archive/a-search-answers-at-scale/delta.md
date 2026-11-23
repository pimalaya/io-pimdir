---
cairn: delta
change: a-search-answers-at-scale
---

## ADDED Requirements

### Requirement: A search answers at the scale of a mailbox
A refresh SHALL read one item's summary and addresses by seek, never by a scan of its collection, and a compiled term's membership SHALL never be `NULL`, so a negated term is one probe per hit. Over 50,000 summary-only messages in one collection (tests/search_scale.rs, release build) the index builds in about 5 s, and `is:unread` answers a page of 50 in about 120 ms and a count in about 25 ms.

#### Scenario: 50,000 messages
- GIVEN 50,000 summary-only messages in one collection, a third read
- WHEN the index is built, then `is:unread`, `not is:unread`, `is:flagged`, `date:`, `from:`, `subject:` are asked a page and a count
- THEN the build takes about 5 s (454 s before) and every page and count under 0.2 s (`is:unread` 25 s before)
