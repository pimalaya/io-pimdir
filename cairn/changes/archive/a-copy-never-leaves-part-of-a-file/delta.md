---
cairn: delta
change: a-copy-never-leaves-part-of-a-file
---

## ADDED Requirements

### Requirement: A copy never leaves part of a file under a name
`store backup` and `store restore` SHALL write every file as a body is written (STORAGE §5): the source opened before anything is created, its bytes into a period-prefixed temporary beside the final name, flushed, then moved onto the name without replacing it (`link(2)` then `unlink(2)`). A body the target holds already under its name SHALL be verified: a sound one is kept, one failing its name is moved into quarantine/ (the restore holding the owner lock and the staging lock exclusively) and the copy put in its place. A body of the source failing its name SHALL be put in nowhere and named in the report's `skipped`. A backup SHALL copy its database under a temporary name and give it pimdir.db last, so a backup stopped part way holds no pimdir.db, which no restore takes.

#### Scenario: Unreadable, truncated and stopped
- GIVEN a store's body made unreadable during a backup, then a backup body made unreadable during a restore, then the store's body truncated under its name
- WHEN each copy runs and is run again
- THEN the stopped backup holds no pimdir.db, the stopped restore leaves nothing under the body's name, and the rerun puts the backup's body in, the truncated one in quarantine/ and `check --deep` sound (tests/store_text.rs `a_backup_and_a_restore_never_leave_part_of_a_file`)

## MODIFIED Requirements

## REMOVED Requirements
