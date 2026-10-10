---
cairn: log
change: reconcile-by-schema-text
date: 2026-10-10
---

# Open reconciles against the canonical schema's own text

Implements pimdir's reconcile-by-schema-text (draft-04, untagged), spec/ re-vendored with `list_held_mail`.

- `schema::reconcile` applies `sql::MIGRATIONS` to an in-memory database and compares every table, index and trigger with the store's by their normalised `sqlite_schema` text: a current store is left untouched; otherwise, foreign keys off and `legacy_alter_table` on, one transaction recomputes the refcounts, rebuilds each differing table from the canonical text (indexes and triggers dropped, renamed aside, created, shared columns copied, `sqlite_sequence` carried, the aside table dropped), creates what is missing, recreates what differs, drops what the canonical schema lacks, backfills `bindings.shared_object` and `mail_summary.invitation` when the rebuild added them, and checks foreign keys.
- It replaces every per-change check: `RECONCILED`, `RECONCILED_OBJECTS`, `reconcile_role`, `reconcile_rounds`, `reconcile_invitation`'s column step, `reconcile_role_constraint` and `rebuild_collections` are gone; the invitation backfill pages `list_held_mail`.
- A store missing a core table or trigger (`SCHEMA`) is refused before reconciling, a damaged store rather than an earlier one.
- Capability moved: store (reconcile on open).

## Verification

The earlier-store tests (draft columns and triggers dropped, references and files tables dropped, the invitation column dropped, the role constraint edited) pass through the generic path; the role test now also checks that the rebuilt `collections` statement equals a fresh store's. `cargo test --all-features`, `--no-default-features --features cli`, clippy `-D warnings`, `cargo doc` and `cargo fmt --check` green; `spec_drift` against the pimdir checkout.
