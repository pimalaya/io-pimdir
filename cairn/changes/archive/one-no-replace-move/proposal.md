---
cairn: change
id: one-no-replace-move
status: landed
created: 2026-11-19
---

# One no-replace move

## Why

pimdir audit-2026-11-12 T7: io-pimdir's blob move was `link(2)` alone (cairn/log/2026-10-31-the-blob-store-follows-storage-draft-08.md), which an Android app's SELinux policy denies on its own files, while the Android app carried its own tested move (android rust/src/offline/blobs.rs: `renameat2(RENAME_NOREPLACE)`, then an exclusive create and copy). Two implementations of one STORAGE §5 rule, and the reference one unusable on a phone.

## What

- `client::blobs::PimdirBlobMove { renameat2 }` with `run(from, to)`, answering `PimdirBlobFiled::{Moved, Held}`: `renameat2` with `RENAME_NOREPLACE` through rustix (a raw system call on Linux and Android, `renameatx_np` with `RENAME_EXCL` on Apple); where unavailable or refused, `link(2)` then `unlink(2)`; on Unix where the link is denied or unsupported, the exclusive create and copy, ported from the Android app with its race handling (a name moved away while copying is filed again, three times). `renameat2: false` skips the rename where a system call filter ends the process on it (Android before API 30), as the app's `renameat2_allowed` does.
- The store's writes (`PimdirBlobs::write`, `stage`, `PimdirBlobWriter::commit`) and `store backup` / `restore` move through it. A name held is still verified by the caller (§5).
- The Android app can drop its move (`rename_noreplace_on`, `file`, `claimed`, `copy`, `holds`, `unsupported`, `renameat2`) for `PimdirBlobMove { renameat2 }.run`, keeping its `checked` verification or reading `PimdirBlobs::verified`. That is the app's change, not made here.
