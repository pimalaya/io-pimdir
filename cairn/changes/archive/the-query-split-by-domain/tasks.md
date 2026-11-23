---
cairn: tasks
change: the-query-split-by-domain
---

# Tasks

## Split (independent of the SEARCH delta)

- [x] `search::query` and `search::coverage` out of the `search` gate; their tests run under `--no-default-features`.
- [x] Features `mail`, `contact`, `calendar`; `search` (read the index: `client`, `open_query`, compiler, search, count, item, tag); `indexer` (the three domains and the I/O-free refresh, no `client`; with `search`, `open`, `refresh`, `erase`). Every combination passes clippy, a CI step checks each.
- [x] client/search.rs split into its read and refresh halves (summary reads in client/search/summary.rs); `FLOATING_ZONE` into `coverage`; `EXTRACTION` and `EXTRACTOR` stay in the core of `extract`, split into mail.rs, contact.rs, calendar.rs.
- [x] Check: `cargo tree --no-default-features --features search,mail` holds no ical-rs nor vcard-rs; `--features indexer` holds no rusqlite.
- [x] blake3 and encoding_rs checked, both kept outside `client`: the Android app names bodies with `hash::hash` without `client`, and `summary::mail` decodes charsets (Annex A.0) for every profile.
- [x] `search::mail`: `PimdirMail::read` and `read_meta`, `match_mails`, `mail_terms`; tests/search_mails.rs runs the vectors over the fixture messages.
- [x] `PimdirQueryPredicate::name()`.

## SEARCH additions (after the pimdir delta is agreed)

- [x] `mid:` in place of `header:` (owner, 2026-11-21: notmuch's field), `unknown-field` kept.
- [x] Instants and `min`, `h`, `mo`; the spans moved to `PimdirQueryDateRange::bounds`, public, for the matchers and the server translations.
- [x] `PimdirSearchSort` as SEARCH §8.2's five named orders (owner: notmuch-style, no direction axis).
- [x] The new vectors pass (spec/ re-vendored from the pimdir working tree; spec/REVISION moves once pimdir commits).

## Release

- [x] CHANGELOG, 0.20.0, consumers' feature lines listed for Cardamum, Calendula, Himalaya and the Android app (2026-11-23, by cairn/changes/what-the-consumers-need task 5.2).
