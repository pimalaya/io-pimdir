---
cairn: log
change: two-members-of-one-id-share-no-body
landed: 2026-11-24
---

# Two members of one ID share no body

neverest tests/moves.rs `a_move_into_a_message_stating_its_id_keeps_both`, variant `listed = true`, failed on Stalwart with io-pimdir 0.20.2: the item keyed by the plain `Message-ID` in the target (B, bound to B's UID) held A's body (`MOVEDMARKER` where the server serves `OTHERMARKER`, tests/moves.rs:321). Follows the owner's ruling of 2026-11-24 on cairn/changes/a-same-id-message-keeps-its-body, option A: correct on every server, one download per such duplicate; `EMAILID` stays out of scope.

## Root cause

Not the relocation landing, the mint nor the listing's keying, which all worked: a scratch replay of the test's first two steps (A synced in Left, B appended to Right, one more sync) logged, under `RUST_LOG=io_pimdir=debug`, `look up 1 link ids in object store`, `linked 1 bodies from store, no fetch`, `upgraded 1 items (0 fetched, 1 linked from store)` for Right's B, before any move. That is src/upgrade.rs `State::LookingUp` linking from Left's A by `agrees` (src/upgrade.rs, src/object.rs `PimdirHeldObject::agrees`): same subject, date, sender and size, no server object id stated, so SYNC §6 linked by agreement, as written. The move then staged A's create in Right under `dup:<id>#\u{1}<id>` with its own body and the `COPYUID` landed it correctly; B kept A's body. The agreement rule alone needed the guard: no plain bug picked a wrong row.

## What landed

- src/upgrade.rs: `PimdirUpgrade::crowded`, whether the collection holds another member under a key minted from the placement's bare hint; in `LookingUp` such a placement links only from a holder stating its own server object id (`same_content`), else fetches.
- src/client/write.rs: a `Handles` load of a mail collection also reads every member under a key minted from a non-derived key it reads (`minted_siblings`, new statement spec/queries/storage/owner/list_minted_links.sql, one range seek per key on the `(collection, link_id)` index), so an upgrade naming the bare member alone sees the other. The write reads the bare hints of the minted keys a batch upserts and every key minted from them (`minted_hints`), and after the absorb releases a shared body (`release_shared`, reading `collection_sources`).
- src/hub.rs: `PimdirHub::release_shared`, with `PimdirHubItem::releases` (STORAGE §11.4's "needs its body", whole) and `same_content`, and `PimdirHub::shared_before` (a sharing another minted member held before the batch, which a fetch confirmed: vector 215 keeps its rows).

## Guards

- tests/engine/membership.rs `a_move_beside_a_message_of_its_id_linked_before_keeps_both_bodies`, `a_copy_beside_a_message_of_its_id_linked_before_keeps_both_bodies`, `a_listing_bringing_two_messages_of_one_id_links_neither`: all three failed on a `git archive` snapshot of 896d4a6 (`archive/a1 holds the body of the member it is bound to`, the other message's hash where the served body's is expected) and pass now.
- vectors/sync/222 (the upgrade fetches) and 223 (the copy's write releases): both failed on the snapshot (`bindings rows differ`, `base_object` b1 for t1, m1 for null).
- neverest tests/moves.rs whole (`--ignored`, Stalwart, io-pimdir patched by path): 3 passed, both variants of the case above among them.

## Spec

pimdir SYNC §6 (*Two members stating one hint in a collection share no body by agreement*, sync-draft-15 edited in place) and STORAGE §9, §11.4, §14 (the `Handles` load, *A body two members stating one hint would share*; storage-draft-17 edited in place), vendored with vectors 222 and 223 and the manifest. spec/REVISION still names dced98c: tests/spec_drift.rs `the_vendored_copy_is_the_spec_byte_for_byte` fails on the three vendored files until the pimdir commit exists and REVISION names it.

[upgrade](../spec/upgrade.md): new *Two members stating one hint in a collection share no body by agreement*. [seam](../spec/seam.md): the `Handles` load's minted members, new *A write releases a body two members stating one hint would share*.

## Left

- A body B linked across collections before its collection held another member stating its id, and differing from that member's, stays: no member of the collection shares it, and nothing a listing states tells it from the message it was linked from (the cross-collection risk SYNC §6 accepts until a server object id is stated).
- A bare member a source of its collection binds not (the lacking mark) keeps a shared body, as a release keeps it, so a two-source pair may propagate it before either fetches it again.
- Android's store implements the load and the write through its own host: it needs `list_minted_links` in its `Handles` load and the release in its write to follow.
