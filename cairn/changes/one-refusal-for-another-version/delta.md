---
cairn: delta
change: one-refusal-for-another-version
---

## ADDED Requirements

### Requirement: A store of another version is refused in one wording
`version::PimdirFormatRefusal { found, own, program }` SHALL order the store's `format_version` against the program's by STORAGE §4.2 (`order()`, a `PimdirFormatOrder`): none first, then `storage-draft-` followed by decimal digits by their number, then `storage-v` followed by digits without a leading zero by their major; any other marker on either side is `Unknown`. Its `Display` SHALL be the one refusal every program shows: for an earlier store, an earlier sync engine's, drained by it until nothing is queued, then moved aside and synced anew; for a later one, "upgrade" the program; for an unknown one, both ways named; never advising to delete the store. `PimdirError::Version` SHALL read through it for "this program", and an application names itself through it.

#### Scenario: A draft store under a release
- src/version.rs `a_store_version_is_ordered_drafts_before_releases` and `a_draft_store_under_a_release_asks_for_its_engine`: `storage-draft-13` under `storage-v1` is earlier and never "upgrade calendula", `storage-v2` under `storage-v1` asks to upgrade, `storage-v01` is unknown, and no wording says delete

## MODIFIED Requirements

## REMOVED Requirements
