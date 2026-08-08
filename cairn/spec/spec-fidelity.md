---
cairn: spec
capability: spec-fidelity
status: current
---

# Spec fidelity

The `sql` module is this crate's copy of the pimdir specification's SQL,
dependency-free and reachable by name so a consumer holding its own SQLite
driver can run it without transcribing anything. The spec lives in a separate
repository, so nothing about the build notices when one moves and the other does
not. This capability is what notices.

### Requirement: The inlined schema is semantically identical to the canonical one
`sql::MIGRATION_0001` SHALL declare the same tables, columns (name, declared
type, nullability, default and primary-key position), foreign keys (including
their `ON UPDATE` and `ON DELETE` actions) and declared indexes as the
specification's migration.

Equality SHALL be checked through SQLite's own pragmas after applying both,
never by comparing text: comments and formatting are free to differ, and a
text comparison would fail on prose while missing a dropped default.

#### Scenario: A foreign-key action is dropped in the copy
- GIVEN a canonical key carrying `ON UPDATE CASCADE`
- WHEN the inlined copy omits it
- THEN the check fails, rather than waiting for a rename to be refused in
  production

### Requirement: No canonical statement is silently absent
Every named statement in the specification's `queries/` SHALL have a constant in
`sql::ALL`, or appear in an explicit list of substitutions naming what replaced
it. An implementation may substitute an equivalent statement (SPEC §4.4), and
may not quietly drop an operation.

### Requirement: Statement text is not compared
The check SHALL NOT require an inlined statement to match the canonical text.
The specification permits an equivalent that preserves the same invariants, and
this crate uses that permission deliberately, so requiring textual equality
would forbid what the specification allows.

### Requirement: Every inlined statement prepares against the inlined schema
Every constant in `sql::ALL` SHALL prepare successfully against a database
created from `sql::MIGRATION_0001`. A statement naming a column the schema does
not have is drift the name check cannot see, and would otherwise surface only
when a consumer ran it.

### Requirement: The check degrades to a skip, never to a failure
The specification is a sibling checkout rather than a vendored copy, so the
comparison SHALL skip when it is absent (a consumer building from the registry
has no reason to hold it) and SHALL run whenever the two sit side by side, which
is where drift is created.
