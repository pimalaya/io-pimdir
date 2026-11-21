---
cairn: log
change: an-item-keeps-local-state
landed: 2026-11-21
---

# Label and property in queries

The query half of an-item-keeps-local-state, left for the planner merge of as-fast-as-notmuch (its phase 2): `label:` and `property:` (pimdir SEARCH §8, §8.3, §8.4, §10; STORAGE §4.3) parse, plan and compile, and the 30 query cases and 7 refresh steps listed for them leave tests/pending_vectors.txt, which holds no case again.

## Rules and the vectors guarding each

- **Parsing** (src/search/query.rs `field`): `label` and `property` join `FIELDS` in the table's order; `PimdirQueryPredicate::Label(String)` and `Property { key, value }`, the key before the first `=`. A value is one the store may hold (`property::is_value`, `is_key`), checked after the field, an empty value, the reserved forms and the wildcard, so SEARCH §8.4's pass order holds; neither field takes a wildcard. Guard: the queries.json refusals `label:`, `label:""`, the two 342-character `label:` values (1,023 bytes accepted, 1,026 refused), `property:himalaya`, `property:=1`, `property:himalaya=1`, `property:Himalaya/muted=1`, the 65-byte key, `property:himalaya/muted=`, `label:rec*` (invalid-wildcard) and `label:~receipts`, `property:/x=1` (reserved-form).
- **A property is the item's** (src/client/search/compile.rs `property`): its condition over a hit is an `EXISTS` on `store.item_property` by the hit's seq, whatever its account, so one item in two accounts is two hits. Guard: `label:receipts`, `property:pimdir/label=Receipts` and `label:"tax returns"` answering `[home, 1]` and `[work, 1]`; `property:himalaya/muted=1` answering one hit for a message filed twice in one account.
- **`property:` byte for byte**: one seek on `item_property_by_value`. Guard: `property:pimdir/label=Receipts` and `=receipts` answering different items, the snooze value holding colons, the 64-byte key, `property:pimdir/session-key=x` answering nothing.
- **`label:` folded both sides** (src/client/search.rs `labels`): `list_property_values(pimdir/label)` read once per query naming a label, each spelling folded in Rust by `property::fold`, those matching bound as one list (`p.value IN (...)`); no SQL function. Guard: `label:receipts`, `label:RECEIPTS`, `label:" Receipts "` (the trim), `label:résumé` against `RÉSUMÉ`, `label:resume` answering nothing (no accent dropped), `label:tax` answering nothing (whole value), `label:family` (a label, never a category), and refresh.json's "an item keeps local state", whose respelling step reads the new spelling.
- **Planner sets** (src/search/plan.rs `estimate`): both terms estimate as `PimdirPlanEstimate::Property`, a count on `item_property_by_value` capped at 100,001, so a positive one may drive a query and a negated one is probed. Guard: every case above under the plan's own driver and each forced one (`Set`, `Walk`, `Scope`, `Fallback`), `label:receipts kind:mail` and `kind:contact -label:family`; src/search/plan.rs `a_set_is_a_positive_conjunct_a_client_reads_whole`.
- **No body read, store terms** (src/search/query.rs `reads`, src/search/card.rs `reads_store`, src/search/mail.rs `mail_terms`): a property term reads no body, so a bodiless item counts against it nowhere, and the in-memory matchers refuse it. Guard: the coverage every case checks against tests/search.rs `unread`; tests/search_cards.rs and tests/search_mails.rs skip the cases (they parse now and would otherwise be answered by no card or message).
- **Refresh**: a label added stamps every row of its item and the index answers it at once, since the term reads the store. Guard: refresh.json's 7 steps, refreshed and rebuilt.

`PimdirQueryPredicate::name()` names them `label` and `property` (src/search/query.rs `a_predicate_names_the_field_a_client_serves`).

tests/search.rs `the_local_state_refresh_case_holds_in_the_store`, the store-side stand-in while the queries waited, is removed: `every_refresh_vector_answers` now runs that case whole, its feed included.

## Proof

Before this change every case above was listed pending and failed (`unknown-field` for both fields). With the held spellings compared byte for byte in `labels` (seen 2026-11-21, restored by hand), `every_search_vector_answers` fails 8 cases (`label:receipts`, `label:RECEIPTS`, `label:" Receipts "`, `label:résumé`, `label:"tax returns"`, `label:family`, `label:receipts kind:mail`, `kind:contact -label:family`) and `every_refresh_vector_answers` steps 1 and 2.

## Breaking

`PimdirQueryPredicate::Label`, `PimdirQueryPredicate::Property` and `PimdirPlanEstimate::Property` are new variants an exhaustive `match` names; none of the three enums is `#[non_exhaustive]`.

## Chosen here

- The label's spellings are read with the canonical `list_property_values` on the index connection, its unqualified `item_property` resolving to the attached store; the set is the crate's own statement with a list slot, the reading `list_value_holders` gives per spelling, rather than one embedded `list_value_holders` per spelling joined by `UNION`.
- A label term whose fold no held spelling has compiles to no set (`0`), as a text term holding no word does.
