---
cairn: log
change: the-crate-follows-draft-15
landed: 2026-11-20
---

# A settled late body brings its summary

The generated replay found it on 2026-11-20 once spec/generator was re-vendored from pimdir ce1d32b: `sources` step 47 ("work/unread-incoming: b's upgrade reads them"), every seed. Source b joins a card it lists without its body, an item conflict recorded for want of it; b's upgrade fetches it and the collection's `prefer-incoming` adopts it. The item took the fetched body (hash agreed with the model) but kept the sort key, summary and address rows of the body it held, so the contact listed, sorted and searched under its old name. src/upgrade.rs landed the late body on the binding without the summary and sort key the fetch derived, and the hub's write takes those of the upsert it adopts.

The standard decides it: STORAGE §9.3 writes the sort key from the body Annex A derives, never from another. An implementation departure, no spec gap.

- src/upgrade.rs `write_fetched`: the item-conflict path restates the binding's summary and sort key from the fetched derivation; the hub keeps the held ones when the item keeps its body (`manual`, `prefer-existing`).
- tests/engine/upgrade_summary.rs: `an_upgrade_adopting_the_incoming_body_restates_its_summary` failed before the fix (sort key "ines simon" where "their copy" was due) and passes after; `an_upgrade_keeping_the_items_body_keeps_its_summary` guards `manual` and `prefer-existing`.
- tests/generated/replay.rs needs no finding for it (G22 was named for it, then lifted before landing).

Capabilities moved: upgrade.
