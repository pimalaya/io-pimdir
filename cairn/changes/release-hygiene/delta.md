---
cairn: delta
change: release-hygiene
---

## MODIFIED Requirements

### Requirement: Schema version
The store schema is version 1 (`user_version` 1), migrations/storage/0001_init.sql verbatim. build.rs SHALL generate `sql::MIGRATIONS`, every canonical migration in file order, and the owner open SHALL run STORAGE §6's runner over it: read `user_version`, then for each migration above it, in order, one transaction running the script and setting `user_version`, the first one seeding `store_meta`. A store stamped above the current version is refused with `PimdirError::Version`. A store stamped current by an earlier draft is reconciled by the owner open ("Open reconciles against the canonical text"); one lacking a core table or trigger the schema declares is refused with `PimdirError::Stale` naming it, by every role. The table scan SHALL run before `store_meta` is read, so a store lacking `store_meta` names that table rather than failing the read of its stamp.
