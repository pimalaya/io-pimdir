---
cairn: log
change: scoped-mail-sync
date: 2026-10-07
---

# A mail sync lists newest first, within a scope on the `Date`

pimdir's `scoped-mail-sync` (`a2e92af`, still draft-04) removes probes, pages rounds, bounds a mail sync by a scope on the summary `date`, records the coverage each source's last closed round left, and adds the owner's collection below a date and the mail readers. This crate re-vendors spec/ and implements it, for neverest and the Android bridge to code their connectors against.

## What landed

- **Vendored spec** (capability `spec-fidelity`): 0001_init.sql with the coverage and round columns, `bindings.round` and `sources_stamp_coverage`, `probes` gone; the new owner and read statements, the probe ones removed. tests/spec_drift.rs holds the copy.
- **The seam** (`seam`): `PimdirEnumerate` and `PimdirListing` in, `PimdirEnumerated` out; `PimdirRemoteItem::meta`; `PimdirRemoteSnapshot` as a page; `PimdirRemote::scope_bound`; the round write ops; `PimdirScope`, `PimdirCoverage`, `PimdirRound`, `PimdirCursor`; unnamed upserts of unbound handles refused.
- **The sync** (`sync`): the listing chosen from coverage and round, band rounds for connectors bound to no scope, pages named, merged, stamped and landed with their cursor, deletes at the last page in scope only, creates held by open rounds and out-of-scope dates, pulls taking the member's meta and carried body, a rejected cursor restarting the round; probes and `Probed` gone.
- **Upgrade and rekey** (`upgrade`, `rekey`): `Meta` revisits unheld claims only and keeps a walked mark; the rekey lists every page and names from the meta, fetching nothing.
- **The store** (`store`): load answers coverage, round and unstamped bindings; writes apply the round ops, stamps after upserts, public ids in batch order; the reconcile of an earlier draft's store; level 0 read as an unheld claim; `collect_before`; coverage on collections and `list_coverage`; the mail counts, pages and search. `pimdir collection list` shows the coverage (`cli`).
- **Annex A.1** (`summaries`): `derive_meta`, `meta_attachment`, `without_body`.
- **Tests**: tests/vectors_sync.rs seeds summaries, rounds and coverage, compares them, scripts pages, interruptions, rejected cursors and unbound connectors, and runs all 45 vectors green; tests/summaries.rs checks `meta_attachment`; tests/scoped_sync.rs covers the store side (interrupted rounds, narrowest coverage, scope refusal, collection, readers, the reconcile of an earlier draft's store); the engine suites moved off probes.

## Resolved in the implementation

- **Vector 41's event order.** It lists `Vanished 9` before `Added 10`, against the handle order every other case keeps (vector 36 lists `Added 13` before `Vanished 8` in the same shape). The harness compares events as a multiset; every other vector also holds in order.
- **Public ids.** Vectors 34 and 37 number a page's new items in handle order, so a write draws ids in the order its batch upserts them rather than in link id order.
- **A page answering another listing.** A round page answering a delta request opens a round over the scope; a delta page answering a round request merges as a delta and lands its checkpoint. A rejected delta checkpoint opens a round.
- **Members always carry meta.** `PimdirRemoteItem::meta` is required: a delta listing a changed known member carries its meta too, the engine refreshing a summary only when it moved.

## Verification

`cargo test --all-features`, `--no-default-features --features cli` and `--no-default-features --lib` green; `cargo clippy --all-features --all-targets -- -D warnings` and `cargo fmt --check` clean.

Capabilities moved: `sync`, `seam`, `upgrade`, `rekey`, `store`, `hub`, `mutate`, `summaries`, `cli`, `spec-fidelity`.
