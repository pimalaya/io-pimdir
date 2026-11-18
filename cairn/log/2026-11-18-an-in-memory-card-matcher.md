---
cairn: log
change: an-in-memory-card-matcher
landed: 2026-11-18
---

# An in-memory card matcher

cardamum's card search (its cairn/changes/card-search) matches the cards a server lists against a SEARCH §8 query, and waited on a matcher answering as the index does.

## What landed

- src/search/card.rs, under `search` and without `client`: `PimdirCard::read`, `PimdirQuery::match_cards`, `PimdirQuery::store_terms` and `PimdirCardMatchError`.
- Tests: src/search/card.rs (text, addresses, `person:`, marks, the refusal), and tests/search_cards.rs running every SEARCH §11 case that parses and reads no store over the fixture's three cards, failing when a mark is read wrong.
- Builds with `--no-default-features --features search`, no SQLite.
- Outside the hit-set guarantee: a character Rust's Unicode tables classify otherwise than SQLite's, and a letter SQLite lowercases otherwise than Rust.

## Capabilities moved

search ("Cards are matched in memory as the index matches them", added).

In the working tree on io-pimdir 7370ac4, uncommitted, no version bumped; CHANGELOG.md [Unreleased] names it.
