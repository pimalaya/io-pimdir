---
cairn: delta
change: the-json-follows-pimalaya-cli
---

## ADDED Requirements

### Requirement: The JSON follows pimalaya-cli's contract
The `--json` output SHALL follow pimalaya-cli's output and error specs, as every Pimalaya binary does. A value pimdir defines (a status, a coverage kind) SHALL be kebab-case. Every key of pimdir's own types SHALL be printed, `null` when the value is absent; a collection's undeclared `kind` is `null`, not an empty string. A queue row's id SHALL print as `row` wherever it appears (`queue list`, `queue status`, `queue cancel`, `item restore`, `tag`). A collection row SHALL carry the shared `{id, name, description, color, default}`, `default` being whether its source states the `default` role (STORAGE §14), beside the store's own columns. A queue row SHALL print its `payload` as the queue stores it, never a summary the CLI composed. The item's public id stays `seq`, the pimdir drafts' name for it, wherever a receipt names the item.

A write SHALL answer a receipt: `item restore` its `seq`, `collection`, `linkId`, `row` and `queued`; `item purge` its `cutoff` and `count`; `export` the `path` it wrote; `item export --output` the `path` and `size` written. A listing that can stop short (`item list`, `search`) SHALL print `nextPage`, `null` on its last page, and take it back through `--page-token`; `--after` and `--offset` remain hidden aliases.

Every failure SHALL print pimalaya-cli's failure object with a kebab-case `code`, `null` when none applies: a store failure carries io-pimdir's `PimdirError::code` by its spelling, and the CLI's own failures carry `item-not-found`, `collection-required`, `item-live`, `body-absent`, `body-missing`, `row-not-found`, `source-required`, `confirmation-required`, `cancelled`, `page-token-invalid` or `dump-exists`. A refused query SHALL fail with `query-refused`, its message naming the fault, where it lies, and what would be valid there.

A command that ran to its end short of full success SHALL print its result, then exit 2, its `--help` saying when: `check` leaving a problem `--fix` does not repair, `item restore` whose drain left the item retained, `tag` with a placement neither planned, unchanged, applied nor queued, `export` leaving a body out, `quarantine empty` leaving an entry in place.

#### Scenario: A body exported under JSON
- GIVEN a mail item holding its body
- WHEN `pimdir --json item export SEQ` runs without `--output`
- THEN it prints `{data, size, mime}`, `data` the body in base64 and `mime` `message/rfc822` (tests/cli_json.rs `an_exported_body_travels_as_data`)

#### Scenario: A check finding a quarantined body
- GIVEN a store whose quarantine/ holds a file
- WHEN `pimdir --json check --fix` runs
- THEN it prints the file under `quarantined` and exits 2 (tests/cli_json.rs `a_check_leaving_a_problem_exits_2`)

## MODIFIED Requirements

### Requirement: The verb surface
`item list` pages with `--page-token` and `--limit`, printing `nextPage`. `search --threads` prints its threads under the `pimdir-search` schema, `--limit` and `--page-token` then counting threads. A refused query fails with `query-refused` and exits 1. `item export` to stdout writes the body's raw bytes, and under `--json` prints it as `{data, size, mime}`, `mime` the collection's declared media type, `application/octet-stream` when none.

### Requirement: The JSON contract is described and versioned
Every runnable command SHALL have a schema whose key names it, `pimdir-error` aside, checked by a test with no exemption: a command printing one of two shapes (`search` with or without `--threads`, `item export` with or without `--output`) is described by both, their keys telling them apart. `pimdir-error` SHALL describe the failure object with its `code` typed by pimdir's code enum.

## REMOVED Requirements

### Requirement: The JSON contract is described and versioned
The `pimdir-search-threads` and `pimdir-query-error` keys, and the refusal object `search`, `count` and `tag` printed before exiting 1.
