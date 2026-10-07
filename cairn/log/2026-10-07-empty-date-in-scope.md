---
cairn: log
change: empty-date-in-scope
date: 2026-10-07
---

# An empty mail date is stored as no date

Reported by the neverest implementer on `35a1c3f`: an undated based mail binding unlisted by a complete round was dropped when the round opened and closed in one run, and kept when the round resumed one an earlier run opened.

The two paths read the date differently. A round its last page opens filters the loaded placements with `PimdirScope::contains`, which reads an empty `date` as no usable date, in every scope (SYNC §5). A resumed round reads `list_unstamped_bindings`, which keeps a `NULL` `date` only: an empty string compares below any `since` and fell out of scope. The statement is right, STORAGE Annex A.1 storing an unparseable `Date` as `NULL`; the store was not, writing the empty `date` a summary carried as it came. A `Date` that does not parse, or no summary at all, already reached the store as `NULL`, and both paths agreed on it.

- `PimdirSummary::stored` reads an empty mail `date` as `None`, and the hub folds every placement's summary through it, so the store keeps `NULL` and the in-memory and SQL paths agree.
- Capabilities moved: sync (absence means deleted in scope only: an empty `date` is unknown on every path).

## Verification

`tests/scoped_sync.rs` `an_undated_member_unlisted_by_a_round_is_dropped`: no `Date`, an empty `date` and no summary, each through a round opened and closed in one run and one resumed from its cursor; it failed on the empty `date` of the resumed round before the fix. `cargo test --all-features`, `--no-default-features --features cli`, `--no-default-features --lib`, clippy and `cargo fmt --check` green; `spec_drift` and the sync vectors run against the pimdir checkout, none skipped.

## Open

A store written before this fix with an empty `date` keeps it until that item's summary is written again; no connector we know of hands one (neverest's dates come from parsed instants).
