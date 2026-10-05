---
cairn: log
change: hub-property-agreement-points
date: 2026-10-05
---

# The hub property reads agreement points as the engine keeps them

CI failed `hub_interleavings_converge_across_sources` on 2026-10-05 on cases earlier seeds never drew. The engine was right in each; the test's ledger was not.

- **A pass that pulls leaves its pushes to the next.** The ledger counted every sync as the source agreeing with every live item, though a pending create the pass neither pushed nor rewrote never saw the edit another source made meanwhile, and its next edit is a divergence (SYNC §9). Now a sync moves no agreement point for such a create; one the pass resurrected holds the shared body and does agree. Guarded by `a_pending_create_a_pass_did_not_push_has_not_seen_the_shared_body` and `a_pending_create_a_pass_resurrected_has_seen_the_shared_body`.
- **No revision, no divergence.** SYNC §9 reads content as mutable from a revision on some base. An item every source holds as a pending create (one never pushed, another resurrected after its server deleted it) shows none, so an edit fast-forwards it. The ledger expected a divergence; it now expects one only when a revision is visible. Guarded by `an_item_held_only_as_pending_creates_fast_forwards`.

## Open

That second case loses an edit: s2's body is replaced by s0's without a conflict, because the hub cannot tell the content mutable once every base is gone. Knowing mutability from the collection's declared kind rather than from revisions would close it; that is a SYNC §9 change, left to the spec.

## Verification

`cargo test --all-features --test engine hub_property` green, nine default runs and one of 2000 cases.
