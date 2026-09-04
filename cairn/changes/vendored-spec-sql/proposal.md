---
cairn: change
id: vendored-spec-sql
status: landed
created: 2026-09-04
---

# The canonical SQL is vendored and generated

## Why

sql.rs transcribed the specification's 99 statements by hand through a one-off script, with two substitutions, and the drift test compared names, never text. A statement reworded upstream went unnoticed.

## What

spec/ holds a byte-for-byte copy of the specification's migrations/storage/ and queries/storage/; build.rs generates one constant per file, documented by its leading comment, plus the migrations, `VERSION` and `CANONICAL`. sql.rs keeps the crate's own statements under `OWN`, and `all()` chains both. The drift test compares the copy against the sibling checkout byte for byte. The two NULL-cursor substitutions were upstreamed first, so nothing is substituted.
