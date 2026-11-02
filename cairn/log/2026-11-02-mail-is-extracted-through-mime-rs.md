---
cairn: log
change: mail-is-extracted-through-mime-rs
date: 2026-11-02
---

# Mail is extracted through mime-rs

The `search` feature drops mail-parser 0.11 for mime-rs (lib `mime`), pinned by git rev d7ea032 as himalaya pins it, with `parser`, `base64` and `encoding`, all `no_std` with `alloc` as the crate is. mail-parser and hashify leave Cargo.lock; mime-rs brings base64-codec, quoted-printable-codec and simd-cpu, and encoding_rs it already held under `charset-tables`. The search spec's extraction requirement is restated over mime-rs, with the rules the port had to make explicit.

- src/search/extract.rs: `PimdirExtract::mail` parses with `MimeCst::parse_with`, the parse's entity bound lifted (`max_entities: Some(usize::MAX)`) so a multipart holding the 10,000th entity is still split, mime-rs leaving one past its bound whole. `Walk::over` numbers the entities the Annex A.1 walk reads, in document order, each with its children's ids, `None` for one left unread, so an alternative with no `text/plain` child still picks its last child, read or not, as before. Headers read as mail-parser read them: a repeated field as its last occurrence (`last`, which `thread` shares), a mailbox list flattened with its groups' members. A leaf's text is `decoded()` then `codec::charset::transcode`, not `text()`, so flowed text stays as written and no byte order mark is sniffed, as before; a US-ASCII label reads as windows-1252, as mail-parser and the WHATWG Encoding Standard read it, where mime-rs reads UTF-8. A part of a `multipart/digest` stating no type is a `message/rfc822` (RFC 2046 §5.1.5) and gives no text, which mime-rs leaves a leaf when its content is encoded or opens with no field. A body whose first block holds no header field, which mime-rs reads as all body, is read as mail-parser read it: the text after its first line of blanks, `unparseable` without one.
- src/search/html.rs: `to_text`, the reduction of `text/html` to text, ported from mail-parser 0.11.9's `html_to_text` with its 2,125 named character references, mime-rs having no HTML yet (its D8, an `html` feature, is open in mime's a-mime-library change). Its known answers are mail-parser's own tests.
- src/search/thread.rs: `parent` reads the header block alone (`structure_depth: Some(0)`), `None` when it holds no field, the last `In-Reply-To` and `References` fields read as message ids.
- tests/search_pure.rs: `the_extraction_reads_mail_as_it_always_has` locks the last of a repeated field, the US-ASCII label, the untyped digest part and the headless body; it passes against the mail-parser code too.

Checks: build, every test and clippy with `--all-features`. A differential run, the old extraction and `parent` against the new over 1,764 inputs (the 294 .eml files of mail-parser 0.11.9's resources, mime's tests/corpus and spec/, each as written, in LF, in CRLF and cut at a third, a half and four fifths), and `html_to_text` against `to_text` over 5,288 slices of them, which agree on every one.

Differences left, all on malformed or legacy input:

- Malformed structure follows mime-rs, which follows RFC 2046 §5.1.1 and the summaries' own `delimits`: a `--boundary` mid-line or with a longer boundary after it no longer delimits (stalwart-issues/156, where mail-parser cut the text at `visit --BND for details`), a nested multipart reusing its parent's boundary yields the outer parts, an open delimiter at the end opens an empty part, and a cut or unterminated part keeps its text, transfer-decoded. mime's tests/corpus/divergence.tsv states each with its reason. The extraction now agrees with the stand-in walk on these, which mail-parser did not (2026-10-13 stand-ins log).
- Headers follow mime-rs: a smuggled name (`Sub ject`, a tab, a NUL, a leading colon) is no field, a header block cut before its blank line still reads (spec/vectors fixtures mail-header-cut, mail-stray-cr, mail-military-zone, which mail-parser read empty or as body), a date no longer swallows the next field, and a header block of name fragments with no colon is `unparseable` rather than an empty `ok`.
- A mailbox written as a lone word (`From: Whomever`) indexes as `Whomever`, not `Whomever ` with mail-parser's trailing space: the same tokens.
- Bytes past base64 padding and an odd trailing byte of UTF-16 decode as encoding_rs and mime-rs decode them.
- UTF-7 (`=?utf-7?` words and `charset=utf-7` bodies) and IBM850 are not decoded: encoding_rs has neither, and mime-rs reads an unknown label as UTF-8. An RFC 2047 word labelled US-ASCII holding 8-bit bytes reads as UTF-8 in headers, the windows-1252 rule covering bodies alone. Each is mime-rs's to close, in `codec::charset::transcode`, which its RFC 2047 decoding shares; until then such mail indexes its raw `+AOk-` runs.

Left: the index version did not move, so an index built by the mail-parser extraction keeps its terms and chunk offsets until it is rebuilt or the item refolded: a snippet of a message whose text changed above may be cut from shifted offsets (read through `get`, so never out of bounds).
