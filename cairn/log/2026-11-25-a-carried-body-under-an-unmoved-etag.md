---
cairn: log
change: a-carried-body-under-an-unmoved-etag
landed: 2026-11-25
---

# A carried body under an unmoved ETag

The owner ruled pimdir's Q1 on 2026-11-25, reading B (pimdir cairn/log/2026-11-25-a-carried-body-under-an-unmoved-etag.md): a member carrying, at the revision its base names, a body differing from the one that base holds is a remote edit (pimdir SYNC §5, §6, §8, sync-draft-16 edited in place). Before, the engine decided a pull by the revision alone: a DAV listing serving another card under the base's ETag (quirks `revisions` step 2, every seed) left the old body under the carried body's summary and sort key, a row disagreeing with itself, and a local edit pending over it was pushed gated on that ETag over the server's body unread.

## What moved

- src/placement.rs: `PimdirPlacement::moved_by(revision, carried)`, the one reading: a revision the base does not hold, or at the base's own one a carried body whose object hash differs from the base's. A member stating no revision changed nothing; one carrying no body, or a base holding none, leaves the revision alone deciding.
- src/sync.rs: `name_members` leaves such a member's summary and sort key to the content axis; `reconcile_content` (the item conflict's branch included) and a tombstone's revive in `merge` read it through `moved_by`, so the policies, the lost-push read and `KeepLocal` apply as at a moved revision.
- src/hub.rs: the shared-item fold tells a pull dropping a local edit, which it keeps as a retained item, by the base's body as well as its revision, a pull under an unmoved revision moving the body alone.
- src/upgrade.rs: a `Full` fetch over a local edit at the base's revision carrying another body than the base holds is the both-changed case, a `Conflict` at that revision with the fetched body as `conflict_object`.
- src/rekey.rs: `carry` reads the carried member the same way.
- spec/generator/src/engine.rs: the model's `refresh` and `rekey` follow the rule, so the model and the engine agree.
- tests/generated/campaign.rs: `ETAG_KEPT_OVER_A_SERVER_EDIT` and the `known` lookup filing replayed differences under it removed; a replayed difference stands under no finding again.
- tests/engine/common.rs: the fake remote stored a pushed object's name as the body and served, under the revision the push was answered at, a body hashing to another name, a server rewriting every body it accepts; read as a remote edit now, the hub suites never settled (15 tests of tests/engine, "the hub never settled"). It now serves the bytes a push wrote, read back from the store's blobs (`MemRemote::blobs`, set by `Client`), as a server keeping what it accepted does; the name stays the body the tests read.
- spec/vectors/: pimdir's vectors/ copied byte for byte, vectors 237 to 242 new.
- cairn/spec/sync.md (new requirement "A carried body is a remote edit under an unmoved revision", the re-listed member's refresh narrowed to a member carrying no body or the base's), cairn/spec/upgrade.md, cairn/spec/rekey.md.

## Checks, each failing on b5606f3

- vector 237, nothing staged, pulled: "events differ" (no `ContentChanged`).
- vector 238, a local edit under `manual`, conflicted at `r1`: "pushes differ" (an `Update` gated on `r1`).
- vector 239, the same under `prefer-local`, the carried body kept before the push: "events differ" (no `Overruled`).
- vector 240, a tombstone revived: "pushes differ" (a `Remove` gated on `r1`).
- vector 241, a rekey carrying a conflict at `r1`: "bindings rows differ".
- vector 242, `prefer-remote` pulling and keeping the local edit: "pushes differ" (an `Update` gated on `r1`); with src/hub.rs left as it was, "events differ" (the edit dropped unkept, no `Overruled`).
- `upgrade::tests::a_body_fetched_at_the_base_revision_over_an_edit_conflicts`: `Dirty`, not `Conflict`.
- The quirks profile over campaign-1, its `revisions` step 2 compared again with no known finding.

## Gates

- `cargo fmt --all --check`, `cargo clippy --all-features --all-targets -- -D warnings`: clean.
- `cargo test --all-features --no-fail-fast` and `--no-default-features --features cli`: 1,225 passed, one failed, `the_vendored_copy_is_the_spec_byte_for_byte`, spec/REVISION naming a pimdir commit that does not hold vectors 237 to 242 yet; it passes once spec/REVISION names the pimdir commit landing them. `--no-default-features --lib`: 440 passed.
- spec/generator: fmt and clippy clean, 99 tests passed.
- The quirks profile over campaign-1, 24 seeds (1201 to 1224), release, 12 workers: 384 jobs, no failure, no difference.
