---
cairn: log
change: the-mail-write-names-from-himalaya
date: 2026-11-02
---

# The mail write names himalaya declares

Follows pimdir b63b39a (re-vendored; pimdir master, which now holds the former audit-2026-10-31 branch as one squashed commit), which lands pimdir cairn/changes/mail-write-names-from-himalaya into storage-draft-08 without a number bump. spec/ changes only under payloads/ (the two new schemas) and vectors/ (payloads.json, its schema, the manifest).

- **Flags** (src/capability.rs; store.md, the gate requirement). `MAIL_FLAGS_IMPORTANT` and `MAIL_FLAGS_JUNK`, read only where a row states them and so in no declaration list, as `CALENDAR_COMPONENT_VTODO` is. `flag_changes` maps `$important` and `$junk` (any case) onto them; `gating` reads a source's own row for either, `none` included, and `mail.flags.keywords` from a source stating none, the refusal naming the capability it read. `check` and the tag preview (src/client/search/tag.rs) go through it. The existing `a_set_flags_needs_only_what_it_changes` removed `$Junk` and expected `mail.flags.keywords`; it now removes `$Label`, the junk case moving to `important_and_junk_name_their_own_capability`.
- **Intents** (src/intent.rs, src/codec.rs, src/client/capability.rs, src/client/producer.rs; store.md, a new requirement). `PimdirCollectionRename` (`source`, `name`) and `PimdirCollectionDelete` (`source`), read strictly, the collection being the row's and never a member. `COLLECTION_RENAME` and `COLLECTION_DELETE` join the three domain lists beside `COLLECTION_CREATE`, and `intent_capability` maps both kinds. The gate refuses either with `NoPerformer` on an undeclared account, as it does a `collection-create`; `enqueue_collection_rename` and `enqueue_collection_delete` resolve the performer at the collection. Performing them stays the owner's.

Checks: tests/payloads.rs reads the six new payload cases through the codec and the types; src/capability.rs `important_and_junk_name_their_own_capability`, `important_reads_its_own_row_else_the_keywords_one` (its undeclared-row case fails on the code before, which read only the named row) and the Annex B grammar test at 34 names; tests/capabilities.rs `a_collection_is_renamed_and_deleted_on_itself_by_a_source_declaring_it`, `a_collection_rename_or_delete_needs_a_declared_source`. `cargo test --all-features` 809 passed, clippy clean.

Left: neverest declares and performs the two intents once its connectors grow the verbs, and declares the two flags where a provider carries them as fields (Gmail `IMPORTANT` and `SPAM`, Graph `importance`); himalaya maps them onto `mailbox.rename` and `mailbox.delete`.
