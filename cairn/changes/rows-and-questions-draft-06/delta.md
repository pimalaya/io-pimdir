---
cairn: delta
change: rows-and-questions-draft-06
---

Folds into `spec/summaries.md`, `spec/store.md` and `spec/search.md`.

## ADDED Requirements

### Requirement: A task carries its capture and its contacts
The task derivation SHALL carry `created`, CREATED as an instant on Annex A.3's rules, else DTSTAMP, else none, and every CONTACT under the `contact` role in document order: its address the `mailto:` ALTREP's, else its text, canonical; its name the text decoded, none when the address was read from a text that is an addr-spec; an empty text with no such ALTREP filing no row.

#### Scenario: Captured and waiting on an address
- GIVEN a VTODO with CREATED, a later DTSTAMP and `CONTACT;ALTREP="mailto:Landlord@Example.org":Mr Landlord`
- WHEN it is derived
- THEN `created` is the CREATED instant, and one `contact` row reads `landlord@example.org`, named `Mr Landlord`

### Requirement: A store derived under revision 1 is re-derived on open
An owner SHALL re-derive, from every held task body, `task_summary.created` and the task's addresses of a store whose `store_meta.derivation` is 1, a page at a time, recording revision 2 in the transaction writing the last page, the next open resuming an interrupted one.

#### Scenario: A draft-05 store
- GIVEN a store holding a task with CREATED and a CONTACT, derived under revision 1
- WHEN the owner opens it
- THEN its `created` and `contact` row are filled and `store_meta.derivation` is 2

### Requirement: A card's properties and categories are questions for the index
The extraction SHALL record whether a card holds a TEL, an ADR, a BDAY and a PHOTO and its folded CATEGORIES, every other kind recording none, and the query SHALL answer `has:phone`, `has:address`, `has:birthday`, `has:photo` and `category:` on a card from them.

#### Scenario: A card without a phone
- GIVEN a card with an ADR, a BDAY, a PHOTO and no TEL
- WHEN the index is asked `has:phone` and `has:address`
- THEN the first misses it and the second finds it

### Requirement: An agenda reads a range's occurrences in order
`PimdirIndex::occurrences_between` SHALL answer every occurrence overlapping a half-open range, one row each with its instants, ordered by start, then seq, then collection.

#### Scenario: A weekly series
- GIVEN a weekly series with three occurrences in a range
- WHEN the range is asked
- THEN three rows come back in time order

## MODIFIED Requirements

### Requirement: The store records the derivation revision
The store SHALL record in `store_meta.derivation` the Annex A revision its summaries, addresses and sort keys were derived under (STORAGE §6): `init_store_meta` writes this crate's (`schema::DERIVATION`, revision 2), a reconciliation gives an existing store the column's default 1, and an owner at a later revision re-derives what changed then records it with `set_derivation`, which never lowers it. Raising `DERIVATION` owes that re-derivation.

## REMOVED Requirements
