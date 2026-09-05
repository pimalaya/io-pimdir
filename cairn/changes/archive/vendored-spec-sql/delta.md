---
cairn: delta
change: vendored-spec-sql
---

# Delta

## ADDED Requirements

### Requirement: The vendored copy is the specification's, byte for byte
spec/migrations/storage/ and spec/queries/storage/ SHALL be identical to the specification's directories of the same name, file by file and both ways, and tests/spec_drift.rs SHALL compare them whenever the spec checkout sits beside this one. Re-vendoring is a copy of the two directories; nothing under spec/ is edited here.

### Requirement: The constants are generated, never written
build.rs SHALL emit one constant per statement file, named after the file in upper case, documented by the file's leading comment, plus one per migration and `VERSION` as the count of migrations, and `sql::CANONICAL` indexing them. `sql::OWN` indexes this crate's own statements, and `sql::all()` both. A canonical statement is therefore never substituted: the specification's text is the constant's.

## MODIFIED Requirements

### Requirement: No canonical statement is silently absent
Every statement file under the specification's queries/storage/ profile directories (read/, queue/, owner/) SHALL have a constant reachable through `sql::all()`. The substitution list is gone: nothing is transcribed, so nothing can be substituted.

### Requirement: Every statement prepares against the schema
Every constant `sql::all()` yields SHALL prepare against a database created from `sql::MIGRATION_0001`, and every statement the specification checkout carries SHALL prepare against its own migration. Merged from the two prepare requirements, since the inlined and the canonical statements are the same text now.

## REMOVED Requirements

### Requirement: Statement text is not compared
Gone: the text is the specification's own, so comparing it is a byte comparison of the vendored directories rather than a semantic one of transcribed statements.
