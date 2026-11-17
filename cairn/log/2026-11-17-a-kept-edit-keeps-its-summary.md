---
cairn: log
change: a-kept-edit-keeps-its-summary
landed: 2026-11-17
---

# A kept edit keeps its summary

CLI family audit of 2026-11-15, PIM-8, open at dcf49e2: a listing naming a member at its base revision overwrote the summary of a placement holding a staged local body, against SYNC §5 ("A placement whose local body the run keeps, pushed or conflicted, keeps that body's summary"): a contact renamed locally showed its old name until the push landed and a later listing named the new revision. A rekey carrying the edit did the same.

## What landed

- src/sync.rs: `name_members` refreshes no summary nor sort key of a placement holding a local edit of a mutable kind.
- src/rekey.rs: `carry` keeps such an edit's summary and sort key when the remote did not edit it. SYNC §8 says the sort key is carried "preferring the fetch's"; read with §5, the fetch's describes the server's body, so the edit's is kept: a wording point for draft-14.
- Test: tests/engine/summary_kept.rs (three cases, failing on dcf49e2).

## Capabilities moved

sync ("A kept local edit keeps its summary", added; it names the rekey's carry too).

In the working tree on io-pimdir dcf49e2 (0.15.1), uncommitted, no version bumped; CHANGELOG.md [Unreleased] names it.
