---
cairn: delta
change: show-the-bindings
---

# Delta

## ADDED Requirements

### Requirement: A binding is readable, and `item show` prints it
The library SHALL expose every source's binding of one item, keyed by source:
the handle that source addresses it by, the base the last sync agreed on
(flags, body and revision, and whether a base exists at all), and the two
exception markers, `conflicted` with the revision observed when it was recorded,
and the other handles the source holds the identity under.

`item show` SHALL print one block per binding under each placement it names, in
text and under `--json`. The two exception lines SHALL be printed only when they
apply, so a frozen or diverged binding stands out from the ordinary ones beside
it rather than being one more line that is usually empty.

This is what makes a frozen identity actionable. `check` reports one by
collection, link id, source and count, and the next question is always which
resources the source holds it under; without this the only answers were the
server and the database file.

`item list` SHALL NOT carry bindings. Its rows are a page served by one query,
and a per-row binding lookup would make a listing cost what a listing must not.
The verb that names one item is the one that can afford to say everything about
it.

#### Scenario: A frozen identity names the handles that froze it
- GIVEN a source holding one link id under two handles
- WHEN `item show` names that item
- THEN its binding prints the handle it is bound to and the other handle beside it

#### Scenario: Each source reports its own view
- GIVEN two sources holding one item, one of them diverged from its own remote
- WHEN `item show` names that item
- THEN each source prints its own handle and base, and only the diverged one prints a conflict
