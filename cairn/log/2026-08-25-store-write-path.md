---
cairn: log
change: store-write-path
date: 2026-08-25
---

# Three whole-store costs for one-row questions

Each of these is the shape `store-algorithm-audit` fixed elsewhere, left in three places it did not reach. No behaviour changes; the whole existing suite passes untouched, which is the check that says so.

## What landed

- **A body is written before the transaction opens.** `apply_ops` wrote it between `BEGIN IMMEDIATE` and `COMMIT`, so SQLite's single writer lock spanned a file write, two `fsync`s and a rename. pimdir SPEC §14 says the opposite in as many words, and the reason is that the I/O path touches no database page: every other writer was serialising behind it for nothing. `stage_blobs` runs ahead of `write` and `write_rekeyed`; `apply_ops` keeps its own write as the floor for the queue drain, which builds its ops inside the transaction that claims its row, and the write being idempotent makes that one existence check.

- **The collector asks about the file in front of it.** It read every hash into a `BTreeSet<String>` to diff the blob tree against, which at the scale §1 promises is hundreds of thousands of 52-character names held to answer a question about one file. `OBJECT_EXISTS` is a point lookup on the primary key, and the format gained the statement. `LIST_OBJECT_HASHES` stays for the §7 diagnosis that visits every row anyway.

- **A purge releases the pins its own delete reported.** `PURGE_ITEM` and `PURGE_RETAINED_BEFORE` are `DELETE ... RETURNING object_hash, conflict_object` now, and `RETAINED_ITEM_BY_SEQ` and `RETAINED_BEFORE` are retired. The pins have to be released by whoever deletes the rows, and reading them first visited every swept row twice for an answer the delete already had; the time-based sweep is the one that takes fifty thousand at once.

- **`LIST_GARBAGE_OBJECTS` matches the canonical text** (`refcount <= 0`, not `= 0`). It has no caller, so nothing was wrong, but it is the statement a consumer reaches by name and it had stopped matching the partial index built for it.

## Verification

104 tests green with no test edited for these, `cargo clippy --all-targets --all-features` clean, `cargo fmt`. The spec-fidelity suite covers the two statement changes on both axes, and its new canonical-prepare check covers the format's side of `object_exists`.

Capabilities moved: `store`.
