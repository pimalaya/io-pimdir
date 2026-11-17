---
cairn: log
change: store-text-is-printed-defused
landed: 2026-11-17
---

# Store text is printed defused

CLI family audit of 2026-11-15, PIM-4, open and wider at dcf49e2: `queue status` (its collection, its error, and since draft-13 the `Refused by` line with the server's own reason), `item restore`, `item erase` and the confirmation prompts printed store strings raw, so ESC, BEL and OSC 52 bytes a server put in a mailbox name or an answer reached the operator's terminal; cli.md requires the sanitizer.

## What landed

- src/cli/queue.rs, src/cli/item.rs, src/cli/store.rs: every store string those outputs print goes through `pimalaya_cli::sanitize::text`, and the few others found on the way (`item list`'s empty line, `item show`'s hashes and table, `store info`'s id and format version).
- src/cli/mod.rs: `confirm` sanitizes its whole question.
- Tests: src/cli/queue.rs `a_status_prints_what_the_store_names_sanitized`, src/cli/item.rs `a_restore_and_an_erase_print_the_collection_sanitized`. The prompts need a terminal and are judged by reading.

## Capabilities moved

cli ("Store strings a server named are printed defused", added).

In the working tree on io-pimdir dcf49e2 (0.15.1), uncommitted, no version bumped; CHANGELOG.md [Unreleased] names it.
