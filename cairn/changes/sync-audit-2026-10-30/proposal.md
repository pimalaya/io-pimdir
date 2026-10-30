---
cairn: change
id: sync-audit-2026-10-30
status: landed
created: 2026-10-30
---

# Sync audit of 2026-10-30

## Why

The release audit (review-io-pimdir.md, should-fix S7 to S16) found, by code reading, places where the engine may lose or misreport an edit. None has a failing test yet: per the testing rules each needs one against 4c3353e before a fix, and a finding no test reproduces is closed as unproven, not fixed blindly. The blockers of the same audit have changes of their own (a-page-without-a-cursor-is-refused, the-agreed-body-stays-pinned, a-listing-proves-what-it-drops for S12, the-agreed-body-stays-pinned for S14).

## What

The ledger, one task per finding: reproduce, then fix and add a vector where the spec covers it, or close as unproven.
