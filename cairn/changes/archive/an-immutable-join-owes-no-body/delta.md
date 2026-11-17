---
cairn: delta
change: an-immutable-join-owes-no-body
---

## ADDED Requirements

### Requirement: An immutable binding owes no body
`PimdirHub::immutable`, set by the store's `read_hub` for a `message/rfc822` collection, SHALL make the projection compare no body against a binding's base (SYNC §3 rule 4: the body axis is a mutable kind's), and the absorb SHALL let a binding of such a collection adopt the shared body as its base whatever its own base named, a listing carrying none included (SYNC §3, §9). A message another source holds, joined by a member a page before the last names, or listed by both sources before either hydrated it, so projects `Clean` and derives no `Update`, which an IMAP connector rejects every run while no flag change for the message ever pushes; a binding a store already wrote with a body-less base is read so without a rebuild.

#### Scenario: A message another source holds
- GIVEN a mail collection one source holds a message in, hydrated, and a second source listing its own copy on the first page of a round of two, or both sources listing it at `Meta` before each hydrates
- WHEN the second source syncs several times, then the first's server gains a flag
- THEN the second source pushes no `Update`, its placement is `Clean`, and the flag reaches its server (tests/engine/join_immutable.rs)

### Requirement: A source binding an item for the first time joins its flags as a union
The absorb SHALL take the union of the item's flags and those of an upsert binding the item for a source that bound it in no row before (`PimdirFlags::merge` against no base, SYNC §5), whichever page of a round named the member, so a first pairing never removes a flag from the server that had it; a source already binding the item sets the flags as before.

### Requirement: An offered create is landed with the union of its flags
A member landing a `Created` placement the store offers this source (an item it does not bind yet, `PimdirLoaded::offered`) SHALL be landed with the union of the placement's flags and the member's, neither side holding a base to have removed one against (SYNC §5); a pending create of this source keeps its staged flags as before.

#### Scenario: A first pairing ends alike whatever the page
- GIVEN two mail sources holding one message, one with `\Seen` and the other without, either way round
- WHEN the second source lists it on the first page of a round of two, or on the last
- THEN both servers end with `\Seen` (tests/engine/join_immutable.rs `a_first_pairing_ends_alike_whatever_the_page`)

## MODIFIED Requirements

## REMOVED Requirements
