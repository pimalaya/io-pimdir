---
cairn: change
change: own-the-kind-conventions
---

# Delta

## ADDED Requirements

### Requirement: The crate owns the per-kind derivations
`conventions::derive(kind, body)` SHALL return the `link_id`, the `meta` and the `sort_key` spec Annex A fixes for `message/rfc822`, `text/vcard` and `text/calendar`, and `None` for a media type it has no conventions for. It SHALL live in the I/O-free core, so a consumer running its own SQLite driver reaches it, and it SHALL NOT be reachable through any store handle: it is a library a writer calls before the bytes reach the store, exactly as `hash` is one for naming them. §13's rule that the store never parses `meta` is unchanged.

A consumer SHALL derive through it rather than carry its own copy. Annex A is informative because the store never parses either value, but it is still an agreement every writer of one collection keeps: two writers disagreeing produce a collection whose rows one of them cannot render or order, and neither is in a position to notice, exactly as two hashes produce blobs neither finds.

This does not make Annex A normative. It makes one implementation of it the one three consumers share, in the place they all already depend on.

### Requirement: A fallback id is fixed, not left to the writer
Content carrying no usable identity SHALL still be linked, and the derived id SHALL be the same for every writer of this crate. A message with no `Message-ID` links as `alt:{subject}|{date}|{from}`, and a card or a calendar resource with no `UID` as `hash:{fnv1a64}` over the body.

The format leaves this open deliberately (§16: "check that one was derived, never that it equals a value"), which is exactly why it has to be closed here: two writers disagreeing about the id of one item link it twice and store one body twice, which is the object-hash bug on the identity axis. The mail fallback keys on what identifies the message rather than on its bytes, because the same message read at two detail tiers is two byte strings and one item.

### Requirement: The derivations are checked against the format's vectors
The crate SHALL be checked against the specification's `vectors/meta.json` for each kind it derives, reading the fixtures as bytes and comparing parsed structures rather than JSON text. Those vectors were derived from the prose rather than from any implementation, so they are what a claim to implement the conventions means; a vendored copy is not a substitute, and the test skips rather than passes when the spec checkout is absent.

## MODIFIED Requirements

None.

## REMOVED Requirements

None.
