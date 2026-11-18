---
cairn: delta
change: an-in-memory-card-matcher
---

## ADDED Requirements

### Requirement: Cards are matched in memory as the index matches them
Under the `search` feature, without `client`, `PimdirQuery::match_cards(&[PimdirCard])` SHALL answer, per card read by `PimdirCard::read`, whether it is a hit of the query as an index holding those cards alone answers (SEARCH §5, §6, §8, §8.5): text folded and split as `unicode61` splits it, matched consecutively in one column with a trailing prefix, the role fields and `with:` over the card's addresses, `person:` over every card given, `has:` from the card's marks, `category:` folded, `kind:contact` alone holding, a date reaching a card beside `kind:contact`. A term a store alone answers (`PimdirQuery::store_terms`) SHALL refuse the match, naming its span (`PimdirCardMatchError`).

#### Scenario: The contact cases
- tests/search_cards.rs `every_contact_case_answers_the_index_hits_among_the_cards`: every case of vectors/search/queries.json that parses and reads no store answers, over the fixture store's cards, the hits it expects among them (more than 100 cases); `has:phone` read from the `ADR` mark fails it
- src/search/card.rs: text, addresses, `person:`, marks and the refusal of a store term

## MODIFIED Requirements

## REMOVED Requirements
