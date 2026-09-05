---
cairn: delta
change: format-conformance
---

## ADDED Requirements

### Requirement: An index whose columns moved is rebuilt
A store opened against a schema whose index of the same name holds different columns SHALL have that index dropped and recreated, since `CREATE INDEX IF NOT EXISTS` keys on the name and leaves the old shape in place. The check SHALL compare the columns rather than drop unconditionally, an index rebuild on every open of a large store being the cost this avoids.

### Requirement: Object naming is checked against the format's vectors
The crate SHALL check its object names against `vectors/objects.json` (pimdir SPEC §16), for every body, under both `hash_algo` values, whole and streamed, including the shard path derived from each name. It is the one vector file the format makes a MUST, and the reason is the failure mode: a store whose writers name a body differently reports nothing, it silently never deduplicates.

### Requirement: The format's own statements are checked
The fidelity suite SHALL prepare every canonical statement against the canonical schema, not merely check that each is inlined by name. The spec repository holds no toolchain, so this is the only place its SQL is loaded.

## MODIFIED Requirements

None.

## REMOVED Requirements

None.
