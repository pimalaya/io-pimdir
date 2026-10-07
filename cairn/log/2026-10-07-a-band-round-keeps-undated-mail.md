---
cairn: log
change: a-band-round-keeps-undated-mail
date: 2026-10-07
---

# A band round infers no delete of an undated member

Reported by the neverest implementer (neverest 3938d10): a band round, the widening a connector bound to no scope lists instead of the whole scope, dropped every bound undated mail. A mail with no usable `date` is in every scope (pimdir SYNC §5), and the round's last page found absent every unstamped based binding whose `date` is in its scope or unknown, while the band's listing, a date filter (`SENTSINCE`, Gmail's `after:`, JMAP's `after`), never returns undated mail. Nothing was pushed, but the mail left the store until a round over a whole scope relisted it. neverest relisted undated mail itself on a band round's last page; the Android bridge had no workaround.

Implements pimdir 00535c1 (draft-04, untagged), spec/ re-vendored.

- `PimdirWriteOp::OpenRound` and `PimdirRound` carry `band`; `open_round` records it in `sources.round_band`, `close_round` clears it, `load_round` answers it.
- The round its last page opens finds absent what `RoundPlan::finds_absent` says: dated in its scope, or undated when it is no band round; a resumed round reads `list_unstamped_bindings`, which applies the same rule to the store's stamps.
- `choose` resumes an open round only when its kind is the one the run would list, so an open round of a store reconciled from an earlier build, read as no band round, restarts as a band round.
- Open adds `round_band` to an older store with the other round columns.
- Capabilities moved: sync (a run chooses its listing; absence means deleted in scope only), store (coverage and rounds per source).

## Verification

`tests/scoped_sync.rs` `a_band_round_keeps_an_undated_member_it_does_not_list`: no `Date`, an empty `date` and no summary, each through a band round opened and closed in one run and one resumed from its cursor; the run-opened variants fail with the old in-memory filter, the resumed ones with the old statement. Unit tests `a_band_round_finds_no_undated_member_absent`, `a_whole_scope_round_finds_an_undated_member_absent` (the counterpart) and `an_open_round_of_the_other_kind_restarts`; the reconcile test checks the column. Sync vectors 41 (one page), 46 (two pages, failing with the old statement) and 47 reproduce. `cargo test --all-features`, `--no-default-features --features cli`, `--no-default-features --lib`, clippy `-D warnings` and `cargo fmt --check` green; `spec_drift` and the sync vectors run against the pimdir checkout, none skipped.
