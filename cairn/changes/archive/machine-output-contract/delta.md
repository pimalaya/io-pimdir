---
cairn: delta
change: machine-output-contract
---

## ADDED Requirements

### Requirement: The JSON contract is described and versioned
`pimdir json-schema` SHALL describe, under the key `pimdir-error`, the failure object every command prints under `--json`. Every generated schema SHALL carry an `$id` naming the contract version, which is the minor version while pimdir is below 1.0 (`v0.8`) and the major version after. Every registered payload SHALL be a JSON object, checked by a test. A list SHALL be printed under a key naming its rows, `collection list` printing `{"collections": [...]}`. Piped `--json` output SHALL print one value per line.

#### Scenario: A caller reads a failure against its schema
- GIVEN `pimdir json-schema pimdir-error`
- WHEN it runs
- THEN the schema of the JSON failure object is printed, its `$id` being `https://pimalaya.org/schemas/pimdir/v0.8/pimdir-error.json`
