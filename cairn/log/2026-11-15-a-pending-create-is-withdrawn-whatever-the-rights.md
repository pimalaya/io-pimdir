---
cairn: log
change: a-pending-create-is-withdrawn-whatever-the-rights
landed: 2026-11-15
---

# A pending create is withdrawn whatever the rights

Follows pimdir's change of the same id, in the working tree of both repositories, uncommitted. The gate asked a source for its remove or its move to withdraw a create its server never took: live, a message added to a Graph folder other than Drafts, rejected by every sync, could not be deleted while deletes were off. `load_item_capabilities` now answers `held`, and `capability::holding` leaves out a source binding the item as a pending create alone for a `remove`, a `move` and a `copy`; a move or a copy asks it, when it syncs the target, an `add`'s capabilities there. A `set-flags` and an `update` still read every source binding the item, `at_item` and `PimdirReader::item_capabilities` unchanged.

## Checks

- tests/capabilities.rs `a_pending_create_is_withdrawn_whatever_its_source_may_delete`: failed on the old gate with `Unsupported(PimdirRefusal { capability: "mail.message.remove", source: "graph", detail: Some("pull-only") })`, passes now; it also holds that a remove and a move of a message the source holds are still refused, and that the drain applies the move, parking nothing.
- Capabilities moved: store ("A pending create is withdrawn whatever its source may delete", added).
