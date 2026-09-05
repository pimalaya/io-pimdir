---
cairn: tasks
change: revised-standard-alignment
---

- [x] spec/: re-vendored; `collection_sources` upstreamed the same day.
- [x] `sync.rs`: `KeepBoth` and the delete policy gone; `beside_other_sources`; a vanished edit re-staged; a create waits for probes; the item-level conflict held; a tombstone with a staged edit follows the policy.
- [x] `hub.rs`: `Conflict` for every binding of a conflicted item; a tombstone clears it; mutability across bindings; the immutable base adopts the shared body; the `rebinding` stash.
- [x] `mutate.rs`: provisional handles, `Undeliverable`, a `Remove` settles a conflict, the target read asks for the minted key.
- [x] `upgrade.rs`: the base moves on a fetch; a conflict body equal to the placement's own clears; a restated hint is keyed afresh; the size witness; `LookupObject` carries `PimdirObject`.
- [x] `rekey.rs`: drops before upserts; pairing by base revision then body; an item-level conflict carried with no revision.
- [x] `client/`: store-wide `drain` with `MAX_ATTEMPTS`; `PIN_OBJECT` on enqueue; the collector recomputes and refuses in flight; `rename_collection` retargets the queue; `delete_collection`; `list_item_conflicts`; `list_pending_actions`; the trash of every deleted row; `Stale { missing }` over tables and triggers.
- [x] Suites: thirty-two vectors, the engine properties, the queue, retention, refcount and stale-store suites on the new rules.
- [x] sync.md, hub.md, mutate.md, upgrade.md, rekey.md, seam.md, store.md, cli.md, spec-fidelity.md folded; the log entry written.
