---
cairn: change
id: linked-and-streamed-body-columns
status: landed
created: 2026-10-14
---

# A body gaining a row states what only the body gives

## Why

Two gaps were left by stand-ins-as-the-body-enters (neverest's log of 2026-10-13, "Not done"):

1. **A body linked without a fetch keeps its listing's guess.** When a placement gains a body the account already holds (the upgrade's `LookingUp` path, an IMAP COPY or MOVE, a Gmail second label, a direct write of a held hash), the row keeps the summary its listing read off the header block: the attachment mark guessed from the top-level `Content-Type`, no invitation, the server's size or none. `summary::without_body` then protects that guess on every later listing, since the row now holds a body, so it never heals.
2. **The invitation of a streamed body stays unknown.** A writer streaming a body to its blob (neverest) never holds it whole, so it cannot call `derive`: it reads the mark with `PimdirAttachmentWalk`, and leaves the invitation `NULL`.

## What

- `summary::mail::attachments`: the walk also reads the invitation (Annex A.1: the first `UID` of the first `text/calendar` part stating one, transfer encoding undone, lines unfolded) as the octets go by, keeping one unfolded line of at most `CALENDAR_LINE_MAX` (64 KiB), and answers the mark as `derive` reads it (a top-level multipart disposed as an attachment included): `PimdirParts { attachments, attachment, invitation }`, by `PimdirAttachmentWalk::finish_parts` and `parts(body)`. `done()` waits for a calendar part's `UID`.
- `summary::mail`: the transfer encoding of a calendar part is undone on its octets, the text read only after (it was read lossily first, so a raw non-UTF-8 octet beside a quoted-printable escape read differently from a streamed walk).
- `client::write::apply` (`client::attachments::gaining`), before the diff is saved, for every body a mail item gains: a body entering is walked once, its mark and invitation stated on the row over the writer's, its stand-ins recorded as before (none for a writer-derived key, which is now walked for its columns too); a body the account holds is linked, the row taking its size from the object and its mark and invitation from a row holding it (`held_mail_summary`, a new canonical statement), the listing's mark kept when no such row states one.
- pimdir: `held_mail_summary`, Annex A.1's rule for a linked placement and a streamed body, SYNC §6 and GUIDE's upgrade step.
