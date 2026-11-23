---
cairn: change
id: stand-ins-as-the-body-enters
status: landed
created: 2026-10-13
---

# A body entering the store stands for its attachments

## Why

pimdir's file kind (STORAGE §14.3) stands for a message's attachments with files holding no body, referenced from the message. The Android app records them itself, in Java, after storing a body (`FileStore.recordAttachments`). Every other writer goes through io-pimdir: neverest lands bodies by the sync engine, himalaya's drafts arrive by the queue, and none of them records stand-ins, so a store they write has no files and `list_attachments`, `list_attachments_by_account` and `search_files` answer nothing. A rule each writer must remember is a rule some writer forgets; the write path every writer shares is where it belongs.

The links are made at instant T, when a body enters the store, and never recomputed: no pass over the bodies a store already holds, and a body filed again (a move, a copy, a second Gmail label) walks nothing.

## What

- `summary::mail::attachments`: the walk of a message's parts fed a chunk at a time (`PimdirAttachmentWalk`, `attachments` for a body in memory), answering one `PimdirFileSummary` per part carrying `Content-Disposition: attachment`, its IMAP section, its name (RFC 2231 and RFC 2047 decoded, the disposition's `filename` before the type's `name`), its media type (`None` when unstated) and its decoded size (`None` for a multipart part), the walk agreeing with the attachment mark's on which parts count. Memory stays bounded whatever the body: `HEADER_MAX` per header block, `LINE_HEAD` per other line, one boundary per open level down to `MIME_DEPTH_MAX`, at most `ATTACHMENTS_MAX` (1000) summaries; a single-part message is done once its header is read.
- `client::write::apply` runs it for every collection of `message/rfc822`: the bodies the diff brings into a live, stored item under a key naming an identity, less those `lookup_objects` finds already held under the key in the account before the diff, are read from their blob files after the diff is saved, in the batch's transaction. Each part not held yet gets a stand-in at `Meta` (`insert_item` with the key's shared `seq`, `upsert_file_summary`), then `add_reference` from the message, `attachment`, `auto`. The account's attachments collection is the file collection holding the role `attachments`, else `<account>/\u0001attachments` (`\u0001attachments` with no account), declared with that kind, the name `attachments` and the role, as the Android app names it.
- Every writer gets it with no call of its own: a sync landing a fetch, an upgrade, a drained `add`, a direct `write`.
- pimdir: STORAGE §14.3 states when stand-ins are recorded and how the collection is found and named, Annex A.7 the size and name precisely; summaries.json carries each message's `attachments`, and sync vector 70 a body entering with three.
