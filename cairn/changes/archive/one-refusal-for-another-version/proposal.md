---
cairn: change
id: one-refusal-for-another-version
status: landed
created: 2026-11-18
---

# One refusal for a store of another version

## Why

pimdir audit-2026-11-12, line C1: the refusal every user meets at v1 had three wordings, himalaya's, cardamum's and calendula's, and calendula's turns wrong then: it orders versions by parsing `storage-draft-N` on both sides, so under `storage-v1` every draft store reads as a newer engine's and says "upgrade calendula" (calendula src/pimdir/client.rs:206-245). pimdir STORAGE §4.2 now orders format versions (none, then drafts by number, then releases by major) and asks a refusal to say which is the earlier and what that asks, never to delete the store; vectors/versions.json pins the order.

## What

- src/version.rs: `PimdirFormatOrder` (`Earlier`, `Same`, `Later`, `Unknown`) and `PimdirFormatRefusal { found, own, program }`, whose `order()` follows STORAGE §4.2 and whose `Display` is the one wording, the program naming itself.
- src/client.rs: `PimdirError::Version` reads through it for "this program".
- The applications show `PimdirFormatRefusal { found, own: io_pimdir::sql::FORMAT_VERSION, program: "<app>" }` in place of their own text; their change is theirs (calendula, cardamum, himalaya).
- Left for the re-vendor of pimdir draft-14: a test running vectors/versions.json.
