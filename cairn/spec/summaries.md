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
A vCard or iCalendar text value SHALL be unescaped, a content line split on the first colon outside a quoted parameter, a structured value on the semicolons no backslash escapes, and every address made canonical (the addr-spec alone, lowercased whole, `mailto:` stripped). Invalid UTF-8 is replaced, never refused, once the octets are decoded, each maximal subpart of an ill-formed sequence one U+FFFD (`String::from_utf8_lossy`): a transfer encoding is undone on the octets, never on text read from them. Unfolding a vCard or iCalendar line takes the one leading character with it. A mail header is read as Annex A.1 reads it (below).

### Requirement: A header is read as Annex A.1 reads it
`message_id`, `in_reply_to`, `subject`, `sender`, `sender_name`, `date` and an `alt:` key SHALL derive by the parses of pimdir STORAGE Annex A.1 at revision 3 (src/summary/mail.rs, each public for a connector reading a raw header field: `message_id`, `message_ids`, `decode`, `addresses`, `display_name`, `instant`, `charset_decode`):

- the header block ends at the first empty line, at CRLF or a bare LF; a field is unfolded (a line break before a space or a tab removed, the whitespace kept), read from its first occurrence, its value trimmed of spaces and tabs alone; comments (nested, a backslash quoting) and quoted-strings are RFC 5322 §3.2's, a comment read as one space where a rule removes it;
- `decode`, an unstructured value: every RFC 2047 encoded word decoded wherever it stands (a token charset, `B` or `Q` in either case, printable ASCII but `?`); a `B` word holding a character outside the alphabet, an `=` but in a trailing padding of at most two, or a lone sextet, a `Q` word holding an `=` not followed by two hexadecimal digits, and a word whose charset `charset_decode` does not know, each kept as its literal text; the whitespace between two decoded words removed and no other; the ends trimmed of spaces and tabs;
- `charset_decode`: a label of the WHATWG Encoding Standard, an RFC 2231 language dropped, matched without regard to ASCII case, decoded by its encoding with no byte order mark sniffed, `iso-8859-1` and `us-ascii` among the windows-1252 labels; a label naming the replacement encoding, or none, is unknown (`None`);
- `addresses` and `display_name`: an address list split at the commas outside a quoted-string, a comment and angle brackets, a group's name and colon dropped and its `;` splitting; a mailbox's address between its angle brackets, else its text with its comments removed; its display name the text before `<`, comments as spaces, each quoted-string its content with its quoted pairs undone, decoded, `None` when empty;
- `message_id`: comments skipped, the first `<` outside one to the next `>`, trimmed and not empty; with no such `<`, the field with its comments removed and trimmed when it holds no whitespace, no `<`, `>` or `"` and exactly one `@`, else `None`, case kept; `message_ids` (`In-Reply-To`) each such `<…>`, empty ones dropped, else each whitespace-separated word `message_id` would take;
- `instant` (`Date`): RFC 5322 §3.3 with §4.3's obsolete forms: comments removed, the day of the week up to the first comma skipped unchecked, five words (whitespace around a colon not counting), a day of one or two digits, the month in any case, a year of two digits or more (below 50 in the 2000s, 50 or more and three digits in the 1900s, then 1900 to 9999), `hh:mm[:ss]` of two digits each, a zone `+hhmm` or `-hhmm` (`mm` at most 59), `UT`, `GMT`, the six North American names, or one letter but `J` at `+0000`; the day existing in its month, the hour 00 to 23, a second of 60 read as 59, nothing carried into the next day; an instant past the year 9999 unparseable.

The link id is made once and never re-derived (SYNC §6): on a store an earlier revision wrote, the `alt:` key of a message stating no `Message-ID`, and the key of one stating a bracketless or commented one, keep the reading they were made with while their columns are re-derived (store.md, "A store derived under an earlier revision is re-derived on open").

Every build SHALL carry the charset tables: the UTF-8 and windows-1252 labels are read by the crate, every other WHATWG encoding through `encoding_rs`, a plain dependency (`no_std` with `alloc`), so the no_std core derives the subjects, display names and `alt:` keys every other build derives (Annex A.1, which has a writer carry the legacy indexes). The `charset-tables` feature is gone with the departure it bounded (decision 5 of the-crate-follows-storage-draft-08): a build without it kept such a word literal and derived other keys silently, and no alias feature replaces it. encoding_rs follows the Encoding Standard as it stands at its release, which the standard pins at the snapshot of 19 October 2026; `search` linked it already through mime-rs.

An RFC 2231 parameter (an attachment's `filename*`) is decoded by `charset_decode` too, its octets read as UTF-8 when the charset is unknown: Annex A.7 states no reading for one.

#### Scenario: The vectors
- tests/summaries.rs: the cases of vectors/summaries.json reading a bracketless id, one naming nothing, comments around the ids, a day the month lacks, the obsolete years and zones, no seconds, a folded and padded subject, malformed encoded words and raw 8-bit bytes.

#### Scenario: The parses the vectors do not reach
- src/summary/mail.rs unit tests, each failing against the reading before revision 3: `24:00`, a missing zone, a word after it, `J`, a digit, `+0160`, a six-character zone, a one-digit hour, 1899 and a four-digit 0026 unparseable; a comment before the time, whitespace around a colon, a leap second read as 59, a long year and a lowercase military letter read; a bracketless id holding a `"` or two `@`, `<>` and an unclosed `<` naming nothing, a bracketless `In-Reply-To` read word by word; `iso-8859-1` and `us-ascii` as windows-1252, three pads, a lone sextet, a no-break space kept at the ends, whitespace beside literal text kept; a comment inside a display name read as a space, a quoted-string inside one unquoted; a replacement label and an unknown one kept as written.

#### Scenario: A GBK subject
- GIVEN a message whose header reads `Subject: =?gbk?B?...?=`
- WHEN its summary is derived, in any build
- THEN the subject row holds the decoded text (src/summary/mail.rs `a_gbk_subject_is_read_in_every_build`; the five charset cases of vectors/summaries.json: windows-1252 behind `iso-8859-1`, ISO-8859-2, ISO-2022-JP, GBK and a replacement label kept literal)

### Requirement: A fallback key is fixed
Content stating no identity SHALL still be keyed, the same way by every writer: `alt:{subject}|{date}|{sender}` for a message with no `Message-ID`, `hash:` over the FNV-1a 64 digest of the bytes (offset basis `cbf29ce484222325`, prime `100000001b3`) for a card or a resource with no `UID`, the key vectors/summaries.json pins. A `UID` that is not usable, empty once trimmed of the space and the tab (no other character counting as whitespace here), SHALL take the same `hash:` fallback, its `uid` column kept verbatim (`summary::uid_key`, STORAGE §9; the cases on card-empty-uid.vcf and event-blank-uid.ics): until draft-10's second batch an empty `UID` was the key, one two unrelated resources would share. A resource holding overrides and no master is read from its first component carrying a `RECURRENCE-ID` (Annex A.3, event-override-only.ics). A sort key folds by `str::to_lowercase`, which applies Final_Sigma and no other conditional mapping, and trims by `str::trim`, the White_Space property (Annex A.2; card-final-sigma.vcf, card-nbsp-name.vcf). A header field's name is trimmed of trailing whitespace before it is matched, and a field stated twice is read from its first occurrence (Annex A.1; mail-spaced-field-name.eml, mail-repeated-fields.eml).

### Requirement: The two tiers agree
A `PimdirMailSummary` built without the body (an envelope, `summary::mail::derive_meta` over the header block, a Graph `$select`) SHALL yield the same key, row and sort key as the `Full` derivation of the same message, `attachment` and `size` aside, which it reads without the body (below): `size` is what the source states, which may be an estimate (Graph's `PidTagMessageSize`, Gmail's `sizeEstimate`).

### Requirement: The attachment mark is read from the parts
With the body, the mark SHALL be `true` when a part the walk reads makes it (pimdir STORAGE Annex A.1, `summary::mail::makes_mark`): its `Content-Disposition` type any but `inline`, or, stating no disposition, a media type neither `multipart/*` nor `text/*` and no `Content-ID`, the media type `text/plain` when absent (under a `multipart/digest` too) or holding not exactly one `/`. A multipart message's own entity, every child of a `multipart/encrypted` and every child but the first of a `multipart/signed` (RFC 1847) SHALL never make it. The derivation's walk and the streamed walk read the rule through the same functions.

#### Scenario: The mark's cases
- vectors/summaries.json "parts sent bare", "a message that is a file" and "an encrypted message"; src/summary/mail/attachments.rs `a_part_sent_bare_as_a_file_is_an_attachment`, `the_protocol_parts_of_an_encrypted_message_are_no_attachment` and `a_multipart_message_disposed_as_an_attachment_makes_no_mark`, each through both walks.

### Requirement: The attachment mark is read without the body
Read without the body, the mark SHALL be the source's own flag where it states one (Graph's `hasAttachments`, JMAP's `hasAttachment`), else `summary::mail::meta_attachment`: `true` when the top-level `Content-Type` is `multipart/mixed`, `false` otherwise (pimdir STORAGE Annex A.1). `derive_meta(header, size, attachment)` takes the source's flag as `attachment`. The walk of the parts replaces it, and the body's octets the stated size, once the body is read, and a summary read without the body SHALL NOT replace either over a placement holding its body (`summary::without_body`, for a listing, a `Meta` fetch and a rekey; vectors/sync/52, 53).

### Requirement: A zoned time resolves through the IANA database
A calendar `sort_key` SHALL resolve a zoned start through the first that names its `TZID` byte for byte: the resource's own `VTIMEZONE`, then the IANA time zone database `jiff` bundles, else as floating (pimdir STORAGE Annex A.3, RFC 7809). `jiff` (`alloc`, `tzdb-bundle-always`) is a direct no_std dependency of the core (decision 6 of the-crate-follows-storage-draft-08): the database ical-rs's `tzdb` feature brings to the index, one lock entry, so the sort key and the occurrences read one database version, and the no_std summary conforms too. A wall time a transition repeats or skips SHALL take the offset in effect before it (RFC 5545 §3.3.5): of a repeated one the earlier instant, a skipped one read at the offset before the gap, so `20260308T023000` in New York is `07:30:00Z`. A date-only value reads as midnight UTC, a floating one as UTC.

A `VTIMEZONE` observance's onsets are its `DTSTART` and what its `RRULE` yields when the rule steps by a fixed span (`SECONDLY` to `WEEKLY`, with `INTERVAL`, `COUNT`, `UNTIL`) or is `YEARLY` in one `BYMONTH` by `BYDAY`, an ordinal before it or a `BYMONTHDAY` list beside it, with `INTERVAL`, `COUNT`, `UNTIL`: the shapes zones are written in. A rule SHALL be followed for at most 1,000,000 periods from its `DTSTART`, the last onset found standing beyond them (Annex A.3). The resolution reads, of each observance, its first onset, its last before three days around the wall time and every one within them, so a rule stepping by the second costs a bounded walk; a rule of another shape contributes its `DTSTART` alone.

#### Scenario: The vectors
- tests/summaries.rs: a spring-forward gap, a fall-back fold, a `TZID` with no `VTIMEZONE`, an event's people under one, a zone whose rule steps by the second.

#### Scenario: What the vectors do not reach
- src/summary/calendar.rs unit tests: a skipped hour at the offset before (`the_ambiguous_and_skipped_hours_take_the_offset_before`, which asserted the offset after under revision 2), an observance rule stopping at its `UNTIL`, a weekly rule followed and stopped by its `COUNT`, the database's fold and gap, and `europe/paris` read as floating.

### Requirement: The derivations are checked against the format's vectors
tests/summaries.rs SHALL run every case of the vendored spec/vectors/summaries.json, 71 at pimdir c315220, none of them on tests/pending_vectors.txt: the fixture's BLAKE3 name, the key (the minted one where the case states a hint and a handle), the summary row, the address rows, the sort key, for mail the mark read without the body (`meta_attachment`) with every other column agreeing, and the stand-ins' parts, compared as parsed structures.

### Requirement: The walk of a body is bounded in depth
The walk of a message's parts SHALL read its entities in document order, a multipart before its children, the message's own at depth 0, a `message/rfc822` part one entity never opened, none deeper than `summary::mail::MIME_DEPTH_MAX` (64) and none after the `summary::mail::MIME_ENTITIES_MAX`th (10,000), the message's own counted (pimdir STORAGE Annex A.1): an entity left unread, with every part under it, makes no attachment mark, no invitation, no stand-in and no extracted text. The derivation's walk (`has_attachment`, `invitation`), the streamed walk (`PimdirAttachmentWalk`) and the search extraction (src/search/extract.rs, over mime-rs's tree) bound alike. A line delimits a multipart body only when it is `--` and the boundary, `--` after it when it closes, then spaces and tabs alone (RFC 2046 §5.1.1, `delimits`), so a boundary another begins with (`d1` and `d10`) delimits only its own body. A line of a body ends at CRLF, a bare LF or the end of the message, a `\r` ending the message read as its line break's, as `parts` and `unfold` read it: a delimiter cut before its `\n` still delimits, and a line holding only a `\r` before its CRLF is no empty one, so it ends no header block. The calendar derivation SHALL keep components down to the third level (an observance under a `VTIMEZONE` under the `VCALENDAR`), the deepest any column reads, dropping deeper ones as they close. Either walk SHALL fit a 1 MiB stack whatever the body nests, a sender controlling both trees.

Departure, bounded to messages no mail agent writes: the streamed walk counts a part as it begins, the derivation's walk the parts a delimiter ends, so in a message past 10,000 entities that also leaves a part unended, the two may bound at neighbouring entities.

#### Scenario: Deep nesting derives within a small stack
- tests/deep_nesting.rs, each case on a 1 MiB thread: 10000 multipart levels and 200000 nested components derive; the walk reaches a part at the bound and not one past it.

#### Scenario: The bounds
- vectors/summaries.json "the walk's depth" and "the walk's length"; src/summary/mail.rs `the_walk_reads_no_entity_past_its_bound` (the mark and the invitation of a 10,001st entity) and `a_boundary_another_begins_with_delimits_only_its_own_body`; tests/search_pure.rs `the_extraction_reads_the_parts_the_walk_reads` (a text part at depth 64 extracted, which the extraction skipped before, one at 65 and a 10,001st entity not).

### Requirement: The walk of a message's parts is streamed and bounded
`summary::mail::attachments::PimdirAttachmentWalk` SHALL take a body a chunk at a time and keep at most `HEADER_MAX` octets of a header block, `LINE_HEAD` of any other line, one boundary per open level down to `MIME_DEPTH_MAX`, and read at most `MIME_ENTITIES_MAX` entities, a part beginning past them skipped while its levels' delimiters are still read (a part they never end is still none); it SHALL stand for at most `ATTACHMENTS_MAX` (1,000) parts, the first the walk marks (STORAGE §14.3), and keep one unfolded calendar line of `CALENDAR_LINE_MAX` octets, answering the same whatever the chunking; it SHALL stand for exactly the parts the attachment mark's walk examines and finds making the mark, a part no delimiter ends being none. `finish_parts` SHALL answer the attachment mark and the invitation `summary::mail::derive` gives for the same octets, within those bounds: a `UID` line past `CALENDAR_LINE_MAX` reads as none and ends the search. A delimiter line longer than `LINE_HEAD` is read on its first `LINE_HEAD` octets.

#### Scenario: The vectors' attachments
- tests/summaries.rs, the walk fed whole and in chunks of 7 octets, its mark and invitation the row's; the unit tests of src/summary/mail/attachments.rs.

#### Scenario: Streamed or whole, alike
- tests/walk_agrees.rs: a table of encodings, foldings, nestings and truncations, a message cut between the `\r` and the `\n` of its closing delimiter and a header line holding a `\r` (`the_walk_reads_the_invitation_the_derivation_reads`, `a_built_message_cut_before_its_last_line_feed_reads_alike`, the property's case of 2026-11-01, kept in tests/walk_agrees.proptest-regressions), and a property over built messages fed whole, by 1, 7 and 64 KiB octets and cut anywhere.

### Requirement: A task carries its organisation
A `VTODO` summary SHALL carry the four columns pimdir STORAGE Annex A.4 reads without the body: `priority`, `PRIORITY` 0 to 9, `None` when absent or out of range; `categories`, every `CATEGORIES` value in document order, each property's list split on the commas no backslash escapes, unescaped, empty values dropped; `parent`, the first `RELATED-TO` whose `RELTYPE` is `PARENT` or absent, verbatim; `mid`, the message id of the first `ATTACH` or `URL` holding a `mid:` URI, before any `/`, percent-decoded. An `X-` property is left in the body. The store writes `categories` as `task_category` rows, and the readers and the CLI's summary output carry all four.

#### Scenario: The organised task vector
- tests/summaries.rs, the case "todo, organised" of pimdir vectors/summaries.json; the unit test `a_task_reads_its_organisation_on_annex_a4s_edges` of src/summary/calendar.rs (a priority out of range, an escaped comma, a `CHILD` relation skipped, a `mid:` URI with a part).

### Requirement: A task carries its capture and its contacts
A `VTODO` summary SHALL carry `created` (Annex A.4, revision 2), `CREATED` as an instant on Annex A.3's rules, else `DTSTAMP`, else `None`, and `contacts`, every `CONTACT` in document order, filed under `PimdirAddressRole::Contact` (`contact`): its address the one a `mailto:` `ALTREP` names, else its text, canonical either way (A.6); its name the text decoded, `None` when the address was read from a text that is an addr-spec, or when a `mailto:` `ALTREP` comes with an empty text; an empty text with no such `ALTREP` filing nothing. The store writes `created` and the `contact` rows, the readers and the CLI's summary output carry `created`. The search index's extraction keeps no `contact` field (SEARCH §6 reads a task's people from `ORGANIZER` and `ATTENDEE`), so `with:` reaches a waited-on person through `item_address` alone.

#### Scenario: The capture vectors
- tests/summaries.rs, the cases "todo, captured and waiting on an address" and "todo, waiting on a text" of pimdir vectors/summaries.json and the three earlier task cases' `created`; the unit test `a_task_reads_its_contacts_on_annex_a4s_edges` of src/summary/calendar.rs (a `CREATED` that is no instant, an empty `CONTACT`, a `mailto:` `ALTREP` with no text, another `ALTREP`).
