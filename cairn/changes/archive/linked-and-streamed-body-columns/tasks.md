---
cairn: tasks
change: linked-and-streamed-body-columns
---

# Tasks

- [x] Review the uncommitted `Calendar` reader of the walk against `derive`: kept, the mark added to `PimdirParts`, `parts(body)`.
- [x] `derive` decodes a calendar part's octets before reading them as text.
- [x] tests/walk_agrees.rs: a table (base64, quoted-printable, soft breaks, folded `UID`s, nested and attached calendar parts, a truncated message, an inner part cut short, a foreign base64 byte, no `UID`, a disposed multipart) and a property over built messages cut anywhere; the line bound.
- [x] tests/summaries.rs: the walk's mark and invitation agree with every vector's row.
- [x] `held_mail_summary` in pimdir and vendored; `client::attachments::gaining` in `apply`.
- [x] tests/attachments.rs: a streamed body's columns, a copy of a held body, an upgrade linking one.
- [x] pimdir STORAGE Annex A.1, §14, SYNC §6, GUIDE.
- [x] Fold, log.
