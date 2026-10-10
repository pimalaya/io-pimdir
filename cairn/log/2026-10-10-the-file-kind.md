---
cairn: log
change: the-file-kind
date: 2026-10-10
---

# Files, and attachments that stand for a message's parts

Implements pimdir's the-file-kind (draft-04, untagged), spec/ re-vendored: `file_summary`, the `item_reference_collects_files` trigger and the statements `upsert_file_summary`, `load_file_summaries`, `list_files_page_asc`, `get_file`, `list_attachments`.

- `summary::file`: `KIND`, `PimdirFileSummary` (name, media type, size, part) with its sort key and derivation, and `part_key`; `PimdirSummary::File`, which states no hint, titles by name and names no address.
- The write path's `PimdirSummaryTable::File` loads, reads and upserts the table for a collection of the kind; an undeclared kind never reads it, a file collection being always declared, so a reader of an unreconciled store never queries a missing table.
- `PimdirStore::put_file` writes or restates a body-less, binding-less file at `Meta` in one transaction, its `seq` shared with the key's other rows (`part:` and `file:` are not among the keys kept apart), and refuses another kind with `PimdirError::NotFiles`. A file with a body (saving to a folder) waits for the attachments step.
- `PimdirReader::list_attachments` and `PimdirAttachment`; `list_summaries` pages a file collection; the CLI prints `file_summary`.
- Open reconciles `file_summary` among `RECONCILED` and the trigger through `RECONCILED_OBJECTS`.
- Capabilities moved: store (files), summaries (Annex A.7).

## Verification

tests/files.rs: stand-ins listed A to Z at `Meta` with no body, restated under the same `seq`, a mail collection refused; a message's attachments in the order recorded with their part, the saved copy's body read through the stand-in, a stand-in gone with its reference and the rest with the message's last row while the saved copy stays; an older store read as having none, then reconciled. `cargo test --all-features`, `--no-default-features --features cli`, clippy `-D warnings`, `cargo doc` and `cargo fmt --check` green; `spec_drift` against the pimdir checkout.
