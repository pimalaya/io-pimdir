---
cairn: log
change: the-crate-follows-storage-draft-08
date: 2026-10-31
---

# The vectors are vendored

Phase 0 of the-crate-follows-storage-draft-08. The suites proving the crate against the format read the pimdir checkout beside this one and skipped without it, and tests/spec_drift.rs compared spec/ with that checkout's working tree, so it went red as soon as pimdir's branch moved to storage-draft-08 (d671bbd) while the crate still follows draft-07. Nothing else moves in this phase: no source file changed.

## What landed

- spec/vectors/ copied from pimdir at 9606b43 byte for byte (`git archive`, then every file compared with `git cat-file blob`), 163 files, 888 KB; spec/REVISION holds 9606b43885c4f3b5888db64ea57ca5d5c467d632. The pin is 9606b43 because spec/migrations/ and spec/queries/ already equal pimdir there (they equal a04e377, the storage-draft-07 tag, too, its vectors identical; c20bf0a's migrations differ by a comment 9606b43 restored), and it is the revision the crate passes today.
- .gitattributes marks spec/vectors/** and spec/payloads/** `-text`, as pimdir marks its own: the fixtures are CRLF and named by their bytes.
- Cargo.toml `exclude` leaves spec/vectors/ and spec/payloads/ out of the package; spec/REVISION ships with the SQL it names (`cargo package --list`).
- tests/objects.rs, tests/summaries.rs, tests/vectors_sync.rs and tests/search.rs read spec/vectors/ and never skip: `skipped: no pimdir spec checkout beside this one` is gone from the crate.
- tests/spec_drift.rs compares spec/migrations/, spec/queries/ and spec/vectors/, both ways, with pimdir at spec/REVISION, read from the sibling checkout's git history (`git cat-file -e`, `ls-tree`, one `cat-file --batch`, all read-only), whatever its working tree is on. With no checkout holding the revision it prints `no pimdir checkout holding <rev> beside this one: nothing compared` and passes. Its statement checks read spec/ and always run.
- .github/workflows/tests.yml checks out pimdir at spec/REVISION rather than the storage-draft-07 tag, and its last step fails when spec_drift compared nothing, in place of the grep for the skip line.
- Proven failing rather than skipping, on a scratch copy: objects.json removed fails tests/objects.rs, an expected subject altered fails tests/summaries.rs, spec/vectors/sync/ moved away fails tests/vectors_sync.rs, and spec_drift names both altered files against the pinned revision. The real ../pimdir sits on d671bbd and spec_drift passes against 9606b43.
- 683 tests by default, 738 with every feature (one ignored, the 50,000-message scale test), none skipped.
- Capabilities moved: spec-fidelity ("The vendored copy is the specification's, byte for byte", modified: vectors/ and spec/REVISION; "The check degrades to a skip, never to a failure", renamed "The vectors are read from the vendored copy").

## Departures

- payloads/ and the check against vectors/manifest.json wait for phase 1: neither exists at 9606b43.
