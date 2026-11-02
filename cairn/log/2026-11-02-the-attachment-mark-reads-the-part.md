---
cairn: log
change: the-attachment-mark-reads-the-part
date: 2026-11-02
---

# The attachment mark reads the part

Follows pimdir fbd25c9 (re-vendored), which folds a new attachment mark into storage-draft-08 (pimdir cairn/log/2026-11-01-the-attachment-mark-reads-the-part.md): a part with no `Content-Disposition` makes the mark when it is neither text nor multipart and no `Content-ID` names it, an unknown disposition reads as `attachment`, and the RFC 1847 protocol parts and a multipart message's own entity never make it.

- src/summary/mail.rs: `makes_mark`, `mark_media_type` and `is_protocol_part` state the rule once; `has_attachment` takes whether the entity is a protocol part and skips a multipart message's own entity.
- src/summary/mail/attachments.rs: `PimdirAttachmentWalk` reads the same functions, each open level keeping its media type to tell protocol parts; `disposed`, which marked a multipart message disposed as an attachment, is gone, the mark being whether any part was stood for.
- The search extraction's `attachment` field reads the stand-ins (src/search/extract.rs), so it follows unchanged.
- tests/walk_agrees.rs: the `disposed` case now expects no mark; the `stray_cr` case, which used that mark to show a header block goes on past a line holding a `\r`, now shows it by a `Content-Type` and a bare PDF after that line, both the invitation and the mark depending on it.

Checks: the three new summaries cases (59 at fbd25c9) and three unit tests in src/summary/mail/attachments.rs, each through both walks. Every test passes but one that fails on master too.

Left: `property::full_and_delta_replicas_agree` (tests/engine/property.rs) found a divergence between the full-sync and the delta replica, the placement re-added after a remove holding another object, minimal input `[ServerRemove(7384027542936715245), ServerRemove(1591884363211309869), ServerRemove(262153199172116092), ServerAdd(230), ServerEdit(1255009403624172179, 0), ServerEdit(9197819837854837992, 0), LocalASetFlags(0, Known({})), SyncA, ServerRemove(11944108121488943858), ServerAdd(230), ServerEdit(15466553731663953980, 1)]` (seed `cc ec5ecf035ea89e559ba4ea822842b5b8d19fee21ad53924de7a9117205114dda`). It fails the same against fd0c060's code, so it is not this change's, and the seed is not kept in tests/proptest-regressions/property.txt.
