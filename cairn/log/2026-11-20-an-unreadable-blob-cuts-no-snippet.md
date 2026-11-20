---
cairn: log
change: an-unreadable-blob-cuts-no-snippet
landed: 2026-11-20
---

# An unreadable blob cuts no snippet

Found by the pimdir backend tests of 2026-11-20 (cardamum `pimdir search`, the commands spec scenario "A page and a hit the store cannot read"): a hit whose blob could not be read (permission denied) failed the whole page with `PimdirError::Io`, since the snippet's `blobs().get(hash)?` propagated out of `PimdirIndex::hit` and every page loop applies `?` to it. A blob gone from the directory already cut no snippet.

## What landed

- src/client/search.rs: `PimdirIndex::snippet` cuts no snippet when the blob read fails, logging it at `debug`, so `search`, `threads` and `thread` answer every hit. The hit itself is read from the index alone; a consumer reading the body (cardamum's `card-unreadable`) meets the failure on that hit. No API change: `PimdirSearchHit::snippet` documents the case.
- Tests: tests/search.rs `a_hit_whose_body_cannot_be_read_lists_without_a_snippet`, which failed on 5859b69 with `Io(Os { code: 13, kind: PermissionDenied })`.
- Left: the refresh's `WantsBodies` answer (src/client/search/refresh.rs) still fails the run on a blob it cannot read where SEARCH §4 records a gone one in `missing`; not reported by the tests.

## Capabilities moved

search ("Hits are addressable and typed", modified).

In the working tree on io-pimdir 5859b69 (0.18.1), no version bumped; CHANGELOG.md [Unreleased] names it.
