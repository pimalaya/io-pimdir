---
cairn: log
change: as-fast-as-notmuch
landed: 2026-11-21
---

# The refresh writes less

Phase 3's index refresh of as-fast-as-notmuch. The 517k Enron baseline (cebe50d) indexed in 337 s of CPU and wrote 55.6 GB for a 1.98 GB index.db, about 107 KB a message. Profiled on 40,000 Enron messages with temporary timers (perf is unavailable): of 455 µs a message, 235 µs went to FTS5 flushing its pending terms at every statement savepoint (one segment per body, merged later), 65 µs to reading each body a second time for the relink and rerunning it every 16 bodies, 30 µs to normalising ASCII, and the relink's `children` scanned the changed set per member.

Kept, CPU and bytes a message on that corpus (means of three alternating runs, bytes exact):

- Full-text writes held to the end of each `Writes` yield, in order: one segment per batch.
- Heads taken from the extraction (`thread::parent` in `Fold::read`); `rethread` with a parent-to-children map.
- An ASCII fast path in `fold::text`.
- Summary text dropped only for a placement held bodiless (F6's safe half).
- Together, at batch 256: 342 to 186 µs, write_bytes 62.6 to 64.7 KB, wchar 120.1 to 97.0 KB. Both indexes dumped whole (every row and every full-text instance, 13.7 million lines) are identical.
- Batch 4,096 by default: 17.3 KB written a message (from 64.7), 160 µs, 47 MB peak against 19 MB. With FTS merges off the bytes barely move: the amplification is the random-key B-trees dirtying a page per row per commit, written to the WAL and again at each checkpoint.
- `wal_autocheckpoint = 16384` on the indexer's connection: 15% fewer bytes at 4,096.

Measured, not kept: FTS5 `automerge` 8 (-9 to -10% bytes, a table configuration written into the index; left for a decision), `crisismerge` and `pgsz` (no effect), `automerge` 0 with a final optimize (-26% at 256, unbounded segments meanwhile), `cache_size` 32 MB (wchar halved, write_bytes unchanged, 70 MB more), parallel extraction (extraction, fold and chunking are now 10 µs of 186; FTS5 tokenising and SQLite statements dominate and stay in the writer). Left: the message reads, 36 to 38 µs a touched message; F6's other half, which needs a digest the schema lacks; `secure_delete` on index.db is a MUST of SEARCH §4 (12% at 256, nothing at 4,096).

Capabilities moved: search.md (the batch requirement, the I/O-free refresh, threads and summary text). Checks: tests/search.rs `a_batch_writes_its_text_as_one_segment` (two segments against one on interleaved writes) and the whole suite. The 517k rerun measures the import gate.
