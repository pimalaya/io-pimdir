---
cairn: log
change: reference-implementation
date: 2026-09-04
---

# The reference implementation, by profile

The standard now binds by profile (reader, producer, owner) and carries SYNC as the reference engine's specification, with the statements sorted one per file under read/, queue/ and owner/. This crate's handles already were those profiles; the README, the crate header, the sql header and spec-fidelity now say so, and the drift test walks the three directories.
