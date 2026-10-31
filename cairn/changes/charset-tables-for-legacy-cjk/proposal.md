---
cairn: change
id: charset-tables-for-legacy-cjk
status: landed
created: 2026-10-29
---

# Charset tables, opt-in, for legacy CJK headers

## Why

A neverest deployment against a legacy CN mail server (~18k messages archived) filed every GBK-subject message into `mail_summary` as replacement characters: that mail emits `=?gbk?B?` encoded words as its default, and `charset_decode` reads every charset beyond latin1/1252 through `from_utf8_lossy` — Annex A.0's documented deviation. The held bodies stay intact (himalaya's reader path carries the tables; cf. himalaya#706 enabling mail-parser `full_encoding` for the same class of mail), but the listing and search tier, which is why the summary row exists, is unreadable for the majority of a real CN mailbox, and the derivation-time copy cannot be repaired from the store alone without re-deriving from each body.

The stated reason for the deviation is dependency discipline: the I/O-free core carries no charset tables, which keeps the default build lean for consumers whose mail is UTF-8. That is a right default and a low ceiling: a consumer facing legacy CJK mail cannot opt in short of patching the crate.

## What

An optional, non-default feature `charset-tables`:

- Cargo.toml: `encoding_rs` optional (`default-features = false, features = ["alloc"]`), `charset-tables = ["dep:encoding_rs"]`.
- `charset_decode`: under the feature, a name beyond the built-in three is looked up as a WHATWG label and decoded by its encoding (gbk, big5, euc-jp, iso-2022-jp, shift_jis, euc-kr, the windows-125x and iso-8859-x rows, utf-16 among them) before the lossy fallback; an unknown label falls back exactly as today.
- Tests beside the decoders, a real legacy GBK subject as the vector: default build lossy, feature build exact.

Default behaviour is byte-for-byte unchanged; "the crate carries no charset tables" moves from an absolute to the default. neverest follows with one line (`io-pimdir = { ..., features = ["charset-tables"] }`), out of tree. On landing: fold delta into `cairn/spec/summaries.md`, append the log entry.

