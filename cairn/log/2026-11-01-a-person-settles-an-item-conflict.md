---
cairn: log
change: a-person-settles-an-item-conflict
date: 2026-11-01
---

# A person settles an item conflict

Found by the Android app's port, whose bridge stood in for it (android/rust/src/offline/merge.rs `settle`): pimdir SYNC §7 says a person's `Edit` on a conflicted item resolves it, `items.conflicted` cleared and its `conflict_object` released, but `PimdirHub::reconcile_content` cleared an item conflict only when the edit was a body one source alone changed. Two sources diverged under `manual` (the item holding P, its `conflict_object` S, the server binding's `shared_object` the ancestor B), a person's `Edit` without `overrule` settled two cells of six:

| Edit through | Keep P | Take S | Merged X |
|---|---|---|---|
| the source agreeing with the item | stayed conflicted | settled | settled |
| the source the item moved away from | stayed conflicted | stayed conflicted, the item holding P | stayed conflicted, the item holding P |

The three facts read an edit restating the item's body as no change, and one through the source the item moved away from (whose `shared_object` stays B) as either no settle or a divergence again. The hub cannot tell a person's edit from a sync's upsert by the row: an engine's `PreferRemote` pull and an upgrade's settle of a binding conflict also upsert a non-`Conflict` status over a conflicted item, and those must leave the item's conflict standing.

So the mutate states it, as an overruling edit states `Overrule`: a new `PimdirWriteOp::Settle { collection, handle }`, staged before the upsert by an `Edit` without `overrule` whose placement projects `Conflict` or `Tombstone` (the only statuses an item conflict projects for a bound placement; the mutate does not see the item). The hub records the handle and, when that handle's next upsert meets a conflicted item, adopts its body, clears `conflicted` and `conflict_object` and records no divergence and no loser, the binding's `shared_object` moving to the body; the write routes the op to the hub as it routes `Overrule`, and the refcount diff releases the item's reference to S. An item in no conflict ignores the op, so an ordinary edit folds by the three facts exactly as before (vector 28, 103 and the rest unchanged). The overruling side is unchanged too: an overruling edit settles an item conflict only by restating the shared body or through the content rule, as vectors 102 and the engine's `PreferLocal` keep rely on.

Trivial fix, landed without a proposal. **Breaking:** the new `PimdirWriteOp` variant. Moved: [mutate](../spec/mutate.md) (new requirement "A person's edit settles the item's conflict"), [hub](../spec/hub.md) (the settling upserts, a scenario). Tests: tests/item_conflict_settle.rs, six cells through `PimdirStore`, four failing first (keep P through either source on `!item.conflicted`; take S and merged X through the server on the item's body); src/hub.rs `a_settle_resolves_the_item_conflict_by_its_own_upsert_alone`; src/mutate.rs `a_persons_edit_of_a_conflict_states_the_settle`. Six mutate unit tests over a `Conflict` or `Tombstone` placement read the upsert at `ops[2]` instead of `ops[1]`, the settle now preceding it; what they assert is unchanged.

## For pimdir: a vector of a person resolving an item conflict

pimdir has no sync vector for a person's `Edit` on an item conflict: 102 is the overruling side, 103 a binding conflict. A draft for pimdir to adopt, vector 102's store with no `overrule`, the person keeping the item's body through the source the item moved away from (the cell io-pimdir and the Android bridge both got wrong):

```json
{
  "v": 2,
  "label": "a person's edit settles an item conflict whatever body it keeps, keeping nothing it discards",
  "note": "the store of 102 with no overrule: a pulled c3 into the item, then b's own edit c2 diverged under manual. A person keeps c3 through b, the source the item moved away from: the item's conflict clears and c2 is released from the item, kept by nobody and reported by nothing (SYNC.md §7). b's base stays c2 at s2, so b owes c3, and its shared_object moves to c3; a's base already holds c3",
  "store": {
    "collections": [{ "id": "Contacts", "account": null, "kind": "text/vcard", "conflict": "manual" }],
    "objects": { "c1": { "body": "fixtures/card-basic.vcf" }, "c2": { "body": "fixtures/card-basic-edited.vcf" }, "c3": { "body": "fixtures/card-basic-renamed.vcf" } },
    "items": [{ "collection": "Contacts", "link_id": "urn:uuid:4fbe8971-0bc3-4e1f-8a2d-1c9b7f6e5d40", "seq": 1, "flags": [], "object": "c3", "level": 2, "deleted": 0, "conflicted": 1, "conflict_object": "c2" }],
    "bindings": [
      { "collection": "Contacts", "link_id": "urn:uuid:4fbe8971-0bc3-4e1f-8a2d-1c9b7f6e5d40", "source": "a", "handle": "a.vcf", "base_flags": [], "base_object": "c3", "base_revision": "r2", "base_present": 1, "shared_object": "c3" },
      { "collection": "Contacts", "link_id": "urn:uuid:4fbe8971-0bc3-4e1f-8a2d-1c9b7f6e5d40", "source": "b", "handle": "b.vcf", "base_flags": [], "base_object": "c2", "base_revision": "s2", "base_present": 1, "shared_object": "c1" }
    ],
    "sources": [
      { "collection": "Contacts", "source": "a", "checkpoint": "a1" },
      { "collection": "Contacts", "source": "b", "checkpoint": "b1" }
    ]
  },
  "run": { "verb": "mutate", "collection": "Contacts", "source": "b", "mutation": { "kind": "Edit", "seq": 1, "object": "c3", "if_match": "c3" } },
  "remote": { "snapshot": null, "fetch": {} },
  "expect": {
    "pushes": [],
    "outcomes": [],
    "events": [],
    "store": {
      "items": [{ "collection": "Contacts", "link_id": "urn:uuid:4fbe8971-0bc3-4e1f-8a2d-1c9b7f6e5d40", "seq": 1, "flags": [], "object": "c3", "level": 2, "deleted": 0, "conflicted": 0, "conflict_object": null }],
      "bindings": [
        { "collection": "Contacts", "link_id": "urn:uuid:4fbe8971-0bc3-4e1f-8a2d-1c9b7f6e5d40", "source": "a", "handle": "a.vcf", "base_flags": [], "base_object": "c3", "base_revision": "r2", "base_present": 1, "shared_object": "c3" },
        { "collection": "Contacts", "link_id": "urn:uuid:4fbe8971-0bc3-4e1f-8a2d-1c9b7f6e5d40", "source": "b", "handle": "b.vcf", "base_flags": [], "base_object": "c2", "base_revision": "s2", "base_present": 1, "shared_object": "c3" }
      ],
      "sources": [
        { "collection": "Contacts", "source": "a", "checkpoint": "a1" },
        { "collection": "Contacts", "source": "b", "checkpoint": "b1" }
      ]
    }
  }
}
```

The other rows the vector set could carry, on the same store and run but for the mutation:

| Source | `object` (`if_match` c3) | Expected item | Expected `shared_object` of the source | Owes after |
|---|---|---|---|---|
| b | c2 (take the diverging body) | c2, `conflicted` 0, `conflict_object` null | b: c2 | a (base c3) |
| a | c3 (keep the item's body) | c3, `conflicted` 0, `conflict_object` null | a: c3 | b (base c2) |
| a | c2 | c2, `conflicted` 0, `conflict_object` null | a: c2 | a (base c3) |

Every row reports no event and retains no item; b's base stays c2 at s2 and a's c3 at r2. A merged body would need a fourth fixture card.
