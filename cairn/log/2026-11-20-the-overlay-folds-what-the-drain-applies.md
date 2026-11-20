---
cairn: log
change: the-crate-follows-draft-15
landed: 2026-11-20
---

# The overlay folds what the drain applies

pimdir the-freeze-plan, finding 2 of its section 4 review, and the generated suite's G2 and G3 (tests/generated/overlay.rs): `PimdirReader::with_pending` showed rows the owner parks, and a folded update kept the committed summary and sort key. pimdir draft-15 restates STORAGE §15.4 (pimdir cairn/log/2026-11-20-what-an-overlay-shows.md).

- src/client/reader.rs `fold` and `PimdirStanding`: an item no source binds folds as the owner settles it (STORAGE §15.3): where a source syncs its collection a `set-flags` and an `update` fold into nothing, a `move` parks anywhere, and a `move` or `copy` of such an item brings nothing into its target. A `move` into a collection of no declared kind folds into nothing.
- `admits` and `admitted` run the producer's gate over every pending row the overlay folds, the owner's capability backstop (STORAGE §15.6) parking what a source does not support, in the collection read and in the one an arrival leaves.
- `summarize`: a folded `update` carries the summary and sort key Annex A derives from its new body, read from the blob directory, as the owner applying it derives them.
- `overlaid`: an item with a pending `update` is read apart from the committed page (`committed_item`) and kept where its folded key falls past the cursor, so a keyset walk lists it once; `list_summaries` joins addresses onto the committed page alone, a restated summary carrying its own. Both faults appeared on the way and the generated suite caught them ("work/contacts: seq 40 listed twice"; two emails where `get_item` gave one).
- tests/overlay.rs `an_item_no_source_binds_folds_as_the_owner_settles_it` and `a_move_into_an_undeclared_collection_folds_into_nothing` fail on f33c00d's reader ("a parked move leaves it"; the item folded out) and pass. tests/generated/overlay.rs: G2 and G3 lifted, a difference of either kind now failing the test; G4 and G5, the pending add and the arrival, wait on the generator following draft-15's §15.4.
- spec/queries and spec/vectors re-vendored from the pimdir working tree (`load_pending_arrivals`, sync vectors 178 to 180).

Capabilities moved: store.
