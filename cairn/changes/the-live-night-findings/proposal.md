---
cairn: change
id: the-live-night-findings
status: active
created: 2026-11-24
---

# The live night findings

The live test night of 2026-11-23 (workspace .lanes/live/report.md). Its silent loss, a relocation landing on the staged flags, is fixed in 65a5026 (pimdir d5645bf, vector 214). One finding is left.

## What

**A fourth copy of one Message-ID into a collection is refused and parked**: "Pimdir binding stalwart/imap/Deleted Items/dup:dup:nv-abs-10@example.org#…#… holds handle 4 and this write carries …: a binding pins one handle". The `dup:` key minted for the fourth copy repeats the third's, so the copy can never go through (nothing is lost: parked, server untouched). Repro: himalaya copies INBOX to a folder then trashes it, repeated (neverest scratch store .lanes/live/stores/neverest-step1, queue rows #53 to #60). The minting must take the next free key, checked against every key the collection holds, and the key of a key (`dup:dup:`) should be looked at: a copy of a minted item mints from the original link id, not from the minted one.
