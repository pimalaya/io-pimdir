---
cairn: log
change: the-crate-follows-draft-15
landed: 2026-11-20
---

# A base read at its own revision keeps the item conflict

`hub_property::hub_interleavings_converge_across_sources` found a seed on 2026-11-20 (pimdir the-freeze-plan, finding 1 of step b): "the divergence on lnk-1 was resolved by nobody". Its shrunk ops: an arrival on s0's server; s0 rebuilt, binding it at `Meta` (a base naming revision 0 and no body); s0 edited it twice, s1 being offered the first edit between the two; s1 edited it over the first, a divergence the hub recorded, its diverging body held. s0's upgrade then read its server's body at revision 0, the body s0's edits were made over: the upgrade lands an item conflict's binding body as its object (src/upgrade.rs, the path of SYNC §6 for an item conflict waiting for a body), and `PimdirHubItem::reconcile` read it as a fast-forward, the source having changed against a base naming none and the item not having changed for it. The item took the server body, the conflict cleared, and both s0's edit and s1's diverging body were lost, kept nowhere.

The standard decides it: SYNC §5 has a local body over a base naming a revision and no body wait for the upgrade, "which fetches that body into the base (§6), after which the local body is an edit over a body read", and SYNC §6's revisit of an item conflict by such a binding is for a conflict holding no `conflict_object`. An implementation departure, no spec gap.

- src/hub.rs `PimdirHubItem::reconcile`: an upsert whose base reads the body at the revision the binding's base named with no body, while the item's conflict holds its diverging body, settles nothing: the item keeps its body, its summary and its conflict, and the binding's base takes the body, `shared_object` unmoved.
- tests/engine/hub_property.rs `a_base_read_at_its_own_revision_keeps_the_item_conflict`, the shrunk ops; the seed kept in tests/proptest-regressions/hub_property.txt. Both failed before the fix ("resolved by nobody") and pass after; the property's other six tests pass.
- spec/REVISION names pimdir 4e1f96d, spec/generator re-vendored from it, the four other directories unchanged.

Capabilities moved: hub.
