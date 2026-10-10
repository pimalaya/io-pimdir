---
cairn: log
change: automatic-references
date: 2026-10-10
---

# The writer records the automatic references

Implements pimdir's automatic-references (draft-04, untagged), spec/ re-vendored: the rule statements `link_senders_of`, `link_mail_from`, `link_invitations_of`, `link_invitations_to`, `mail_summary.invitation` and its index.

- `client::write` runs the rule for an item whenever its summary is first written or moves, after its addresses, in the batch's transaction (`link_references`): a mail's senders and, when it carries one, its invitation; a contact's mail; a calendar item's invitations. Every frontend writing through `PimdirStore` records them; an owner servicing the engine's writes itself (the Android app runs the canonical statements from Java) runs the same statements.
- `PimdirMailSummary::invitation`, derived by `summary::mail::derive` walking the parts (transfer encoding undone, lines unfolded), `None` from `derive_meta`; `summary::without_body` keeps a known one, as `upsert_mail_summary` does. The boundary parse is shared with the attachment walk.
- Open adds the column, cut from the canonical DDL, and the index among `RECONCILED_OBJECTS`. Reader statements do not read it, so an unreconciled store reads as before.
- No search change: io-pimdir implements no search layer.
- Capabilities moved: store (automatic references, invitation), summaries (Annex A.1).

## Verification

tests/references.rs `the_writer_records_the_automatic_references`: a contact before and one after the mail it sent, an event after its invitation, all recorded `auto` from the mail; the earlier-store test drops the column and index and finds them added. tests/summaries.rs runs the new invitation vector (the meta derivation agreeing but for the mark and the invitation). `cargo test --all-features`, `--no-default-features --features cli`, clippy `-D warnings`, `cargo doc` and `cargo fmt --check` green; `spec_drift` against the pimdir checkout.
