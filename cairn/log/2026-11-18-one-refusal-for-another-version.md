---
cairn: log
change: one-refusal-for-another-version
landed: 2026-11-18
---

# One refusal for a store of another version

pimdir audit-2026-11-12, line C1: three applications worded the refusal of a store of another version three ways, and calendula's ordered versions by parsing `storage-draft-N` on both sides, so under `storage-v1` every draft store would read as a newer engine's ("upgrade calendula").

## What landed

- src/version.rs: `PimdirFormatOrder` and `PimdirFormatRefusal { found, own, program }`, ordering by pimdir STORAGE §4.2 as it stands in the pimdir working tree (none, drafts by number, releases by major, any other marker unknown) and wording the one refusal: an earlier store drained by its engine then set aside, a later one asking to upgrade the program, an unknown one naming both ways, none advising deletion.
- src/client.rs: `PimdirError::Version` reads through it for "this program".
- Tests: src/version.rs `a_store_version_is_ordered_drafts_before_releases`, `a_draft_store_under_a_release_asks_for_its_engine`; the second states what calendula's parsing gets wrong.
- Left: a test over pimdir's vectors/versions.json once spec/ is re-vendored at draft-14; calendula, cardamum and himalaya showing it in place of their own text.

## Capabilities moved

store ("A store of another version is refused in one wording", added).

In the working tree on io-pimdir d6ca71a (0.16.0), uncommitted, no version bumped; CHANGELOG.md [Unreleased] names it.
