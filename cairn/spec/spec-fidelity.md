---
cairn: spec
capability: spec-fidelity
status: current
---

# Spec fidelity

The `sql` module is generated at build time from spec/, a byte-for-byte copy of the pimdir specification's migrations/storage/ and queries/storage/, so a consumer holding its own SQLite binding runs the format's own statements by name and nothing is transcribed. The spec lives in a separate repository, so nothing about the build notices when one moves and the other does not. This capability is what notices.

### Requirement: The vendored copy is the specification's, byte for byte
spec/migrations/storage/ and spec/queries/storage/ SHALL be identical to the specification's directories of the same name, file by file and both ways, and tests/spec_drift.rs SHALL compare them whenever the spec checkout sits beside this one. Re-vendoring is a copy of the two directories; nothing under spec/ is edited here.

### Requirement: The constants are generated, never written
build.rs SHALL emit one constant per statement file, named after the file in upper case, documented by the file's leading comment, plus one per migration and `VERSION` as the count of migrations, and `sql::CANONICAL` indexing them. `sql::OWN` indexes this crate's own statements, and `sql::all()` both. A canonical statement is therefore never substituted: the specification's text is the constant's.

### Requirement: A statement of the crate's own serves no profile
`sql::OWN` SHALL hold only what the operator tool asks that no profile of the standard needs: the consistency diagnostics behind `pimdir check` and the figures behind `store info`. A statement a reader, a producer or an owner needs to meet the standard is upstreamed to the specification, never kept here.

#### Scenario: A statement is added upstream
- GIVEN a new file under the specification's queries/storage/
- WHEN the two directories are copied over
- THEN the next build carries its constant, with no Rust edit

### Requirement: The generated schema is semantically identical to the canonical one
`sql::MIGRATION_0001` SHALL declare the same tables, columns (name, declared type, nullability, default and primary-key position), foreign keys (including their `ON UPDATE` and `ON DELETE` actions) and declared indexes as the specification's migration, checked through SQLite's own pragmas after applying both.

### Requirement: No canonical statement is silently absent
Every statement file under the specification's queries/storage/ profile directories (read/, queue/, owner/) SHALL have a constant reachable through `sql::all()`.

### Requirement: Every statement prepares against the schema
Every constant `sql::all()` yields SHALL prepare against a database created from `sql::MIGRATION_0001`, and every statement the specification checkout carries SHALL prepare against its own migration. The specification repository holds no toolchain, so this is the only place its SQL is ever loaded.

### Requirement: The check degrades to a skip, never to a failure
The comparisons against the specification checkout SHALL skip when it is absent (a consumer building from the registry has no reason to hold it) and SHALL run whenever the two sit side by side, which is where drift is created.

### Requirement: Object naming is checked against the format's vectors
The crate SHALL check its object names against the specification's vectors/objects.json (STORAGE §16): every body, under both `hash_algo` values, fed whole and fed in pieces, together with the shard path each name derives. A store whose two writers name the same body differently reports nothing: it silently never deduplicates and never finds the blob the other side wrote. `PimdirBlobs::path` is public for the same reason, STORAGE §14 inviting a consumer to stream a body straight to it.
