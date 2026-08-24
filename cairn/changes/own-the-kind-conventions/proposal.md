---
cairn: change
id: own-the-kind-conventions
status: active
created: 2026-08-24
---

# The crate owns the per-kind meta and sort-key derivations

## Why

This is the same leak as own-the-content-hash, one axis over, and it has already produced a divergence rather than merely risking one.

pimdir Annex A fixes what a `meta` blob holds and what a `sort_key` means per kind. It is informative, because the store never parses either, but it is still an agreement two writers of one collection must keep. Today each writer implements it alone:

- `message/rfc822`: neverest's `MetaSummary` (src/kind/mail.rs) and the Android app's `PimdirMeta.mail`.
- `text/vcard`: cardamum's `CardSummary` (src/pimdir/card.rs) and `PimdirMeta.contact`.
- `text/calendar`: calendula's src/pimdir/meta.rs and `PimdirMeta.calendarValidator`.

Mail agrees, field for field, and shows what the arrangement looks like when it works: both sides were written against the same prose, and the Java's two extra fields (`from_name`, `attachment`) are absorbed by unknown-field tolerance. Its only gap is that the Java has not picked up `in_reply_to`, added to Annex A.1 on 2026-08-16.

Contacts agree in practice and with each other, but with the prose only loosely: both lower-case the display name where Annex A.2 says casefold, so `ß` sorts apart from `ss` in both. Harmless until a third writer implements what the text says.

**Calendars have diverged completely.** calendula writes the whole Annex A.3 object and a resolved sort key; the Android app writes `{"v":1,"etag":…}` and an empty sort key for every item (EventStore.java, the `replaceEvents` and `replaceEvent` paths). So calendula reading a phone-written calendar has no summary to render a row from and no ordering at all, with `items_by_sort` inert for that collection, and the phone reading a calendula-written one has no `etag` to guard a push with. `PimdirMeta.calendarSortKey` exists, is carefully written, and has no callers.

Two further things that leak out of the same hole:

- `etag` is not an Annex A.3 field. It sits in `meta` because that path does not use `bindings.base_revision`, which is the column §13 defines for exactly a mutable-content validator.
- The **link-id fallbacks** are per-consumer digests (cardamum's `hash:{fnv1a64}`, neverest's `alt:` digest). Two implementations disagreeing on a fallback re-link items and store one body twice: the identical failure to the object-hash bug, on the identity axis.

## What

A `conventions` module beside `sql`, `codec` and `hash`, holding one `derive` per kind: raw body to `(link_id, meta, sort_key)`, with the fallback digests fixed here rather than per consumer. Consumers call it instead of carrying a copy.

This does not violate §13's "the store never parses `meta`", for the same reason `hash` does not violate the I/O-free rule: the module is a library the *writer* calls before handing bytes to the store. The store's own read and write paths keep treating `meta` and `sort_key` as opaque values they ferry.

It belongs in the I/O-free core (no I/O, `alloc` only), which is also what lets a consumer that runs its own SQLite driver reach it.

## Ordering

Worth landing after cross-implementation-vectors exists in the pimdir repository, or at least alongside it: the module makes the derivations agree once, and the vectors are what keep them agreeing, including with the Java side, which this module cannot reach.
