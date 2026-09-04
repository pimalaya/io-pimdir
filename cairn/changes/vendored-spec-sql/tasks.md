---
cairn: tasks
change: vendored-spec-sql
---

- [x] Upstream the NULL-cursor form of the two descending pages into the specification.
- [x] Vendor spec/migrations/storage and spec/queries/storage; build.rs generates canonical.rs.
- [x] sql.rs keeps the crate's own statements only; `all()` chains `CANONICAL` and `OWN`.
- [x] tests/spec_drift.rs compares the copy byte for byte and drops the substitution list.
