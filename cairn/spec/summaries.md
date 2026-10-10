---
cairn: spec
capability: summaries
status: current
---

# Summaries

What a writer derives from an item before its row reaches the store (pimdir STORAGE Annex A): the key it is filed under, the row of its kind's summary table with the people it names, and its sort key. The store parses no body; `summary::derive` is the library a writer calls, and the kind modules expose their decoders so a connector building a `Meta` tier summary from an IMAP ENVELOPE lands on the same bytes.

### Requirement: The crate owns the per-kind derivations
`summary::derive(kind, body)` SHALL return the `PimdirDerivation` Annex A fixes for `message/rfc822`, `text/vcard` and `text/calendar`, and `None` for a media type it has no conventions for. It SHALL live in the I/O-free core. A calendar resource holding no `VEVENT`, `VTODO` or `VJOURNAL` derives a key and no summary.

### Requirement: Decoding follows Annex A.0
A mail header SHALL be decoded (RFC 2047 encoded words, adjacent ones joined) and every address made canonical (the addr-spec alone, lowercased whole, `mailto:` stripped); a vCard or iCalendar text value SHALL be unescaped, a content line split on the first colon outside a quoted parameter, a structured value on the semicolons no backslash escapes. Invalid UTF-8 is replaced, never refused. Unfolding a header removes the CRLF alone and keeps the whitespace (RFC 5322 §2.2.3); unfolding a vCard or iCalendar line takes the one leading character with it.

An encoded word's charset is decoded by the byte for `iso-8859-1` and `us-ascii`, by its table for `windows-1252` (the 0x80 to 0x9F row is not latin1), and as UTF-8 otherwise. The crate depends on no charset library, so any other 8-bit charset reads its bytes as UTF-8, lossily: a known deviation from Annex A.0, bounded to the headers of messages in charsets the format's vectors do not carry.

### Requirement: A fallback key is fixed
Content stating no identity SHALL still be keyed, the same way by every writer: `alt:{subject}|{date}|{sender}` for a message with no `Message-ID`, `hash:` over the FNV-1a 64 digest of the bytes (offset basis `cbf29ce484222325`, prime `100000001b3`) for a card or a resource with no `UID`, the key vectors/summaries.json pins.

### Requirement: The two tiers agree
A `PimdirMailSummary` built without the body (an envelope, `summary::mail::derive_meta` over the header block, a Graph `$select`) SHALL yield the same key, row and sort key as the `Full` derivation of the same message, `attachment` and `size` aside, which it reads without the body (below): `size` is what the source states, which may be an estimate (Graph's `PidTagMessageSize`, Gmail's `sizeEstimate`).

### Requirement: The attachment mark is read without the body
Read without the body, the mark SHALL be the source's own flag where it states one (Graph's `hasAttachments`, JMAP's `hasAttachment`), else `summary::mail::meta_attachment`: `true` when the top-level `Content-Type` is `multipart/mixed`, `false` otherwise (pimdir STORAGE Annex A.1). `derive_meta(header, size, attachment)` takes the source's flag as `attachment`. The walk of the parts replaces it, and the body's octets the stated size, once the body is read, and a summary read without the body SHALL NOT replace either over a placement holding its body (`summary::without_body`, for a listing, a `Meta` fetch and a rekey; vectors/sync/52, 53).

### Requirement: A time is resolved from what the resource carries
A calendar `sort_key` SHALL resolve a zoned start through the `VTIMEZONE` the resource carries and nothing else; an ambiguous or nonexistent wall time takes the numerically greater offset; a zone that will not resolve reads as floating; a date-only value reads as midnight UTC.

### Requirement: The derivations are checked against the format's vectors
tests/summaries.rs SHALL run every case of the specification's vectors/summaries.json: the fixture's names under both hashes, the key (the minted one where the case states a hint and a handle), the summary row, the address rows, the sort key and, for mail, the mark read without the body (`meta_attachment`) with every other column agreeing, compared as parsed structures, skipping when the spec checkout is absent.
