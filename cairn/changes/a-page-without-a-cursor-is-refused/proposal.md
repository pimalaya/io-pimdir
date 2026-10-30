---
cairn: change
id: a-page-without-a-cursor-is-refused
status: landed
created: 2026-10-30
---

# A page without a cursor is refused

## Why

A round page answered with `last` false and no cursor is read as the round's last page (src/sync.rs:572-577, deliberate per its NOTE: "a page carrying no cursor cannot be resumed past"). Closing the round drops every unstamped binding in scope as `Deleted` (sync.rs:676-698), and the hub then pushes `Remove` to every other source: a connector bug becomes a cross-server deletion, the "partial listing read as complete" class of the data-loss lessons (report-io-pimdir.md, blocker 5).

Rekey has the same flaw (rekey.rs:499, `page.cursor.filter(|_| !page.last)`), and never checks `complete` (rekey.rs:218-231): a delta answered to its round request is taken as the whole spine, every old placement it does not list dropped as `Deleted`.

pimdir spec edit 3 (plan, 2026-10-30): `complete`, `last` and the cursor become MUSTs (SYNC.md:87, :143, :155) and an engine refuses a non-last page without a cursor.

## What

- `PimdirArgError` gains two variants, the engine's refusal of a page breaking the seam, which writes nothing of the page:
  - `UnresumablePage`: a round page neither last nor carrying a cursor, refused by the sync and by the rekey;
  - `IncompleteListing`: a page answering a rekey's round request that is not a round's (`complete` false).
- The sync and the rekey complete with that error at once; what earlier pages of the run landed stays, as for any interrupted round.
- A delta page's `last` and cursor (S12, a paged delta landing its checkpoint early) are a-listing-proves-what-it-drops's.
