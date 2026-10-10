---
cairn: log
change: audit-answered
date: 2026-10-10
---

# The 2026-10-10 audit answered, and the linking reads

Implements pimdir's audit-answered (draft-04, untagged), spec/ re-vendored.

- The canonical changes ride in through the vendored SQL: the collecting trigger's seek, the derived-key `CHECK`s and filters, `collect_before` keeping person-referenced mail, `sum_mail` per message with `held`, `add_reference` answering an existing reference, and the new statements.
- `PimdirLinkId::is_derived`, used by the writer's public-id rule and by `summary::file::part_key`, now `Option`.
- `PimdirReader::sum_mail` takes `held`; `get_mail_row`, `describe_endpoint` (`PimdirEndpointView`), `search_contacts`, `search_files`, `search_calendar` (`PimdirCalendarHit`), `list_attachments_by_account` (`PimdirAccountAttachment`, `PimdirAccountAttachmentCursor`). The searches reuse `PimdirMailCursor` and `PimdirMailEntry`, whose docs now say they serve any page across collections. Address joining is shared by mail and contacts (`attach_entry_addresses`).
- `PimdirStore::delete_unbound`; `delete_collection` skips the recompute when `collection_holds_objects` answers false.
- Open: `reconcile_role_constraint` rebuilds `collections` from the canonical DDL when its role `CHECK` lacks `attachments` (foreign keys off, `legacy_alter_table` on so the rename does not re-parse other tables' triggers, its indexes and triggers recreated, `foreign_key_check` before commit); `reconcile_invitation` backfills the column from every held body and runs `link_invitations_of` over every mail. `schema::init` takes the store directory for it. A store reconciled by the previous dev commit (3c6aa82) already has the column and gets no backfill; it was never released.
- Capabilities moved: store (reads, deletes, reconcile), summaries (derived keys).

## Verification

tests/linking.rs (the searches, `describe_endpoint`, a message counted once and `held`, `get_mail_row`, a derived key taking no reference, the invitation backfilled on open), tests/files.rs (the role and `list_attachments_by_account` paged, `delete_unbound` refusing a bound item, an older role constraint rebuilt with its rows kept), and the updated reference and sum assertions. `cargo test --all-features` (34 binaries), `--no-default-features --features cli`, clippy `-D warnings`, `cargo doc` and `cargo fmt --check` green; `spec_drift` against the pimdir checkout.
