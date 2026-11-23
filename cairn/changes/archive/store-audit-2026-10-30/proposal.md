---
cairn: change
id: store-audit-2026-10-30
status: landed
created: 2026-10-30
---

# Store audit of 2026-10-30

## Why

The release audit (review-io-pimdir.md, should-fix S1, S4 to S6, nice-to-have N8 to N11, N14) found places where the store departs from a STORAGE MUST or states its behaviour only in this cairn. The version contract (S2's advice, S3's foreign database) and the error codes have changes of their own (the-store-version-contract, errors-carry-a-stable-code).

## What

The ledger, one task per finding, each fixed with a test or stated as a deviation in cairn/spec and README.
