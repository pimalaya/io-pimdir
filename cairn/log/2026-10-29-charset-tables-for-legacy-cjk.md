---
cairn: log
change: charset-tables-for-legacy-cjk
date: 2026-10-29
---

# Charset tables, opt-in, for legacy 8-bit header charsets

Follows this change's proposal (2026-10-29): the WHATWG label table behind a non-default feature, so a deployment facing legacy CJK mail can read its subjects.

- **summary**: `charset_decode` gains the `charset-tables` arm — `encoding_rs` (alloc-only, optional) consulted through `Encoding::for_label_no_replacement` and decoded `decode_without_bom_handling`, so the label holds against a byte-order mark. A label the table names with no encoding behind it (iso-2022-kr among them) and a label it does not name fall back to the lossy UTF-8 read; the default build is byte-for-byte unchanged. Guarded by `charset_table_tests` in src/summary/mail.rs: a real legacy GBK subject lossy without the feature and exact with it, gb2312/big5/shift_jis under the feature, the fallbacks and the BOM read pinned.

summaries.md's Annex A.0 requirement now states the feature's bound. A consumer facing legacy CJK mail opts in with `io-pimdir = { ..., features = ["charset-tables"] }`.
