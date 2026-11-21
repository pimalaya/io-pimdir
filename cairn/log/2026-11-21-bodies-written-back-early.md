---
cairn: log
change: as-fast-as-notmuch
landed: 2026-11-21
---

# Bodies written back early

The body flushes of cairn/changes/as-fast-as-notmuch (phase 3, pimdir the-write-path-before-the-freeze item 1). A sync batch fsynced each body's temporary as it was written (`put`): on the 517k tier the sync took 572 s wall for 129 s of CPU, waiting on the disk.

- **`PimdirBodyFlush`** (src/client/blobs.rs), on `PimdirSourceStore::body_flush`, passed to `PimdirBlobs::stage`. `Each` is the old order. `Deferred { writeback }` writes the batch's temporaries, up to `DEFERRED_MAX` (256) open at once, calling `writeback` on each once written, then flushes every one and only then moves each onto its name; the shard directories are flushed as before, then the transaction opens. Per body the order STORAGE §5 asks is kept (temporary, flush, no-replace move, directory flush, commit), and no name reaches a body before every flush of its group returned, so another writer never links an unflushed body. A temporary not moved is unlinked when its group fails.
- **The default on Linux** is `Deferred` with `start_writeback`, `sync_file_range(fd, 0, 0, SYNC_FILE_RANGE_WRITE)`: the kernel starts writing each body back while the batch writes the next, so the flushes find their work mostly done. Elsewhere `Each`. `libc` is a dependency on Linux under `client` (rustix has no `sync_file_range`). The producer's `stage_body`, `PimdirBlobWriter::commit` and a single-body write still flush each body alone.
- **The bench** reads `PIMBENCH_FLUSH` (`each`, `deferred`, `writeback`), the library's default when unset.

## Proof

src/client/write.rs `every_body_is_flushed_and_moved_before_a_row_names_it` writes 600 bodies, one twice and one already held, under each mode, a probe reporting each flush and move: at every event no committed `objects` row names the body yet, each body is flushed before it is moved, the deferred group flushes 256 bodies before its first move, the hint is called 600 times or none, and after the commit every body sits under its name with its bytes and no temporary is left. It fails with `settle_temps` moving before it flushes.

## Measured

517k tier (518,401 messages, 3,312 folders), pimbench sync then index, core 12, the machine quiet, one run each in the order each, writeback, deferred (runs/flush-*/log):

| mode | sync wall | user | sys | write_bytes |
| --- | --- | --- | --- | --- |
| each | 572 s | 74.5 s | 54.7 s | 27.9 GB |
| writeback | 305 s | 65.1 s | 51.3 s | 27.9 GB |
| deferred | 512 s | 73.6 s | 57.7 s | 27.9 GB |

The bytes do not move, the waits do: the hint lets the disk work while the batch writes. One run each; a second pair follows the 1M tier.
