---
cairn: log
change: as-fast-as-notmuch
landed: 2026-11-21
---

# A page lands in one transaction

Phase 3's item 4 of as-fast-as-notmuch (pimdir the-write-path-before-the-freeze, item 4, measured in its prototypes-2026-11-21.md). `PimdirSync::WRITE_CHUNK` goes from 1,024 to 4,096 writes: a new member is two writes (its body, its placement), so a page of 1,000 new members split in two batches, two transactions, where pimdir GUIDE §10 writes a page in one. On 7,932 Enron messages this took 65 transactions for 57 pages, and one per page cut write_bytes by 9%; at 517k, 3,925 transactions become 3,535. No spec change: nothing in STORAGE or SYNC forced the second transaction. The batch holds no memory the page's listing does not hold already.

Capabilities moved: sync.md (the write-chunk requirement states the value and why). Checks: the 121 `sync::` unit tests, `a_full_write_batch_is_handed_over_mid_merge` among them at the new size.
