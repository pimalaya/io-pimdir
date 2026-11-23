---
cairn: tasks
change: one-no-replace-move
---

- [x] `PimdirBlobMove` and `PimdirBlobFiled` in src/client/blobs.rs, the store's writes and the backup through it
- [x] Unit tests: a free name takes the source on both paths, a held name or link never replaced, a denied link files by an exclusive copy (`0600`, every byte, source gone), an unavailable rename links, racing copies file once, a missing source claims no name
- [x] cairn/spec/store.md folded, log, CHANGELOG
- [ ] The Android app moves through `PimdirBlobMove` and drops its copy (android's to do)
