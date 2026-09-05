---
cairn: delta
change: cli-and-packaging
---

# Delta

## ADDED Requirements

### Requirement: The JSON output is a contract
Every data command SHALL hand the printer one output type deriving `Display`, `Serialize` and `JsonSchema`, with camelCase keys on it and on every nested row and status enum, and a `json-schema` subcommand SHALL list every command's output type by its dash-joined path (`pimdir-item-list`), so a consumer can validate what `--json` prints. The summary row is the one value printed with the store's own column names, since it is printed as stored.

### Requirement: The toolkit verbs are singular
`completion`, `manual` and `json-schema` SHALL be spelled singular, each with its plural as a hidden alias, as every Pimalaya CLI spells them.

### Requirement: One handle per verb
A verb SHALL open the store once, in the role it needs, and every helper it calls (the blob directory, the write source) SHALL take that handle rather than open another. Repair is the one case that holds two roles in sequence, dropping the reader before taking the owner.

### Requirement: The dump format is versioned
The manifest SHALL carry `format_version`, bumped whenever the dump's shape changes. Version 2 carries an item's summary columns and address rows where version 1 carried a raw meta string.

## MODIFIED Requirements

### Requirement: The verb surface
`store info` prints one schema version: the one the reader verified on open, which the build services, since a reader refuses any other stamp. Every command SHALL render as JSON under `--json` except `item export` to stdout, whose output is the body's raw bytes: it SHALL refuse `--json` unless `--output` names a file, and then reports the write as JSON.

The intro no longer claims a read-only diagnostic connection: the diagnostics are a library read on the handle the verb holds.

## REMOVED Requirements

None.
