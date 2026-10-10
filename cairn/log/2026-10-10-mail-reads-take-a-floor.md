---
cairn: log
change: mail-reads-take-a-floor
date: 2026-10-10
---

# The mail reads take a floor, and a range sums

A reader listing mail down to a date cut the canonical reads' rows after the fact, so every count read every stored row of the shown collections; the Android download window also needs to say what a range of dates weighs.

Implements pimdir's mail-reads-take-a-floor (draft-04, untagged), spec/ re-vendored: `:since` on `count_mail`, `count_mail_by_day`, `count_unread` and `list_mail_page_filtered`, seeked on `items_by_sort` and `items_by_sort_global`, and the new `sum_mail` over `[:since, :until)`.

- `PimdirReader::count_mail`, `count_mail_by_day`, `count_unread` and `list_mail_page_filtered` take `since: Option<&str>` after the filter; `search_mail` is unchanged. Breaking for their callers, `None` restoring the old reads.
- `PimdirReader::sum_mail(collections, filter, since, until)` answers `PimdirMailSum { count, size, unknown }`.
- Capability moved: store (the mail reads).

## Verification

`tests/scoped_sync.rs` `the_mail_readers_take_a_floor_and_sum_a_range`: a floor keeps a key equal to it and drops the undated, per day and unread above it, a page ending at it; sums above a floor, open below (holding the undated), under the unread chip with the ceiling excluded, and empty. The plans are pimdir's invariants.sh. `cargo test --all-features`, `--no-default-features --features cli`, clippy `-D warnings` (all targets, and `--no-default-features --lib`) and `cargo fmt --check` green; `spec_drift` against the pimdir checkout.
