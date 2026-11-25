---
cairn: log
change: the-engine-findings-close-for-the-freeze
landed: 2026-11-25
---

# The engine findings close for the freeze

The sync and storage findings standing before pimdir's freeze, against storage-draft-18 and sync-draft-16 (pimdir cairn/log/2026-11-25-the-engine-findings-close-for-the-freeze.md). spec/ carries pimdir's vectors 231 and 232 and its restated 114 and 115.

## What moved

- **The hub property's lost edit** (tests/engine/hub_property.rs): the case `cc 32b3574a...` failed "the edited item is still hubbed". s0 pushed `edit-153-msg-0`, its server deleted the member, and s1, unsynced, staged that same body; the quiesce dropped s0's placement (its body its base's) and s1 pushed the delete. The hub dumps after each op showed s1's binding at base `4os3` (the seeded body), `shared_object` `dr4j` (the shared one), and all three servers without the member after the quiesce. The engine follows pimdir SYNC §3, §5 and §9; the property's ledger cleared a pending server delete on any staging edit. It now leaves it standing when the edit restates the body the live item shares (`Owed::removed`). The seed line is kept in tests/proptest-regressions/hub_property.txt and the case is `an_edit_restating_the_shared_body_leaves_a_server_delete_standing`, which failed before the property's fix; pimdir vector 231 pins s1's run.
- **G12, a revive's kept body** (src/client/write.rs `revive_item`, spec/queries/storage/owner/retained_item.sql): the body a bodiless revive of a mutable kind sets aside took the reviving member's sort key; it keeps the retained row's, its body's own, beside the row's summary. pimdir vector 232 failed on 55d2702 ("items rows differ", `jane a. doe` for `jane doe`).
- **G20, `retained_by` of an unbound item** (src/client/producer.rs `apply_unbound` and the superseding add): a queued `remove` of an item no source binds retained it under the drain handle's source; it is left `NULL`, no source's removal having retired it (STORAGE §13). tests/unbound_items.rs `a_queued_action_on_an_item_no_source_binds_applies_or_parks` asserts it, failing before (`Some("phone")`).
- **The generator follows the text** (spec/generator): a body kept in a write draws its `seq` after every item the write's upserts name (STORAGE §9.1, `Model::number_kept`), G15's 225 differences gone; `apply_copy` parks a move or a copy out of a collection no source syncs or into one (STORAGE §15.3, G18) and lets a bodiless item through when its binding is based and its source syncs the target (SYNC §7, G19); the rekeys history lists its address book's renumbering without bodies, so its steps still stage an edit over a body unread, which the 2026-11-23 rule storing a carried body had made vacuous and left `a_rekey_pulls_a_server_edit_and_records_one_over_a_local_edit` and `an_upgrade_no_item_conflict_waits_on_only_moves_the_base` failing.
- **tests/generated**: replay.rs drops G12, G13, G15, G16 and G17 with the readings that named them and its ignored twin, every history replaying clean on all seeds; scale.rs drops G18, G20 and G21 and its ignored twin, the oracle accepting the automatic `sender` references STORAGE §14.2 lets an owner record (each extra one the rule's, and every one it states from that mail) or none.
- **CI** (.github/workflows/tests.yml) runs the generator's own tests.

## Left

- G29 (tests/generated/search.rs) waits on the owner: STORAGE §5, §11.4 and §14 disagree on whether a release lets `shared_object` go.
- A mutable revival whose member carries another body: this crate releases the old body as STORAGE §11.1 reads, the generator keeps it; for the owner.
- The generator's `query::vectors::tests::the_oracle_answers_the_search_vectors_alike_where_it_decides` fails on four cases of the search lane's vectors (`subject:""`, two `thread:` and `linked:sender:`), that lane's to settle.
