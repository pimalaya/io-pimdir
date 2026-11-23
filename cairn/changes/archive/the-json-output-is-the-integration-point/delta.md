---
cairn: delta
change: the-json-output-is-the-integration-point
---

## ADDED Requirements

### Requirement: Integrators read the JSON output
The `--json` output of `search`, `count`, `show`, `thread`, `tag`, `item list`, `item show`, `collection list`, `queue list`, `queue status`, `store info` and `version`, and the failure object `pimdir-error`, SHALL be stable within a contract major: a minor only adds keys, and values to vocabularies declared open; anything else moves the major. Each payload's schema SHALL be pinned by a test against a committed copy.

#### Scenario: A key renamed
- GIVEN the committed schema of `pimdir-search`
- WHEN a change renames one of its keys without moving the contract's major
- THEN the schema test fails

### Requirement: A program reads the contract's version from one probe
`pimdir --json version` SHALL print `formatVersion`, the contract's major, which is io-pimdir's major version (`0` below 1.0). No other payload SHALL carry it, and no `--format-version` flag SHALL be served until a second major exists.

## MODIFIED Requirements

### Requirement: The CLI is outside the crate's 1.0 contract
The binary's commands, flags, text output and operator verbs SHALL be outside the semver contract io-pimdir 1.0 makes for its library (pimdir the-freeze-plan D4, amended 2026-11-21), but for the JSON output the requirement "Integrators read the JSON output" names, and `pimdir --help` and README.md SHALL say which part is the contract.

## REMOVED Requirements
