---
cairn: log
change: a-truncated-message-walks-alike
date: 2026-11-01
---

# A truncated message walks alike

Found by the property `a_built_message_reads_alike_streamed_or_whole` of tests/walk_agrees.rs on 2026-11-01, shrunk to `seed = 1794814323191950817, crlf = true, truncate = Some(1242), cut = 0`: a three-level multipart message cut one octet short, its last line `--b047--\r` with no `\n`. `derive` read the attachment mark `true` (a `Content-Disposition: attachment` PDF under `b250`), the streamed walk (`PimdirAttachmentWalk`) `false`.

## Where the two parted

- `derive` (src/summary/mail.rs `parts`) strips one `\r` from a line's end whether a `\n` or the end of the body follows it, so `--b047--\r` closed the outermost level and every part stood.
- `PimdirAttachmentWalk::finish_parts` fed a `\r` pending at the end of the message to the line as content, so `--b047--\r` was no delimiter (a `\r` is no space or tab, RFC 2046 §5.1.1), the outermost level never closed and its child under way, holding the whole message, was no part: no mark, no stand-in, no invitation.

The whole derivation is the reference: Annex A.1 binds a writer reading the body as it streams to derive "the mark and the invitation as the whole body gives them", the rows hold `derive`'s columns, and the crate reads a line's trailing `\r` the same way everywhere else (`unfold` for header blocks and calendar lines, the streamed calendar reader). The streamed walk was the one outlier, so it moved; no derived column moves.

A wider search (every truncation of about 3,000 built messages, and stray `\r`, `\r\r`, `\n`, space, `-`, tab, `=`, `\r\n`, `\n\r`, `\r \r`, `--` and `\xff` inserted at every offset of about 1,000 more, near 12 million cases) found one more disagreement of the same family: a header line holding only a `\r` before its CRLF (raw `\r\r\n`). `split_headers` and `unfold` read it as a line holding `\r`, no empty one, so the header block goes on; the streamed walk kept the line as `\r` and then a `\n`, which `unfold` took for a CRLF and so for an empty line, ending the fields there, the dispositions after it lost.

## What landed

- src/summary/mail/attachments.rs: `finish_parts` reads a `\r` ending the message as the last line's break: a delimiter still delimits, a header line drops it, a part's content keeps it (as the derivation's slice of the part does, so no stand-in's size moves). `header_line` ends a line whose own last octet is `\r` with `\r\n`, so the block reads that `\r` as the line's.
- tests/walk_agrees.rs: the table gains a delimiter cut before its line feed, a header line holding a `\r` and a header block cut after its `\r`; `a_built_message_cut_before_its_last_line_feed_reads_alike` replays the shrunk case. The first two failed before the fix (the mark `false` against `true`); the third guards the header change. The property builds through `built`, and tests/walk_agrees.proptest-regressions keeps the case.
- Capabilities moved: summaries ("The walk of a body is bounded in depth": how a line ends; "Streamed or whole, alike": the new cases).

## Not done

pimdir STORAGE Annex A.1 states line ends for the header block alone ("a line at CRLF or a bare LF") and walks the parts by RFC 2046 §5.1, whose lines end at CRLF. Neither says whether a `\r` ending the message, with no `\n` after it, ends the last line or is its content, which decides whether a closing delimiter cut there closes the multipart, and with it the mark, the invitation and the stand-ins of every part. Read literally, the header rule makes it content (a lone `\r` is no CRLF), and `--b--\r` delimits nothing; this crate reads it as the line break's, as its `unfold` already did for header fields and calendar lines, keeping `derive`'s columns. Proposed to the format: a line of a message ends at CRLF, a bare LF or the end of the message, one `\r` before the end read as the line break's.
