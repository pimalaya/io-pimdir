---
cairn: delta
change: linked-and-streamed-body-columns
---

# Delta

## ADDED Requirements

### Requirement: A body gaining a row states the columns only the body gives
A write SHALL state, on the mail row of every live, stored item gaining a body (pimdir STORAGE Annex A.1), the columns only the body gives, over whatever the writer read without it. A body entering the store (no row of its key in the account held it before the write, `lookup_objects`) SHALL be walked once (`summary::mail::attachments::PimdirAttachmentWalk`), its mark stated and its invitation stated when it gives one, the same walk answering its stand-ins; a writer-derived key's body is walked for these too and stands for nothing. A body the account already holds SHALL be linked: the row's size is the object's, and its mark and invitation those a row holding that body states (`held_mail_summary`), the writer's kept where no such row states one.

#### Scenario: A streamed body
- tests/attachments.rs `a_body_read_without_it_states_what_its_parts_give`: the header block's guess (an attachment, no invitation) written with the body reads as the body gives (none, the `UID`).

#### Scenario: A copy of a held body
- tests/attachments.rs `a_copy_of_a_held_body_takes_the_columns_its_body_gave`: a placement written with a held hash and its listing's summary takes the held row's mark and invitation and the body's size, and records no stand-in.

## MODIFIED Requirements

### Requirement: The walk of a message's parts is streamed and bounded
`summary::mail::attachments::PimdirAttachmentWalk` SHALL take a body a chunk at a time and keep at most `HEADER_MAX` octets of a header block, `LINE_HEAD` of any other line, one boundary per open level down to `MIME_DEPTH_MAX`, `ATTACHMENTS_MAX` summaries and one unfolded calendar line of `CALENDAR_LINE_MAX` octets, answering the same whatever the chunking; it SHALL stand for exactly the parts the attachment mark's walk examines and finds disposed as attachments, a part no delimiter ends being none. `finish_parts` SHALL answer the attachment mark and the invitation `summary::mail::derive` gives for the same octets, within those bounds: a `UID` line past `CALENDAR_LINE_MAX` reads as none and ends the search.

#### Scenario: The vectors' attachments
- tests/summaries.rs, the walk fed whole and in chunks of 7 octets, its mark and invitation the row's; the unit tests of src/summary/mail/attachments.rs.

#### Scenario: Streamed or whole, alike
- tests/walk_agrees.rs: a table of encodings, foldings, nestings and truncations, and a property over built messages fed whole, by 1, 7 and 64 KiB octets and cut anywhere.

### Requirement: Decoding follows Annex A.0
A mail header SHALL be decoded (RFC 2047 encoded words, adjacent ones joined) and every address made canonical (the addr-spec alone, lowercased whole, `mailto:` stripped); a vCard or iCalendar text value SHALL be unescaped, a content line split on the first colon outside a quoted parameter, a structured value on the semicolons no backslash escapes. Invalid UTF-8 is replaced, never refused, once the octets are decoded: a transfer encoding is undone on the octets, never on text read from them. Unfolding a header removes the CRLF alone and keeps the whitespace (RFC 5322 §2.2.3); unfolding a vCard or iCalendar line takes the one leading character with it.

An encoded word's charset is decoded by the byte for `iso-8859-1` and `us-ascii`, by its table for `windows-1252` (the 0x80 to 0x9F row is not latin1), and as UTF-8 otherwise. The crate depends on no charset library, so any other 8-bit charset reads its bytes as UTF-8, lossily: a known deviation from Annex A.0, bounded to the headers of messages in charsets the format's vectors do not carry.

### Requirement: A linked body carries the base with it
A `Full` upgrade that resolves a placement's body from the object store instead of fetching it SHALL record that body as the placement's base content, exactly as the fetch path does. A placement holding a body its base does not is the shape of a staged local edit, so a storage projects it dirty and the consumer re-derives a change nobody made, on every sync, without ever converging. The write landing the link SHALL state the columns the body gave (store: "A body gaining a row states the columns only the body gives"), not those the listing read.

#### Scenario: A second label linked from the store
- tests/attachments.rs `an_upgrade_linking_a_held_body_takes_the_columns_its_body_gave`: no fetch, the row's mark and invitation the held row's.
