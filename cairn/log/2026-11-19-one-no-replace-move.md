---
cairn: log
change: one-no-replace-move
landed: 2026-11-19
---

# One no-replace move

The blob move is `PimdirBlobMove` (src/client/blobs.rs): `renameat2` with `RENAME_NOREPLACE` through rustix (`renameatx_np` with `RENAME_EXCL` on Apple), then `link(2)` where the rename is unavailable or refused, then on Unix the exclusive create and copy where the link is denied, ported from the Android app's tested move (android rust/src/offline/blobs.rs) with its race handling. `renameat2: false` skips the rename where a system call filter would end the process (Android before API 30). Every store write and `store backup` / `restore` move through it; a held name is verified by the caller as before. Answers pimdir audit-2026-11-12 T7 on io-pimdir's side; the Android app's switch to it is left to the app.

Capabilities moved: store ("A blob write never replaces a file").

Tests: six unit tests in src/client/blobs.rs, the denied link and the unavailable rename injected. The denial itself (SELinux on a device) cannot be reproduced on the host, so no test fails against the old `link(2)`-only code by the denial; the move's behaviour is the Android app's, whose tests these port. `cargo test --all-features` passes.
