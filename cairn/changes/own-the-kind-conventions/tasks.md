---
cairn: tasks
change: own-the-kind-conventions
---

# Tasks

- [x] `conventions` module in the I/O-free core, one `derive(body) -> (link_id, meta, sort_key)` per kind (`message/rfc822`, `text/vcard`, `text/calendar`), with the summary types serialised per Annex A.
- [x] Fix the link-id fallbacks here: `alt:{subject}|{date}|{from}` for a message with no `Message-ID`, `hash:{fnv1a64}` over the body for a card **or a calendar resource** with no `UID`. The primary ids are the bare `Message-ID` / `UID`, which is what the vectors give and what neverest's `mid:` prefix diverges from.
- [x] **Not done, and not to be done.** Annex A.2 no longer says casefold: it names the Unicode simple lowercase mapping, locale-independent, then a trim, and the vectors stay ASCII because a non-ASCII case would pin behaviour the two runtimes have not been checked on. Casefolding would fold `ß` to `ss` and put this crate at odds with the format and with both implementations. Lowercase-then-trim landed; Rust exposes only the full mapping, which differs from the simple one on the Greek final sigma and on `İ` alone.
- [x] Tests: all sixteen cases of the format's `vectors/meta.json`, which carry every hedged case the task lists, with the fixtures read as bytes and their `blake3` name asserted first; plus both fallbacks, a folded header with an address list and an obsolete zone, and an override-only resource beside a `VTIMEZONE` that never changes.
- [x] CHANGELOG.
- [x] Fold `delta.md` into the new `cairn/spec/conventions.md`; log; land.
- [ ] **Downstream, neverest**: drop src/kind/mail.rs's `MetaSummary` and link-id fallback.
- [ ] **Downstream, cardamum**: drop src/pimdir/card.rs's `CardSummary`, `sort_key` and `link_id`.
- [ ] **Downstream, calendula**: drop src/pimdir/meta.rs.
- [ ] **Downstream, Android**: write the Annex A.3 calendar summary and a real sort key (EventStore's `replaceEvents` and `replaceEvent` write `""` today, and `PimdirMeta.calendarSortKey` has no callers); move the `etag` out of `meta` and onto `bindings.base_revision`; add `in_reply_to` to `PimdirMeta.mail`.

The four downstream tasks stay unchecked: this change is io-pimdir's half. neverest's is the one with a cost, since its `mid:` prefix means adopting the bare id re-links every item in every existing store.
