---
cairn: tasks
change: the-json-output-is-the-integration-point
---

- [x] Owner decisions on the Open list (2026-11-21: one probe, `formatVersion` follows io-pimdir's major, no `--format-version` until a second major)
- [x] pimdir the-freeze-plan D4 amended (pimdir's lead): "outside but for the JSON output this change names", logged in pimdir cairn/log/2026-11-21-the-json-output-is-a-contract.md
- [x] cairn/spec/cli.md: the contract's commands, its stability rules, the version a program reads; README and `--help` say which part is the contract
- [x] `pimdir --json version` prints `formatVersion` (src/cli/version.rs, `pimdir-version`)
- [x] A test pinning each contract command's schema against a committed copy, so a change of shape fails until the version moves (tests/json_contract.rs over tests/json_contract/, proven by renaming `nextPage` in `SearchOutput`: the test failed naming `pimdir-search`)
- [x] CHANGELOG, log entry
- [ ] No vocabulary is declared open yet: every enum of the contract (a status, a coverage kind, the error `code`) is closed, so a new value is a major until a schema or cli.md declares which vocabularies are open
- [ ] The test compares copies whole: an additive change fails too and is rewritten on purpose, the test does not tell an additive change from a breaking one
- [ ] `formatVersion` is `0` below 1.0 while each schema's `$id` names `v0.<minor>`, the version semver breaks on there: the two agree from 1.0 on
- [ ] `store info` prints its own `formatVersion`, the STORAGE draft as a string, beside the probe's integer of the same name
- [ ] The integrator gaps the proposal lists, each its own change: the thread summary and the reply tree (pimdir a-query-reads-across-domains), the message's parts for display, the local properties in the contract payloads (`property list` is outside it today)
