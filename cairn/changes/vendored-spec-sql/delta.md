---
cairn: delta
change: vendored-spec-sql
---

## ADDED Requirements

### Requirement: The vendored copy is the specification's, byte for byte
### Requirement: The constants are generated, never written

## MODIFIED Requirements

### Requirement: No canonical statement is silently absent
Reachable through `sql::all()`; no substitution list.

### Requirement: Every statement prepares against the schema
Merged from the two prepare requirements.

## REMOVED Requirements

### Requirement: Statement text is not compared
