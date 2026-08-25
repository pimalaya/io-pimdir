---
cairn: log
change: own-the-kind-conventions
date: 2026-08-25
---

# The crate owns the per-kind derivations

Annex A fixes what a `meta` blob holds and what a `sort_key` means per kind, and it is informative: nothing enforces it, and nothing reports a disagreement. Three consumers implemented it separately, and by the time this was proposed the calendar kind had diverged outright — calendula wrote the whole Annex A.3 summary and a resolved key, the Android app wrote `{"v":1,"etag":…}` and an empty key for every item, so calendula reading a phone-written calendar had no summary to render a row from and no ordering at all. The crate they all depend on now holds one implementation of it.

## What landed

- **`conventions::derive(kind, body)`** in the I/O-free core, with one module per kind, returning `PimdirDerivation { link_id, meta, sort_key }` and `None` for a media type it has no conventions for. It reaches no store handle: it is a library a writer calls before the bytes reach the store, exactly as `hash` is one for naming them, so §13's "the store never parses `meta`" is untouched.

- **The fallback ids are fixed here.** The format leaves them open on purpose (§16 tells a consumer to check that an id was derived, never that it equals a value), which is precisely why they had to be closed somewhere: two writers disagreeing about the id of a message with no `Message-ID` link it twice and store one body twice, which is the object-hash bug on the identity axis. Mail keys on `alt:{subject}|{date}|{from}` rather than on the body's hash, because the same message read at `Meta` and at `Full` is two byte strings and one item; a card or a resource with no `UID` keys on `hash:{fnv1a64}` over the body, which is all it has.

- **Small scanners, not parsers.** Each kind is a shallow read of a handful of properties over unfolded lines. The core is `no_std`, no mail parser in this org is, and a body crosses this crate byte for byte, so nothing here can rewrite one. The zone resolution reads the `VTIMEZONE` the resource itself carries and nothing else: this crate holds no zone database and reads no clock.

- **One rule for both transition cases.** Annex A states the ambiguous hour (take the offset in effect *before*) and the nonexistent one (take the offset in effect *after*) as two rules. They are the same offset — the numerically greater one, which is the earlier instant — because a fold is an offset going down and a gap is one going up. The resolver is written as that one rule, with both vectors pinning it.

- **Checked against the format's vectors.** All sixteen cases of `vectors/meta.json`, fixtures read as bytes with their `blake3` name asserted first so a mangled read fails there rather than somewhere confusing, and parsed structures compared rather than JSON text. Four more tests cover what the vectors deliberately leave open or do not reach: both fallbacks, a folded header with an address list and an obsolete zone, and a resource holding only an override beside a `VTIMEZONE` that never changes.

## Two tasks that did not survive contact

- **Casefolding.** The task said to casefold the contact key "which is what Annex A.2 says". Annex A.2 does not say that any more: it now names the **Unicode simple lowercase mapping**, locale-independent, then a trim, and the vectors stay ASCII on purpose because a non-ASCII case "would pin behaviour the two runtimes have not been checked against each other on". Casefolding would fold `ß` to `ss` and put this crate at odds with both the format and the two implementations that already lowercase. What landed is lowercase-then-trim, with the note that Rust exposes only the full mapping, which differs from the simple one on the Greek final sigma and on `İ` alone.

- **`in_reply_to` and the `mid:` prefix.** The vectors give a message's `link_id` as the bare `Message-ID` (`basic-1@example.org`). neverest writes `mid:basic-1@example.org`. The bare form is what landed, since the vectors are the format's own and were derived from the prose; the prefix is a divergence, and a live one: adopting this in neverest re-links every item in every existing store, which is a resync, not a recompile. That is the downstream task's problem to state, and it is stated here so it is not discovered there.

## Dependency

`serde` becomes a direct dependency with `derive`. It was already in the tree under `serde_json`, so nothing new is compiled but the macro. The optional `serde` feature that `store-compaction` added for the diagnostics is gone with it: a feature gating derives on types this crate itself has to serialise cannot hold, and an alias feature would be worse.

## Downstream, none of it done here

neverest, cardamum, calendula and the Android app each still carry their own copy. Their tasks stay unchecked, and the Android one is the one that matters: it writes no calendar summary and no sort key at all, and it cannot reach this module.

## A correction

This change's `delta.md` already existed and was overwritten before it was read. Its content is back, merged into the requirement it belonged to: the sentence about two writers producing a collection whose rows one of them cannot render or order, "exactly as two hashes produce blobs neither finds", and the rule that a consumer derives through this rather than carrying its own copy. The frontmatter it carried was `cairn: delta`; the merged file says `cairn: change`, which is what every other delta written since uses.
