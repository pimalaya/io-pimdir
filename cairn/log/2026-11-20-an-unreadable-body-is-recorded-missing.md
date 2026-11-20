---
cairn: log
change: an-unreadable-body-is-recorded-missing
landed: 2026-11-20
---

# An unreadable body is recorded missing

Left by the snippet fix of the same day (an-unreadable-blob-cuts-no-snippet): the refresh's answer to `WantsBodies` (src/client/search/refresh.rs, `self.blobs.get(&hash)?`) failed the whole run with `PimdirError::Io` on a blob it could not read (permission denied), so one unreadable body stopped every later body from being indexed. SEARCH §4 ("A body that cannot be read is tried again") records a placement whose body the indexer cannot open in `missing` and folds it again each pass.

## What landed

- src/client/search/refresh.rs: a read failing is logged at `debug` and the hash left out of `Bodies`, as a gone body is; the coroutine records the placement in `missing`, the relink reads no head from it, a re-expansion keeps its occurrences, and the next pass tries it again.
- Tests: tests/search.rs `a_body_the_refresh_cannot_read_is_recorded_missing_then_indexed`, which failed on ca354b3 with `Io(Os { code: 13, kind: PermissionDenied })`.

## Capabilities moved

search ("A refresh folds the feed in bounded, resumable batches", modified).

In the working tree on io-pimdir ca354b3 (0.18.1), no version bumped; CHANGELOG.md [Unreleased] names it.
