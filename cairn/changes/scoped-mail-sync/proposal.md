---
cairn: change
id: scoped-mail-sync
status: landed
created: 2026-10-07
---

# A mail sync lists newest first, within a scope on the `Date`

> Cross-repo change, same id in pimdir (SYNC.md §2 to §11, STORAGE.md §2, §4.3, §6, §10, §11.3, §13, §14, Annex A.1; vectors 06, 07, 21, 25, 31 rewritten, 34 to 45 added, summaries.json `meta_attachment`), which landed first as `a2e92af`; this crate re-vendors spec/ and follows. pimdir's own proposal holds the why: MOA showing nothing for two minutes on a large Microsoft 365 box, Android unable to reach older mail, both against the same seam.

## Why

The engine enumerated handles, filed what it could not name as probes, and named them with a `Meta` upgrade: a member reached a reader only after a second round trip, and a round reached the store only once it was whole. A connector had no way to bound a mail listing by date, to commit a round page by page, or to name a member at the listing it already read its envelope in.

## What

- **Members arrive named.** `PimdirRemoteItem` carries a `PimdirRemoteMeta` (hint, summary and addresses, sort key, optionally the body DAV reads); the page's own write keys it on the identity rules an upgrade used (minting against the whole collection, a pending create landed rather than minted). `probes`, `PimdirLevel::Probed` and every probe statement go; an upsert naming no identity for a handle nothing binds is refused (`PimdirError::Unnamed`); a level of 0 reads as a `Meta` claim a `Meta` upgrade revisits.
- **Paged rounds.** An enumeration is a `PimdirEnumerate` (a `PimdirListing`, delta or round from a cursor, and a `PimdirScope`) answered by a `PimdirEnumerated`, a page or `CursorRejected`. Each page lands in one write with the round's ops (`OpenRound`, `Stamp`, `SetRoundCursor`, `CloseRound`, `SetCoverage`); deletes are inferred once, at the last page, from the in-scope bindings no page stamped; a rejected cursor restarts the round under a new id.
- **Scope and coverage.** `PimdirSyncOptions::scope` bounds a mail sync on the summary `date`, undated always in scope; absence means deleted only in scope, vanished handles apply whatever the date, a pending create with no origin out of scope waits. The run resumes, opens a round (over the band alone for a connector whose checkpoint is bound to no scope, `PimdirRemote::scope_bound`) or lists a delta, from the coverage `load` returns beside the checkpoint.
- **Readers and the owner.** `PimdirCollection::coverage`, `list_coverage`, `count_mail`, `count_mail_by_day`, `count_unread`, `list_mail_page_filtered`, `search_mail` and `like_pattern`; `PimdirStore::collect_before`; the schema reconciled on open (columns added, trigger created, `probes` dropped).
- **Annex A.1.** `summary::mail::derive_meta` and `meta_attachment`: the attachment mark without the body, which a body-less listing or fetch never writes over a walked one.
