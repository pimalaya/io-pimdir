---
cairn: log
change: message-scoped-seq
date: 2026-08-02
---

# The public id is scoped to the message, not the mailbox

Correction to `public-seq-id` (same day, before shipping): `seq` was per-collection
(numbers restarting per mailbox), which clashes with dedup and a merged view — the
same message got two ids across mailboxes, and id `1` was ambiguous across
mailboxes.

Now `seq` is message-scoped and store-global: `store_meta.next_seq` (replacing
`collections.next_seq`) is the single counter; `insert_item` reuses a `link_id`'s
existing `seq` (`SEQ_FOR_LINK_ANY`, indexed by new `items_by_link`) before
allocating a fresh global one, so all placements of a message share one id. The
store's single-writer serialization makes the reuse-lookup race-free (a concurrent
insert of the same message in another mailbox sees the committed id). `(collection,
seq)` stays unique. Never reused (global counter only rises).

Test rewritten (`public_seq_is_message_scoped_global_and_never_reused`): a message
in INBOX and Archives shares one id; a distinct message takes the next global id;
`seq_for_link` agrees from either collection; a dropped id is not reused (11 green).
Live (neverest → Stalwart): `mid:ms2` filed in INBOX+Archive has seq 1 in both;
himalaya shows id 1 in both mailboxes and `read 1 -m Archive` reads it.

Spec updated: `store` (MODIFIED "Items carry a … public id" → message-scoped,
store-global).
