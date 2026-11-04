---
cairn: log
change: every-command-has-a-schema
date: 2026-11-04
---

# Every command has a schema

- **queue status** (src/cli/json_schema.rs; cli.md). `pimdir-queue-status` registers `QueueStatusOutput`, which `json-schema` left out since the verb landed.
- **Registry test** (src/cli/json_schema.rs). `the_registry_matches_the_command_tree` runs pimalaya-cli 0.3's `JsonSchemaRegistry::check` against `Cli::command()`, exempting the two variant keys, `pimdir-search-threads` and `pimdir-query-error`, which name an output a flag or a refusal selects rather than a command.
