---
cairn: change
id: the-json-output-is-the-integration-point
status: draft
created: 2026-11-21
---

# The JSON output is the integration point

## Why

pimdir's cairn/changes/compared-with-notmuch: notmuch's reach comes from libnotmuch's C ABI and from its CLI's structured output (`--format=json|sexp`, `--format-version=N`), on which notmuch-emacs, alot, aerc and others build. The owner (2026-11-21) rejects a C library for io-pimdir: the JSON output of the `pimdir` CLI is the integration point, more flexible and reachable from any language. Today that output is outside every contract: pimdir's the-freeze-plan D4 (2026-11-20) puts the whole binary outside io-pimdir's 1.0 contract, and cairn/spec/cli.md says a program depends on the library, never on the binary, although the JSON is already described (`pimdir json-schema`), camelCase, schema-versioned (`$id` naming the contract version) and tested for shape.

## What

- **The JSON output becomes a contract.** What `--json` prints for the reading and queueing commands (`search`, `count`, `show`, `thread`, `tag`, `item list`, `item show`, `collection list`, `queue list`, `queue status`, `store info`, and the failure object `pimdir-error`) is stable within a contract major: a minor adds keys and adds values only to a vocabulary declared open; removing or renaming a key, changing a type or closing a vocabulary is a major. The text output, the flags' spelling and the operator verbs (purge, erase, gc, quarantine, check, restore, backup) stay outside it, as D4 says.
- **A program states the version it reads.** Each payload, or the binary through one probe, names the contract version, and a `--format-version N` asks for an older major while it is still served, as notmuch's does; which of the two is the decision below.
- **Gaps an integrator meets first.** The output a front end needs and does not have today: the thread summary and the reply tree (pimdir a-query-reads-across-domains), the message's parts for display (`show` prints the index's extraction, not the MIME tree), the local properties (an-item-keeps-local-state). Listed here, each landing with its own change.
- **Amends D4.** pimdir's the-freeze-plan D4 reads "outside the 1.0 contract" for the whole binary; it becomes "outside but for the JSON output this change names".

## Decided (owner, 2026-11-21)

- **One probe.** `pimdir --json version` states the contract's `formatVersion`; payloads carry none. No `--format-version` is served until a second major exists.
- **The contract follows io-pimdir's major.** `formatVersion` is io-pimdir's major version; a Rust API break bumps it even when the JSON did not change.
