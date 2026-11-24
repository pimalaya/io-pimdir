---
cairn: tasks
change: a-same-id-message-keeps-its-body
---

- [x] Owner decision on linking: option (b), 2026-11-21, fix 1 approved the same day.
- [x] pimdir SYNC §4/§6: a relocation may state the target handle, and the destination's create lands on it in the write recording the outcome (STORAGE §14 `LandPlacement`); vectors/sync/196 fails without it, 197 pins the next listing minting the other member.
- [x] io-pimdir: `PimdirPushResult::assigned` documented for a relocation, `PimdirWriteOp::LandPlacement`, the hub's landing and the write's fallback to a delivery on a bound handle; tests/relocation_landing.rs.
- [x] neverest: `move_messages` hands the `COPYUID` target UID back (neverest cairn/changes/a-move-into-a-same-id-message-keeps-both).
- [x] Option (b) through the layers: `bindings.server_object_id` (storage-draft-15 edited in place), `insert_binding`/`update_binding`/`load_bindings*` and `lookup_objects(account, source, links)`, `PimdirRemoteMeta::server_object_id`, `PimdirPlacement::server_object_id`, `PimdirBinding::server_object_id`, `PimdirHeldObject::server_object_id`, the upgrade's comparison; vectors/sync/198 to 200.
- [x] neverest's repro, variant 1 (B listed before the move): owner ruling of 2026-11-24, option A (two members stating one `Message-ID` in a collection share no body by agreement, one download per duplicate, `EMAILID` out of scope); landed as cairn/log/2026-11-24-two-members-of-one-id-share-no-body.md, both variants pass on Stalwart.
- [ ] Open: the `seq` shared across collections (STORAGE §9.1) and a retired row's purge (§11.1) still read the agreement alone, not the server object id; a decision for the owner.
- [ ] Open: a fetch's server object id is not read (`PimdirFetchedItem` states none); only a listing's is recorded.
- [x] Fold, log, CHANGELOG.
