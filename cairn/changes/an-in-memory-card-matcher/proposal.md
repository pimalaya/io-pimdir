---
cairn: change
id: an-in-memory-card-matcher
status: landed
created: 2026-11-18
---

# An in-memory card matcher

## Why

cardamum's card-search change (its cairn/changes/card-search) answers `card search <query>` on CardDAV, JMAP, Graph and People by listing the address books in scope and matching each card against the SEARCH §8 query in memory, so every backend answers the hit set an indexed pimdir store holding the same cards answers. It waits on a matcher here, tested on the SEARCH §11 contact cases, building under `search` without `client` or SQLite.

## What

- src/search/card.rs, under `search`: `PimdirCard::read(body)` reads a card as the indexer does (SEARCH §6, the summary for one no parser accepts) with its addresses as the store keeps them (STORAGE Annex A.6); `PimdirQuery::match_cards(&[PimdirCard])` answers, per card, whether it is a hit, as an index holding those cards alone does; `PimdirQuery::store_terms()` names the terms a store alone answers (`tag:`, `flag:`, `is:` but `is:encrypted`, `account:`, `collection:`, `folder:`, `changed:`, `thread:`, `id:`, `linked:`), which `match_cards` refuses (`PimdirCardMatchError`, the term's span).
- Text is folded (SEARCH §5) and split as `unicode61` splits it by default, `L*`, `N*` and `Co` forming tokens, read from Rust's character properties: `is_alphanumeric` less the combining marks and the circled and squared Latin letters, plus the private-use planes. A term matches its tokens consecutively in one column, its last as a prefix when it ends with the wildcard (SEARCH §8.5).
- `person:` reads every card given: a card sharing an address with a card its name matches is a hit. A `date:` reaches a card only where `kind:contact` stands in the query (SEARCH §8.1); `has:body` holds, a card being read from its body.
- No new dependency: `unicode-normalization` already gives the combining marks.
