---
cairn: tasks
change: charset-tables-for-legacy-cjk
---

# Tasks

- [x] Cargo.toml: optional `encoding_rs` (default-features = false, features = ["alloc"]), feature `charset-tables = ["dep:encoding_rs"]`, default set untouched.
- [x] src/summary/mail.rs `charset_decode`: under the feature, WHATWG-label lookup through `Encoding::for_label_no_replacement`, decoded `decode_without_bom_handling` so the label holds against a byte-order mark, before the lossy fallback; doc comment states default and opt-in.
- [x] Unit tests `charset_table_tests`: a real `=?gbk?B?` subject asserts lossy without the feature and exact text with it; a feature-only test covers gb2312, big5, shift_jis; iso-2022-kr, an unknown label and a BOM-headed utf-8 vector pin the fallbacks and the label's hold.
- [x] `cargo test` and `cargo test --features charset-tables` green (unit, doc, property suites).
- [x] Landed in this change: delta.md folded into cairn/spec/summaries.md, cairn/log/2026-10-29-charset-tables-for-legacy-cjk.md appended, CHANGELOG.md carries the [Unreleased] line, proposal status landed.
- [ ] neverest follow-up: enable the feature on its io-pimdir dependency (one line + lockfile bump).
