---
cairn: spec
capability: conventions
status: current
---

# Conventions

The per-kind `link_id`, `meta` and `sort_key` a writer derives from an item's
bytes (spec Annex A). The store never parses `meta` and this capability does not
change that: it is a library the *writer* calls before the bytes reach the
store, the way `hash` is one for naming them.

Annex A is informative because the store never parses either value, but it is
still an agreement every writer of one collection keeps. Nothing enforces it and
no error reports a disagreement: two writers produce a collection whose rows one
of them cannot render or order, and neither is in a position to notice, exactly
as two hashes produce blobs neither finds. Three consumers implementing it
separately had already produced exactly that.

### Requirement: The crate owns the per-kind derivations
`conventions::derive(kind, body)` SHALL return the `link_id`, the `meta` and the
`sort_key` Annex A fixes for `message/rfc822`, `text/vcard` and `text/calendar`,
and `None` for a media type it has no conventions for. It SHALL live in the
I/O-free core, so a consumer running its own SQLite driver reaches it, and it
SHALL NOT be reachable through a store handle. A consumer SHALL derive through
it rather than carry its own copy.

Each kind SHALL be read by a shallow scanner rather than by a content parser: a
body crosses this crate byte for byte, the fields a summary holds are a handful
of properties, and the core is `no_std`. The parsers a frontend renders with
belong where the rendering happens.

### Requirement: A fallback id is fixed, not left to the writer
Content carrying no usable identity SHALL still be linked, and the derived id
SHALL be the same for every writer of this crate: `alt:{subject}|{date}|{from}`
for a message with no `Message-ID`, `hash:{fnv1a64}` over the body for a card or
a calendar resource with no `UID`.

The format leaves this open deliberately, which is why it has to be closed here:
two writers disagreeing about the id of one item link it twice and store one body
twice. The mail fallback keys on what identifies the message rather than on its
bytes, since the same message read at two detail tiers is two byte strings and
one item.

### Requirement: A time is resolved from what the resource carries
A calendar `sort_key` SHALL resolve a zoned start through the `VTIMEZONE` the
resource itself carries, and through nothing else: this crate holds no zone
database and reads no clock. A zone that will not resolve SHALL read as floating
rather than be dropped, since the error is then bounded by an offset where an
empty key would move the item to the far end of the listing.

An ambiguous or nonexistent wall time SHALL take the numerically greater of the
two offsets, which is the earlier instant. Annex A states the two cases
separately (the offset in effect *before* a fall-back, *after* a spring-forward)
and they are the same offset, so one rule serves both.

### Requirement: The derivations are checked against the format's vectors
The crate SHALL be checked against the specification's `vectors/meta.json` for
each kind it derives, reading the fixtures as bytes and comparing parsed
structures rather than JSON text. The test SHALL skip rather than pass when the
spec checkout is absent.

#### Scenario: A convention drifts
- GIVEN a fixture whose expected summary the vectors carry
- WHEN this crate derives a different one
- THEN the check fails naming the case, rather than a reader discovering a blank
  row
