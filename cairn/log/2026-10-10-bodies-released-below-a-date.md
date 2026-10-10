---
cairn: log
change: bodies-released-below-a-date
date: 2026-10-10
---

# Bodies are released below a date, and a held body keeps its size

Implements pimdir's bodies-released-below-a-date (draft-04, untagged), spec/ re-vendored.

- `PimdirStore::release_before(collections, until)` runs `release_bases_before`, `release_before` and `recompute_refcounts` in one immediate transaction, refuses while a verb is in flight (`PimdirError::InFlight`), and answers `PimdirReleaseReport { seqs }`. `reader::collections_json` is now crate-visible for it.
- `summary::without_body` keeps the held `size` beside the attachment mark over a placement holding its body, so a listing's stated size (Graph's `PidTagMessageSize`, Gmail's `sizeEstimate`) no longer overwrites the body's octets.
- tests/vectors_sync.rs reads a member meta's stated `size`.
- Capabilities moved: store (the release), summaries (the size rule).

## Verification

Sync vector 52 failed before the `without_body` change (the held 299 overwritten by 312) and passes after; 53 and 54 pass. `tests/scoped_sync.rs` `releasing_bodies_below_a_date_keeps_the_headers`: bodies landed `Full` by a listing, the old and the undated released and the newer kept, flags and summary kept, two objects collected, and a round relisting them fetches nothing and pushes only the owed flag. The skip rules are pimdir's invariants.sh. `cargo test --all-features`, `--no-default-features --features cli`, clippy `-D warnings` and `cargo fmt --check` green; `spec_drift` against the pimdir checkout.
