---
cairn: delta
change: engine-merge
---

## ADDED Requirements

The capability files coroutine, seam, sync, upgrade, mutate, rekey and hub, folded from io-replica with the four rule changes the pimdir spec settled; the summaries capability replacing conventions; in store: summaries and addresses are written with the item, probes are rows, the change feed is trigger-maintained, a rekeyed batch bumps the generation, a store from an earlier draft is refused.

## MODIFIED Requirements

Retention: the hub keeps an unbound item and the store retains it. Events: pull-side only. Drop reasons: `Rekeyed` beside `Superseded`. The write reads summaries and addresses by link. The queue's `add` and `update` derive the summary from the body.

## REMOVED Requirements

The draft reconcile and the unreconcilable-store refusal; the in-memory residual; `write_rekeyed`; the raw `meta`.
