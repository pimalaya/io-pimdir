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

### Requirement: Every canonical statement prepares against the canonical schema
The comparison SHALL prepare each statement the specification's `queries/` files
carry against a database created from its `migrations/0001_init.sql`, not merely
check that each is inlined here by name.

The specification repository holds no toolchain, so this is the only place its
own SQL is ever loaded. Without it, a spec edit naming a column the migration
does not have is found by whichever consumer runs it first, and the name check
above cannot see it.

### Requirement: Object naming is checked against the format's vectors
The crate SHALL check its object names against the specification's
`vectors/objects.json` (SPEC §16): every body, under both `hash_algo` values,
fed whole and fed in pieces, together with the shard path each name derives.

It is the one vector file the format makes a **MUST**, and the reason is the
failure mode rather than the importance. A store whose two writers name the same
body differently reports nothing: it does not error, no read returns a wrong
answer, and it silently never deduplicates and never finds the blob the other
side wrote.

The shard path is as normative as the name, so it is checked with it: two
writers agreeing on a name and disagreeing on where it lives still never find
each other's bodies. `PimdirBlobs::path` is public for the same reason, SPEC §14
inviting a consumer to stream a body straight to it.
