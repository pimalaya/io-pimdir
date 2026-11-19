---
cairn: log
change: the-merge-verdict-is-public
landed: 2026-11-19
---

# The merge verdict is public

On the owner's ruling of 2026-11-19, the cross-source merge of one upsert is public: `PimdirHubItem::reconcile` (the former private `PimdirHub::reconcile_content`, its body moved unchanged) answers a `PimdirHubVerdict` with `diverged`, `kept_shared`, `lost` (a `PimdirHubLoser`, its object and its `PimdirHubLoserSide`) and `unsummarized`. `absorb` calls it, so the Android app, which restated these rules in rust/src/offline/merge.rs for its A8 fix, can read the same verdict from its own copy of the item. Additive; no behaviour moves.

Capabilities moved: hub ("The merge verdict is public").

Tests: tests/hub_reconcile.rs (three tests over the public API); the hub's unit tests and the sync vectors pass unchanged, which is the check that the move changed nothing. A new public surface, so no test fails against the old code but by not compiling.
