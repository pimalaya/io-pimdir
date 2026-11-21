---
cairn: change
id: a-decryption-break
status: draft
created: 2026-11-21
---

# A decryption break

## Why

The owner (2026-11-21): decrypting bodies for the index is not planned now, but will come. SEARCH §6 already allows it ("decrypted content is indexed only when the user asks"). What must not break then is the format: stores, indexes and the clients reading them. pimdir prepares that side (the-query-reserves-its-later-forms declares the extraction status open, an-item-keeps-local-state reserves `pimdir/session-key`). This crate's API may break when the feature lands (owner, 2026-11-21: a breaking change of the coroutine is fine, no `#[non_exhaustive]`), so nothing is owed in the code now; this change records the shape so the feature lands in one place.

## What, when decryption is planned

- **A break in the refresh.** The extraction yields when it meets a `multipart/encrypted` or `application/pkcs7-mime` part, `WantsDecryption { hash, part, protocol }`, through `PimdirIndexRefreshYield` (the coroutine-over-trait rule: a pluggable behaviour is an I/O-free break, never a strategy trait). A runner answers `Plaintext(bytes)` from sequoia or gpg-agent, or `Declined`, which records `encrypted` as today; the crate holds no key and does no cryptography.
- **A `decrypted` status** in `PimdirExtractStatus` and its coverage count in `PimdirSearchUnread`, the session key kept under `pimdir/session-key`, and the `index_meta` column saying the index holds plaintext, each as pimdir specifies them.
- **The Android bridge's runner** answers the new yield in the same release.
