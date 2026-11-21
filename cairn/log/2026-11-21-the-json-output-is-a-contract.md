---
cairn: log
change: the-json-output-is-the-integration-point
landed: 2026-11-21
---

# The JSON output is a contract

The `pimdir` binary's JSON output becomes the integration point, in place of a C library (the owner's ruling of 2026-11-21, after pimdir's compared-with-notmuch). What `--json` prints for `search`, `count`, `show`, `thread`, `tag`, `item list`, `item show`, `collection list`, `queue list`, `queue status`, `store info` and `version`, and the failure object `pimdir-error`, is stable within a contract major: a minor adds keys, and values only to vocabularies declared open; removing or renaming a key, changing a type or closing a vocabulary moves the major. The commands, the flags, the text output and the operator verbs stay outside io-pimdir's 1.0 contract; pimdir's the-freeze-plan D4 is amended to say so (pimdir cairn/log/2026-11-21-the-json-output-is-a-contract.md).

The owner's decisions: one probe, `pimdir --json version`, prints `formatVersion` and no payload carries it; `formatVersion` is io-pimdir's major version, so a Rust API break moves it even when the JSON did not change; no `--format-version` is served until a second major exists. Below 1.0 it prints `0`, the crate's major as it stands, the proposal saying nothing of 0.x. The probe also prints `version` and the STORAGE, SEARCH and SYNC drafts the build implements (src/cli/version.rs, `pimdir-version`).

Rules and their checks: the stability rule by tests/json_contract.rs `every_contract_payload_matches_its_committed_schema`, which compares each payload's schema as `pimdir json-schema` prints it with the copy under tests/json_contract/, its `$id` (the release) and its `description` prose left out, and fails on a copy naming no contract payload; `PIMDIR_BLESS_CONTRACT=1` rewrites the copies on purpose. Renaming `SearchOutput`'s `nextPage` made it fail naming `pimdir-search`, and reverting made it pass. The probe by `the_version_probe_states_the_crate_major` and src/cli/version.rs's unit tests; the registry test keeps `pimdir-version` described.

Capabilities moved: spec/cli.md ("The CLI is outside the crate's 1.0 contract" modified, "Integrators read the JSON output" and "A program reads the contract's version from one probe" added, `version` in the verb surface). README.md and `pimdir --help` say which part is the contract.

Left, as open lines of the change's tasks.md: no vocabulary is declared open yet, so every enum is closed; the test cannot tell an additive change from a breaking one; `formatVersion` `0` beside each `$id`'s `v0.<minor>` below 1.0; `store info`'s own `formatVersion`, a STORAGE draft string; the integrator gaps the proposal lists (thread summary and reply tree, message parts, local properties).
