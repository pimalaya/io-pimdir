---
cairn: log
change: the-query-split-by-domain
landed: 2026-11-21
---

# The query follows search-draft-16

The second half of the-query-split-by-domain, uncommitted: `mid:` (`PimdirQueryPredicate::Mid`, a store seek on `mail_summary.message_id`), instants and `min`, `h`, `mo` (`PimdirQueryDate::Instant`, the units), the spans moved from the compiler to `PimdirQueryDateRange::bounds` so the matchers and a client's server translation read them without `client`, `PimdirSearchSort` as SEARCH §8.2's five orders with `as_str`/`parse`/`ALL` (the CLI's `--sort` takes the names), `PimdirQueryPredicate::name()`, and `search::mail` (`PimdirMail::read`, `read_meta`, `from_summary`, `match_mails`, `mail_terms`), the token matching shared with the card matcher in `fold`. `FORMAT_DRAFT` 16. blake3 and encoding_rs stay outside `client`: the Android app names bodies with `hash::hash` without it, and the mail summaries decode charsets in every profile.

Checks: tests/search.rs `every_search_vector_answers` over the new vectors (pimdir queries.json `v` 5, spec/ re-vendored from the uncommitted pimdir tree, so tests/spec_drift.rs fails until pimdir commits and spec/REVISION moves); tests/search_mails.rs over the fixture messages, failing with the flag check broken; compile.rs tests for a skipped and a repeated local time and a month back; query.rs for the predicate names; mail.rs for a header block and a refused term. Capabilities moved: spec/search.md (dates, the mail matcher, predicate names, the five orders).

Left: the release as 0.20.0.
