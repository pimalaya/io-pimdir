---
cairn: log
change: the-json-follows-pimalaya-cli
date: 2026-11-04
---

# The JSON follows pimalaya-cli

pimdir's `--json` output now follows pimalaya-cli 0.3's output and error specs, as himalaya, calendula and cardamum did on 2026-11-03. The capability moved is cli (cairn/spec/cli.md, new requirement "The JSON follows pimalaya-cli's contract"; search.md's schema list).

- **Codes** (src/cli/error.rs). `ErrorCode` relays io-pimdir's `PimdirError::code` by its spelling and adds the CLI's own; `report` moved there from src/cli/mod.rs and every CLI failure a caller acts on carries one; src/main.rs evaluates through `ErrorReport::<ErrorCode>::eval_coded`. A refused query is a `query-refused` failure; `QueryErrorOutput`, its `process::exit(1)` and `pimdir-query-error` are gone.
- **Schemas** (src/cli/json_schema.rs). `OneOf` as in himalaya: `pimdir-search` describes hits and threads, `pimdir-item-export` the receipt and the bytes. The registry test runs `check` with no exemption, the variant list of feebd26 gone.
- **Shapes** (src/cli/collection.rs, queue.rs, item.rs, search.rs, export.rs). Absent is null, kebab-case values, `row`, receipts, `nextPage` with `--page-token`, bytes under JSON, the stored payload for the composed summary.
- **Exit 2** (src/cli/check.rs, item.rs, search.rs, export.rs, quarantine.rs), through `PartialOutcome`, each documented in its `--help`.
- **Tests**. tests/cli_json.rs pins the bytes, the paging, null for absent and check's exit 2; all four failed against 5b5aa20. tests/search_cli.rs now expects `query-refused` and exit 2 where a placement is not tagged.

The collection row's `default` was first decided as always `false`, on the belief that a store has no default collection; STORAGE §14's `default` role for calendars and address books says otherwise, so `default` reads that role.
