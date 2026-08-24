---
cairn: tasks
change: own-the-kind-conventions
---

# Tasks

- [ ] `conventions` module in the I/O-free core, one `derive(body) -> (link_id, meta, sort_key)` per kind (`message/rfc822`, `text/vcard`, `text/calendar`), with the summary types serialised per Annex A.
- [ ] Fix the link-id fallbacks here (`alt:` for a message with no `Message-ID`, `hash:` for a card with no `UID`), so two writers cannot disagree and store one body twice.
- [ ] Casefold rather than lower-case the contact sort key, which is what Annex A.2 says.
- [ ] Tests: the Annex A shapes; the hedged cases (no `FN`, unparseable `Date`, zoned `DTSTART` on a fold and on a gap, `VTODO` with `DUE` and no `DTSTART`); the fallbacks.
- [ ] CHANGELOG.
- [ ] Fold `delta.md`; log; land.
- [ ] **Downstream, neverest**: drop src/kind/mail.rs's `MetaSummary` and link-id fallback.
- [ ] **Downstream, cardamum**: drop src/pimdir/card.rs's `CardSummary`, `sort_key` and `link_id`.
- [ ] **Downstream, calendula**: drop src/pimdir/meta.rs.
- [ ] **Downstream, Android**: write the Annex A.3 calendar summary and a real sort key (EventStore's `replaceEvents` and `replaceEvent` write `""` today, and `PimdirMeta.calendarSortKey` has no callers); move the `etag` out of `meta` and onto `bindings.base_revision`; add `in_reply_to` to `PimdirMeta.mail`.
