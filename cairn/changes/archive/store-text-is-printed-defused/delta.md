---
cairn: delta
change: store-text-is-printed-defused
---

## ADDED Requirements

### Requirement: Store strings a server named are printed defused
Every text output SHALL pass a string the store holds through `pimalaya_cli::sanitize::text` before printing it: `queue status` its collection, its parked or unconfirmed error and each refusal's source, collection, instant and reason; `item restore` its collection and handle; `item erase` its collection; `item list`'s empty-collection line; `item show` its object hashes and summary table; `store info` its store id and format version; and every confirmation prompt, its whole question. A collection id is a server's mailbox name and an error or a refusal is what a server answered, so ESC, BEL, C1 and bidi characters print as U+FFFD.

#### Scenario: A hostile mailbox name and server answer
- GIVEN a status whose collection holds an OSC 52 sequence and whose error, refusal source, collection, instant and reason hold ESC, BEL and C1
- WHEN it prints as text in each state
- THEN every one of them prints U+FFFD in their place (src/cli/queue.rs `a_status_prints_what_the_store_names_sanitized`, src/cli/item.rs `a_restore_and_an_erase_print_the_collection_sanitized`)

## MODIFIED Requirements

## REMOVED Requirements
