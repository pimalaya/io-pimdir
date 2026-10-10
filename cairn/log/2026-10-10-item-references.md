---
cairn: log
change: item-references
date: 2026-10-10
---

# Items refer to each other

Implements pimdir's item-references (draft-04, untagged), spec/ re-vendored: the `item_reference` table folded into 0001, `item_reference_to`, the `items_drop_references` trigger, and the statements `add_reference`, `remove_reference`, `references_from`, `references_to`.

- The `reference` module, no_std: `PimdirEndpoint { kind, link_id }`, `PimdirReferenceRole` (the four names pimdir gives, and `Application` for an `x-` one, parsed infallibly), `PimdirReferenceOrigin` (`Auto`, `User`) and `PimdirReference`.
- `PimdirStore::add_reference` and `remove_reference` answer the row the statement returned, `None` when it changed nothing; `PimdirReader::references_from` and `references_to` read none on a store lacking the table.
- Open reconciles the table among `RECONCILED`, and the index and the trigger through `RECONCILED_OBJECTS`, which replaces `RECONCILED_INDEXES` and cuts a trigger up to its `END;`.
- Capability moved: store (references, reconcile on open).

## Verification

tests/references.rs: recorded once with a person's taking over a rule's, read from either end, an `x-` role accepted and a bare name refused, a self-reference refused, an end held under no row or another kind recording nothing, removal once; a reference kept while Archive still holds the message and while its last row is retained, gone with its purge and with the delete of the collection holding the other end; a store without the table read as none, then reconciled by its owner's open. The trigger's own cases are pimdir's invariants.sh. `cargo test --all-features`, `--no-default-features --features cli`, clippy `-D warnings`, `cargo doc` and `cargo fmt --check` green; `spec_drift` against the pimdir checkout.
