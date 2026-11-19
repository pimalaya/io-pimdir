---
cairn: change
id: a-page-lands-in-few-statements
status: abandoned
created: 2026-11-04
---

# A page lands in few statements

> Cross-repo. The measurement is the Android app's (android/app/src/test, its storage android/app/src/main/java/org/pimalaya/pimdir/PimdirStorage.java), since its path is the slow one. The stamp change is canonical, so it lands in pimdir first (migrations/storage, queries/storage, STORAGE §4.5) under the same id; this crate re-vendors spec/ and follows, and the app runs the new statements by name.

## Why

Writing a listed page costs about 8 ms per message on a phone, whatever the provider, and the store takes one write at a time, so it is the floor under every first sync and every fill. Measured on a OnePlus A5000 (Android 15, platform SQLite 3.44) on 2026-11-04, from the app's debug log:

| Run | Messages | Store write | Per message |
|---|---|---|---|
| Fastmail (JMAP), first step, near-empty store | 25, 10, 1 | 204, 97, 17 ms | 8 to 10 ms |
| Microsoft (Graph), fill, 14 mailboxes | 3,792 | about 32 s of a 78 s fill | 6.9 to 9.3 ms |
| Posteo (IMAP), fill, 15 mailboxes | 8,466 | 99.5 s of a 165 s fill | about 12 ms |

The cost is the write itself. On the Microsoft run the load before each page took 0.3 s in all, and the 14 slow-database warnings all fell at the end of those writes, held by the sync workers: no reader and no search index held the store. A page is already one transaction (store.md, *A write batch is one transaction*); what costs is what each message does inside it:

- **The app's path** (PimdirStorage.applyUpsert) runs 10 to 15 statements per upsert: the link lookup, the mail check, the held item, its level, the shared object, the merge, the sequence, `INSERT_ITEM` or `RESTATE_ITEM`, the flags, the conflict, the summary rows, the binding and the refcount. Reads go through statements compiled once per batch (`PimdirStorage.one`), writes through `PimdirDb.changed`, which compiles each time and leans on the connection's statement cache (`MAX_SQL_CACHE_SIZE`).- **The schema's triggers multiply the rows each insert touches** (migrations/storage/0001_init.sql). `items_stamp_insert` updates the row it just inserted and `store_meta`; each `placement_flag` insert sets `items.changed = -1`, which `items_stamp_request` answers with two more updates. A message landing with `\Seen` and `\Flagged` updates its own `items` row five times, `store_meta` three times, draws three stamps, and moves its `items_by_changed` entry each time. Every one of those runs on SQLite 3.18 as on 3.44: triggers fire per row whatever statement inserts it.
- **This crate's path** (`client::write::apply`) is not the one measured slow; it shares the triggers, so it gains from the same fix.

## Measured on the desktop (2026-11-05)

android/app/src/test/java/org/pimalaya/mail/FillPageMeasure.java, run with `PIMALAYA_MEASURE=1`: six pages of 500 new messages (`\Seen` and `\Flagged`, a summary with a sender and a recipient) into one mailbox of a fresh store, on the host SQLite 3.18.2 the unit tests run. Desktop times compare the variants; they are not the phone's.

| Variant | First page | Pages 2 to 6 | Last page, in statements |
|---|---|---|---|
| Schema as it is | 642 ms | 458, 394, 371, 365, 355 ms | 345 of 355 ms |
| Stamp triggers dropped | 315 ms | 277, 284, 271, 272, 277 ms | 267 of 277 ms |

The last page, by statement name (a name running two steps counts each):

| Statement | Runs | As is | Unstamped |
|---|---|---|---|
| `LINK_SENDERS_OF` | 500 | 61.6 ms | 58.4 ms |
| `INSERT_ITEM` | 500 | 39.9 ms | 13.3 ms |
| `ADD_FLAGS` | 500 | 35.8 ms | 16.0 ms |
| `WRITE_BINDING` | 1000 | 28.9 ms | 28.6 ms |
| `SET_FLAGS_KNOWN` | 500 | 28.6 ms | 14.7 ms |
| list binding (`temp.param`) | 2000 | 25.6 ms | 25.2 ms |
| `UPSERT_MAIL_SUMMARY` | 1000 | 19.5 ms | 18.9 ms |
| `ADD_BASE_FLAGS` | 500 | 15.8 ms | 15.7 ms |
| `RETAIN_FLAGS` | 500 | 15.3 ms | 9.4 ms |
| `LINK_FOR_HANDLE` | 500 | 15.0 ms | 11.9 ms |
| the other eleven | 6,002 | 59.5 ms | 55.3 ms |

What it says:

- **Statements are the page.** 97% of the write is inside statements; the Java around them (JSON, binding, the merge) is 3%.
- **The stamp chain is about a fifth of a steady page** (355 against 277 ms), half the first. It sits where the proposal said: `INSERT_ITEM`, `ADD_FLAGS`, `SET_FLAGS_KNOWN`, `RETAIN_FLAGS`. Step 2 alone would leave a phone page near 3 s, not 0.5 s, if the phone's ratio is the desktop's.
- **The rest is breadth, not one culprit.** About 27 statement runs per message at about 25 µs each, the costliest under a fifth: per-message statement count matters, against what this proposal guessed before measuring. `LINK_SENDERS_OF` (the sender reference rule, STORAGE §14.2) is the single largest at 120 µs a run, in a store with no contact to link to; the list binding through `temp.param` (`create_param`, `clear_param`, then one insert per value) is 2000 runs for a page.

The device run decides the rest: the same profile from the slow-write log on the phone says whether the ratios hold.

## Measured on the phone (2026-11-05)

The debug build on the OnePlus A5000, a fill: a page of 412 upserts in 3,247 ms (7.9 ms each), 2,805 ms in statements. The list binding was the largest at 481 ms for 1,649 binds, 292 µs each against 13 µs on the host, then `LINK_SENDERS_OF` (337 ms), `WRITE_BINDING` (275 ms) and `INSERT_ITEM` (265 ms); every statement ran 6 to 11 times its host time.

## The list binding (2026-11-05)

`PimdirDb.params` ran `create_param` before every bind. It is DDL, which the platform's statement cache never holds (API 35 `SQLiteConnection.isCacheable` keeps SELECT, update and WITH statements), and it slowed every statement of the connection, not itself alone. With the create run only when `clear_param` fails, and retain and add flags sharing one bind (`PimdirDb.changedSharing`), the host page of 500 went from 355 to 106 ms (277 to 87 ms with the stamp triggers dropped), `LINK_SENDERS_OF` from 61.6 to 5.5 ms, `INSERT_ITEM` from 39.9 to 14.3 ms. Why a no-op create slows the other statements is not established. The phone's numbers after the fix decide whether steps 2 and 3 are still needed.

## What

1. **Measure the app's path.** A Robolectric test in the app, ignored by default, writes one page of 500 new mail upserts (two flags and a summary with addresses each) through `PimdirStorage.applyWrites` into a fresh store, on the host build of SQLite 3.18 the tests already run. It times the page twice: with the schema as it is, and with the stamp triggers dropped (`items_stamp_insert`, `items_stamp_update`, `items_stamp_request`, `placement_flag_stamp_insert`). A debug-only profile sums the time per canonical statement name inside a batch, which the test prints and the app's slow-write log line (`store write … in N ms`) carries, for one device fill on the Microsoft test account. The desktop numbers compare variants; only the device numbers are absolute.
2. **Stamp each item once per batch** (a fifth of a steady page on the desktop, half the first). Inside one transaction a reader never sees a stamp between two writes, so one stamp per item per batch keeps every guarantee of STORAGE §4.5: unique, increasing across commits, a page ordered by `changed` total. The opt-in shape: a writer inserting an item writes `changed = -2` (pending); `items_stamp_insert`, `items_stamp_update` and `placement_flag_stamp_insert` skip a pending row; the batch ends with one `stamp_pending` statement (`UPDATE items SET changed = -1 WHERE changed = -2`), which the request trigger answers once per row. A message with two flags then updates its row twice and `store_meta` once. A writer that never writes `-2` keeps today's behaviour. A pending row at rest is a fault a check can count (`items_by_changed` seeks `-2`), so a batch that forgets the closing statement is caught, never silent. That is why the opt-in is per row and not a batch-wide floor: a floor a writer forgets to reset would skip restamps in later transactions, a loss nothing at rest shows.
3. **Fewer statements per message** (the desktop says step 2 alone misses the target; the device run confirms or not). Per-page statements over the page's members, bound as lists through `temp.param` (STORAGE §4.4), which SQLite 3.18 runs; `json_each` does not exist on the platform SQLite of API 26 to 33 (3.18 on API 26, minSdk 26). The candidates, costliest first: the sender rule run once per page over its links instead of once per message; the list binding made cheaper than four statements per list; one lookup by link for the members the store does not hold, replacing the per-message link, sequence and held-item reads in `applyUpsert`, as `batch_links` does here. Triggers still fire per row whatever statement inserts it, which step 2 handles.
4. **The app follows** by statement name, so the CLI and the app still write the same rows (the storage vectors).

The target: a page of 500 new messages written in under 0.5 s on the phone (about 1 ms each), against about 4 s today. A target, not a requirement: the delta states only what a reader observes.

## Open points

- Whether the pending sentinel takes `-2` or a column of its own. `-2` costs no migration and `items_by_changed` already indexes it; STORAGE §4.5 and §17 then name two reserved values instead of one.
- Whether a stale `-2` should abort the commit (a trigger cannot see a commit; the runner would check before `setTransactionSuccessful`) or only fail the checks.
- Whether the search index's refresh after each write batch (the app's `PimdirIndexDb.refreshSoon`) should wait for a fill to pause. Step 1's device run says what it costs; changing it is the app's own change.

## Closed (2026-11-19)

Closed without landing on the owner's ruling of 2026-11-19 (pimdir the-lines-before-v1): the pending stamp of step 2 left the format (pimdir cairn/log/2026-11-05-storage-draft-09.md), the owner closed write speed as good enough after the phone fill of 2026-11-13 (30 s, 4.2 ms a message) and asked that the write path not be re-proposed, so steps 2 and 3 are dropped and the device numbers are not owed. What landed of it landed in the Android app (step 1's measurement and the list-binding fix of step 3); the delta was never folded.
