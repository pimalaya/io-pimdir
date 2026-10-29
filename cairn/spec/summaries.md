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
A mail header SHALL be decoded (RFC 2047 encoded words, adjacent ones joined) and every address made canonical (the addr-spec alone, lowercased whole, `mailto:` stripped); a vCard or iCalendar text value SHALL be unescaped, a content line split on the first colon outside a quoted parameter, a structured value on the semicolons no backslash escapes. Invalid UTF-8 is replaced, never refused, once the octets are decoded: a transfer encoding is undone on the octets, never on text read from them. Unfolding a header removes the CRLF alone and keeps the whitespace (RFC 5322 §2.2.3); unfolding a vCard or iCalendar line takes the one leading character with it.

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

### Requirement: The walk of a body is bounded in depth
The walk of a message's parts SHALL open multipart bodies down to `summary::mail::MIME_DEPTH_MAX` (64) levels, the message itself at zero: a part nested deeper is examined neither for the attachment mark nor for the invitation. The calendar derivation SHALL keep components down to the third level (an observance under a `VTIMEZONE` under the `VCALENDAR`), the deepest any column reads, dropping deeper ones as they close. Either walk SHALL fit a 1 MiB stack whatever the body nests, a sender controlling both trees. pimdir Annex A.1 states no depth yet: a deviation until it does, bounded to messages no mail agent writes.

#### Scenario: Deep nesting derives within a small stack
- tests/deep_nesting.rs, each case on a 1 MiB thread: 10000 multipart levels and 200000 nested components derive; the walk reaches a part at the bound and not one past it.

### Requirement: The walk of a message's parts is streamed and bounded
`summary::mail::attachments::PimdirAttachmentWalk` SHALL take a body a chunk at a time and keep at most `HEADER_MAX` octets of a header block, `LINE_HEAD` of any other line, one boundary per open level down to `MIME_DEPTH_MAX`, `ATTACHMENTS_MAX` summaries and one unfolded calendar line of `CALENDAR_LINE_MAX` octets, answering the same whatever the chunking; it SHALL stand for exactly the parts the attachment mark's walk examines and finds disposed as attachments, a part no delimiter ends being none. `finish_parts` SHALL answer the attachment mark and the invitation `summary::mail::derive` gives for the same octets, within those bounds: a `UID` line past `CALENDAR_LINE_MAX` reads as none and ends the search.

#### Scenario: The vectors' attachments
- tests/summaries.rs, the walk fed whole and in chunks of 7 octets, its mark and invitation the row's; the unit tests of src/summary/mail/attachments.rs.

#### Scenario: Streamed or whole, alike
- tests/walk_agrees.rs: a table of encodings, foldings, nestings and truncations, and a property over built messages fed whole, by 1, 7 and 64 KiB octets and cut anywhere.

### Requirement: A task carries its organisation
A `VTODO` summary SHALL carry the four columns pimdir STORAGE Annex A.4 reads without the body: `priority`, `PRIORITY` 0 to 9, `None` when absent or out of range; `categories`, every `CATEGORIES` value in document order, each property's list split on the commas no backslash escapes, unescaped, empty values dropped; `parent`, the first `RELATED-TO` whose `RELTYPE` is `PARENT` or absent, verbatim; `mid`, the message id of the first `ATTACH` or `URL` holding a `mid:` URI, before any `/`, percent-decoded. An `X-` property is left in the body. The store writes `categories` as a JSON array, and the readers and the CLI's summary output carry all four.

#### Scenario: The organised task vector
- tests/summaries.rs, the case "todo, organised" of pimdir vectors/summaries.json; the unit test `a_task_reads_its_organisation_on_annex_a4s_edges` of src/summary/calendar.rs (a priority out of range, an escaped comma, a `CHILD` relation skipped, a `mid:` URI with a part).

### Requirement: A reader reads the task columns an owner has not reconciled as NULL

`PimdirReader` SHALL probe `task_summary` for `priority`, `categories`, `parent` and `mid` before every statement naming them (`list_tasks_page_asc`, `get_task`, `load_task_summaries`) and read each one the store lacks as `NULL`, so a listing on a store an owner at io-pimdir 3b4bed8 reconciled never fails "no such column" (STORAGE §6). The query client SHALL do the same on the attached store: `priority:` reads such a task as of no priority, `category:` as carrying none, and an item read from its summary reads the columns `NULL`.

#### Scenario: the 3b4bed8 shape lists
- GIVEN a store whose `task_summary` has the 3b4bed8 shape, holding a task stating priority 1 and `CATEGORIES:inbox,home`
- WHEN a reader lists the collection, reads the item, lists the trash, and a query client asks `category:inbox` and `priority:none`
- THEN every read answers, the task with no priority and no category, `category:inbox` matching nothing and `priority:none` matching it; once the owner opens the store, the columns are backfilled and read (tests/older_task_summary.rs)
