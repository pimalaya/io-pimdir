---
cairn: log
change: a-same-id-message-keeps-its-body
landed: 2026-11-21
---

# A same-ID message keeps its body

The crate follows pimdir cairn/log/2026-11-21-a-same-id-message-keeps-its-body.md (storage-draft-15 and sync-draft-15 edited in place, uncommitted there): spec/ is re-vendored byte for byte from that working tree, and spec/REVISION reads `UNSET` until the pimdir commit exists. The owner chose option (b) on 2026-11-21 and approved fix 1.

- **A relocation names the member it made** (cairn/spec/sync.md): `PimdirPushResult::assigned` now also carries the handle an accepted relocating `Remove` gave its member in `to`. The engine (`PimdirSync::relocated`) then writes `PimdirWriteOp::LandPlacement` for the destination's create, whatever its key, instead of a `DeliverPlacement`; the hub moves the binding to that handle, its base the item's flags and body; the write licenses the provisional handle as a `Superseded` drop does, and records the create delivered instead when the source already binds the stated handle there. Guards: vectors/sync/196 (fails with `assigned` ignored), 197; tests/relocation_landing.rs (three cases).
- **A server object id decides linking where both state one** (cairn/spec/upgrade.md, seam.md): `PimdirRemoteMeta::server_object_id` (with `with_server_object_id`), `PimdirPlacement::server_object_id`, `PimdirBinding::server_object_id`, `PimdirHeldObject::server_object_id`; the sync records a listed member's id on its binding when it names or relists it (a listing stating none clears it), the rekey carries the listed one, `lookup_objects` binds the store's source and answers each holder's id, and the upgrade's `agrees` refuses a holder stating another id than the placement's. Guards: vectors/sync/198 (fails with the ids not compared), 199, 200 (fails with the listing recording no id); src/upgrade.rs `a_body_links_only_under_an_equal_server_object_id`.

`binding_from_row` reads the id only from a row answering it: `list_item_bindings`, a reader's statement, answers none. tests/vectors_sync.rs seeds and compares `server_object_id` on a binding and reads it from a member's meta. Every `PimdirPlacement`, `PimdirBinding`, `PimdirRemoteMeta` and `PimdirHeldObject` literal in src/ and tests/ states the new field.

Results: `cargo test --all-features --no-fail-fast` 1081 passed, 0 failed, 8 ignored over 77 test binaries (200 sync vectors among them, spec_drift's sibling comparison finding no git checkout); clippy and fmt clean.

Left: neverest states no id (io-imap 0.12.0 cannot request `EMAILID`), the `seq` sharing and the purge across collections still read the agreement alone, and a fetch's id is not read.
