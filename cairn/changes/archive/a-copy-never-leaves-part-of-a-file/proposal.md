---
cairn: change
id: a-copy-never-leaves-part-of-a-file
status: landed
created: 2026-11-17
---

# A copy never leaves part of a file

## Why

CLI family audit of 2026-11-15, PIM-2, open at dcf49e2: `store backup` and `store restore` created the final name before opening the source and copied into it, so a source that could not be opened left an empty file, a copy killed mid-way left a truncated one, and the rerun STORAGE §8 prescribes met the name held and kept it; a corrupt local body was kept over the backup's sound one, and a backup killed during the bodies had its database written first and whole with nothing marking it incomplete.

## What

- src/client/backup.rs: every file goes through a period-prefixed temporary, flushed, then `link(2)` onto its name; a held body is verified, one failing its name moved into quarantine/ before the copy is linked; a source body failing its name is left out and named in `skipped`; the backup's database is written under a temporary name and takes pimdir.db last.
- src/client/blobs.rs: `unique` is the crate's.
- Test: tests/store_text.rs `a_backup_and_a_restore_never_leave_part_of_a_file`.
