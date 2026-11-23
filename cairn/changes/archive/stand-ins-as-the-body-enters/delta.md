---
cairn: delta
change: stand-ins-as-the-body-enters
---

# Delta

## ADDED Requirements

### Requirement: A body entering the store stands for its attachments
A write SHALL record, in its transaction, the stand-ins of every `message/rfc822` body it brings into the store (STORAGE §14.3): a body a live, stored item under a key naming an identity gains, which `lookup_objects` did not find under that key in the account before the write. Each part `summary::mail::attachments` answers SHALL get a file at `Meta` holding no body in the account's attachments collection, keyed `part:`, with its `file_summary`, unless the collection holds it, and an `auto` `attachment` reference from the message. The collection SHALL be the account's file collection with the role `attachments`, else `<account>/\u0001attachments` (`\u0001attachments` with no account) declared with the file kind, the name `attachments` and the role. A body filed again, stored again or held before SHALL walk nothing: no backfill, no recompute.

#### Scenario: Recorded once, as the body enters
- tests/attachments.rs; sync vector 70 in tests/vectors_sync.rs.

### Requirement: The walk of a message's parts is streamed and bounded
`summary::mail::attachments::PimdirAttachmentWalk` SHALL take a body a chunk at a time and keep at most `HEADER_MAX` octets of a header block, `LINE_HEAD` of any other line, one boundary per open level down to `MIME_DEPTH_MAX` and `ATTACHMENTS_MAX` summaries, answering the same whatever the chunking; it SHALL stand for exactly the parts the attachment mark's walk examines and finds disposed as attachments, a part no delimiter ends being none.

#### Scenario: The vectors' attachments
- tests/summaries.rs, the walk fed whole and in chunks of 7 octets; the unit tests of src/summary/mail/attachments.rs.

## MODIFIED Requirements

### Requirement: The writer records the automatic references
A write recording an item's summary SHALL run the canonical rule statements for it (STORAGE §14.2): `link_senders_of` for a mail, and `link_invitations_of` when it carries an invitation; `link_mail_from` for a contact; `link_invitations_to` for a calendar item. Each records origin `auto` from the mail to what it names, never twice and never over a person's reference, and never removes one: a link made at the write stays when its end later changes. `PimdirMailSummary::invitation` SHALL be derived with the body (Annex A.1) and kept over a write without it; opening a store as its owner SHALL add the column and `mail_summary_by_invitation` when absent.

#### Scenario: Whichever end lands first
- tests/references.rs `the_writer_records_the_automatic_references`.

#### Scenario: Linked at the write, never recomputed
- tests/references.rs `a_rule_links_at_the_write_and_never_recomputes`: the sender at `Meta`, the invitation once the body states it, both kept when the contact drops the address.
