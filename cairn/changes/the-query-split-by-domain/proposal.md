---
cairn: change
id: the-query-split-by-domain
status: draft
created: 2026-11-21
---

# The query split by domain

## Why

The `search` feature gates everything SEARCH needs at once, the grammar included, and pulls ical-rs, vcard-rs, mime-rs and unicode-normalization. Cardamum turns it on for every backend feature, so a CardDAV-only build compiles an iCalendar parser to read a query. Himalaya, moving its `envelope search` onto the SEARCH language (pimdir cairn/changes/the-query-serves-every-client), would compile all of it into an IMAP-only build. The grammar itself depends on nothing but `alloc`: the weight comes from extraction, occurrences, the matchers and the refresh.

## What

- **The grammar is always there.** `search::query` (parser, AST, refusals) and `search::coverage` compile with no feature, no dependency added. Every field parses in every build: the same query string is accepted or refused the same way whatever a client compiled, so the vectors hold for all of them, and a client refuses a field it does not serve as its own capability (`capability-missing` in the CLIs), never as `unknown-field`.
- **Domain features** gate the heavy halves, each a module gated once at its parent:
  - `mail` (mime-rs, unicode-normalization): mail extraction (`search::extract::mail`) and `search::mail`, a new in-memory mail matcher over a mail summary, the twin of `search::card`, for clients answering without an index (Himalaya's Maildir and mbox, its pimdir fallback).
  - `contact` (vcard-rs, unicode-normalization): card extraction and `search::card`.
  - `calendar` (ical-rs, unicode-normalization): event, task and journal extraction.
  - The text layer (`fold`, `chunk`, the core of `extract`: `PimdirExtract`, `EXTRACTION`, `EXTRACTOR`) compiles with `search` or any domain.
  - `search` reads the index: `client` plus unicode-normalization, `PimdirIndex::open_query`, the compiler, `search`, `count`, `item`, `tag`. A reader never refreshes the index, but `item` (show) and the snippets re-extract the bodies they print, so a reader takes the domains it shows: a body of a domain left out is read from its summary, its snippet empty. Himalaya's `pimdir` takes `search` and `mail` and pulls no ical-rs nor vcard-rs; Calendula takes `search` and `calendar`, Cardamum `search` and `contact`.
  - `indexer` is the I/O-free refresh: the three domains, `search::refresh`, `occurrence` and the threading, no `client`, since the Android app runs the coroutine over its own SQLite. With `search` too, `PimdirIndex::open`, `refresh` and `erase` (client/search/refresh.rs, the summary reads moved out of it to client/search/summary.rs for `item`). `FLOATING_ZONE` moves from `occurrence` to `coverage`, which the read side states the zone through.
- **The SEARCH additions:** the `header:` predicate, instants and the `min`, `h`, `mo` amounts in `PimdirQueryDate`, the `unserved-field` refusal, and `PimdirSearchSort` becoming a key (`Date`, `Title`, `Sender`, `Relevance`) with a direction.
- **Stable predicate names.** `PimdirQueryPredicate::name()` returns the name a capability table keys on (`from`, `with`, `header.message-id`, `date`, `date.instant`, `is.unread`, `has.attachment`, ...), so the three CLIs declare per backend what they serve in one vocabulary.
- **Small dependencies off `client`.** blake3 and encoding_rs are checked: whatever only the store needs moves behind `client`, so a grammar-only build adds the least.

## Not done

Translating a query to a server (IMAP `SEARCH`, a Gmail `q`, KQL, JMAP filters) stays in each client: it is the client's protocol knowledge. A grammar gated by domain was considered (the owner's first idea, 2026-11-21) and is not proposed: the grammar costs no dependency, and gating fields would make one query parse differently per build, split the vectors and push a `cfg` into every consumer's `match`. If the owner prefers it anyway, the variants gate by domain and a compiled-out field is refused with `unserved-field`, never `unknown-field`.

## Consumers

- Neverest: `search` and `indexer`, the one CLI building and refreshing the index (`sync --index-only`).
- Himalaya: `mail` everywhere, `client` and `search` on `pimdir`; himalaya cairn/changes/envelope-search-speaks-pimdir.
- Cardamum: `io-pimdir/search` becomes `io-pimdir/contact` on its non-pimdir backends, `client`, `search` and `contact` on `pimdir`; no code change expected.
- Calendula: `client`, `search` and `calendar` on `pimdir`; the orders follow the new `PimdirSearchSort`.
- Himalaya, Calendula and Cardamum refresh an index in their tests only (fixtures): `indexer` goes in their `[dev-dependencies]`, which resolver 2 keeps out of the shipped build.
- io-pimdir's own `pimdir` binary (`cli`): `search` and `indexer`. The Android app: `indexer` alone, in place of `search` today.

## Order

The grammar additions wait for the SEARCH delta's review; the feature split does not, and lands first. Released as io-pimdir 0.20.0 with the SEARCH draft that carries the additions.
