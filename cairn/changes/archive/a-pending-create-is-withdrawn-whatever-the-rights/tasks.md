---
cairn: tasks
change: a-pending-create-is-withdrawn-whatever-the-rights
---

# Tasks

- [x] spec/queries/storage/read/load_item_capabilities.sql and signatures.json re-vendored
- [x] tests/capabilities.rs `a_pending_create_is_withdrawn_whatever_its_source_may_delete`, failing on the old gate: `Unsupported(PimdirRefusal { capability: "mail.message.remove", source: "graph", detail: Some("pull-only") })` (2026-11-15)
- [x] src/client/capability.rs `holding`, read for a remove, a move and a copy and for a removed resource's content needs
- [x] cairn/spec/store.md "A pending create is withdrawn whatever its source may delete"
- [x] The suite in CI's three configurations (2026-11-15)
- [x] spec/REVISION to the pimdir commit carrying it (pimdir 9ce9c3d)
- [ ] calendula reads `item_capabilities` (src/pimdir/backend.rs:366): if it greys out a delete from it, it greys out a withdrawal too
