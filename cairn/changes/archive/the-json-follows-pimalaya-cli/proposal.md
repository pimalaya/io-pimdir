---
cairn: change
id: the-json-follows-pimalaya-cli
status: landed
created: 2026-11-04
---

# The JSON follows pimalaya-cli

## Why

pimalaya-cli 0.3 fixed the output contract every Pimalaya binary prints (its cairn/spec/output.md and error.md, change every-output-is-data). pimdir adopted only the 0.2-era subset (machine-output-contract): it still prints keys that name no command, a failure object of its own, skipped keys, camelCase values, composed prose and its own paging, and exits 0 on outcomes short of success. Himalaya, calendula and cardamum moved on 2026-11-03; pimdir follows now, so a caller reads one contract across every binary.

## What

Decided with the user on 2026-11-04: the item's public id stays `seq` (the pimdir drafts' word, the SEARCH `(account, seq)` address), carrying the receipt's `id` meaning; the collection row gains `description` and `color` from the store, and `default`, which the user chose to print `false` on the belief that a store has no default collection; STORAGE §14 gives calendar and address-book collections a `default` role, so `default` is that role.

- **One command, one schema.** `pimdir-search` describes both shapes (`hits` or, under `--threads`, `threads`); `pimdir-search-threads` goes. The registry test runs `JsonSchemaRegistry::check` with no exemption.
- **One failure object, coded.** A refused query is a failure (`query-refused`, exit 1) through `pimdir-error`, its position, expected tokens and fields in the message; `QueryErrorOutput` and `pimdir-query-error` go. Failures are evaluated through `ErrorReport::eval_coded` with pimdir's `ErrorCode`: io-pimdir's `PimdirError::code` relayed by its spelling, plus the CLI's own codes for failures a caller acts on.
- **Absent is null.** No `skip_serializing_if`: `collection list` and `queue status` print every key.
- **Values are kebab-case.** `tag` statuses (`unknown-flags`, `ambiguous-source`) and the remote coverage kind (`local-only`).
- **Shared nouns.** A queue row's id prints as `row` (`queue list`, `queue status`, `queue cancel`, `item restore`, `tag`). The collection row as decided above; an undeclared `kind` prints `null`, not an empty string.
- **Receipts.** `item restore` prints `queued`, its `status` going; `item purge` prints `count`; `export` prints `path`; `item export` prints `size`.
- **Paging resumes.** `item list` and `search` print `nextPage` and take `--page-token`; `--after` and `--offset` stay as hidden aliases.
- **Bytes are never raw under JSON.** `item export` without `--output` prints `{data, size, mime}` under `--json`, the media type from the collection's kind; text mode still streams the raw body.
- **No composed prose.** A queue row prints its stored `payload` text in place of the summary the CLI composed.
- **Partial outcomes exit 2.** `check` finding a problem it left unrepaired, `item restore` whose drain did not revive the item, `tag` with a placement neither planned, unchanged, applied nor queued, `export` leaving a body out, and `quarantine empty` leaving entries: each prints its result, then exits 2.

Every key change is breaking for a `--json` consumer; pimdir is 0.x and the schema `$id` is `v0.8`.
