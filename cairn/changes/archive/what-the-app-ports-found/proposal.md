---
cairn: change
id: what-the-app-ports-found
status: landed
created: 2026-10-30
---

# What the app ports found

## Why

Porting himalaya, cardamum, calendula, unfog and neverest onto the 2026-10-30 API (phase A of the pimdir stable-release plan) hit five places where the library leaves the apps to work around it or loses a row:

1. `PimdirProducer::enqueue_with_body` runs the capability gate and drops its partial supports, so cardamum, calendula and unfog run `check` a second time, after (or before) the write, to warn about them.
2. `PimdirReader::open` and `PimdirProducer::open` read a database at `user_version` 0 holding tables as `store-uncreated`; only the owner's open says `store-foreign` (STORAGE §3).
3. A queued action on an item no source binds (a card or event written with `put_item`, kept on the device) is reported `skipped` by every drain and stays pending forever: STORAGE §15.2 has three outcomes, and skipping is reserved for a kind, a payload version or an intent the owner does not run.
4. The drain ignores what the write recorded beyond its ops (`PimdirWritten`), so a body a collection's conflict policy overruled during a drain, kept as a retained item, is reported nowhere.
5. `PimdirBlobs::reader` hands out the file unverified, while STORAGE §7 has a streamed read verify at its end; and the overlay readers fold a queued `update` whose `if_match` the drain will park, showing an edit that will never apply.

## What

1. `PimdirProducer::enqueue_with_body_checked`, the same step answering `PimdirEnqueued { id, partial }`, the gate's partial supports read with the body in hand; `enqueue_with_body` keeps its signature. Migration: an app replaces its `check` plus `enqueue_with_body` pair by the one call.
2. The reader's and the producer's open refuse a `user_version` 0 database declaring tables as `PimdirError::Foreign`.
3. The drain never skips an action on an unbound item. A `remove` retains it (a delete nobody pushes, as a superseded unbound item is). In a collection no source syncs, an `update` repoints the body as `put_item` writes one (flags kept, summary re-derived; a body deriving another identity parks), a `set-flags` sets the flags. Anything else, and anything on an unbound item of a synced collection, parks naming the item. pimdir STORAGE §15.3 is silent on it: reported as a gap.
4. `PimdirDrainReport::overruled`, one `PimdirDrainOverruled { id, collection, link_id }` per body a policy kept as a retained item while applying row `id`. The report loses `Copy`.
5. `PimdirBlobs::reader` answers a `PimdirBlobReader` that hashes what it streams and fails the read at its end (`InvalidData`) when the bytes do not hash to the name, with `metadata` and `len` for the uploads that size the body first. The overlay folds an `update` only when its `if_match` names the body the item holds at that point of the fold, as the drain will judge it.
