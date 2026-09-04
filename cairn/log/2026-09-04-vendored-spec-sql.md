---
cairn: log
change: vendored-spec-sql
date: 2026-09-04
---

# The canonical SQL is vendored and generated

The 99 transcribed constants are gone from sql.rs. spec/ is a byte-for-byte copy of the specification's storage migrations and statements, build.rs generates a constant per file with the file's comment as its documentation, and the drift test compares the copy against the sibling checkout both ways. The two descending pages that bound a NULL cursor were upstreamed first, so the copy is verbatim with no substitution list; spec-fidelity moved accordingly.
