---
cairn: change
id: machine-output-contract
status: landed
created: 2026-10-27
---

# Adopt the shared machine output contract

## Why

pimalaya-cli now owns the machine contract every binary prints (cli `machine-output-contract`, commit `55b6693`): a schema for the failure object, schemas stamped with the contract version, piped `--json` delimited by newlines, and a check that every payload is a JSON object (what an MCP client takes as structured output). Himalaya adopted it first; pimdir follows, without error codes since it has none.

## What

- The JSON Schema registry returns a `JsonSchemaRegistry` (bin `pimdir`, the crate version), registers `pimdir-error` as pimalaya-cli's `ErrorReport`, and tests that no payload root is anything but an object.
- `CollectionsOutput` is a struct holding `collections` rather than a transparent `Vec`.
- Errors are still evaluated through the uncoded `ErrorReport::eval`.
- pimalaya-cli is pinned at `55b6693` until its 0.3.0 release.

`collection list` printed a bare array, the one root that was not an object: its rows now print under `collections`, a breaking change of the JSON output.
