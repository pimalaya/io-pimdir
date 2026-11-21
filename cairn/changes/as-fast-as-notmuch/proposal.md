---
cairn: change
id: as-fast-as-notmuch
status: draft
created: 2026-11-21
---

# As fast as notmuch

## Why

pimdir's cairn/changes/compared-with-notmuch benchmarked io-pimdir (5d6419a) against notmuch 0.40 on the Enron corpus, 517,401 and 1,034,802 messages, through a Maildir connector driving the real sync engine. The owner's bar (2026-11-21): pimdir as efficient as notmuch, more being welcome and not owed.

- **Queries are 23 to 220 times slower at 517k, 24 to 300 at 1M, and cost what the store holds, not what they answer.** At 517k, `kiwi` (35 hits) counts in 198 ms against 3 ms, a `from:` address in 227 ms against 8, a page of all mail in 3,045 ms against 133, a threaded page in 13.8 s against 0.14. Timed inside the crate: the compiled statement opens with `live(akey, account, seq) AS (SELECT DISTINCT … FROM store.items … WHERE i.deleted = 0 AND i.retained_at IS NULL)` and tests every live item against every term's set (125 ms whatever the answer); `unsorted` reads every hit before a page is cut; a threaded page groups every hit; the coverage of every answer costs 69 ms at 3,311 collections: `search_temporal` runs `list_coverage` once per collection (20 ms), `COUNT_UNINDEXED` plans as a walk of `items_by_seq` over every item of the collections instead of a seek on `items_by_changed` (41 ms with the kinds' counts), and the ids are bound again into `temp.param` (3 ms). The first two are F1 and F8 of pimdir's performance-audit-2026-11-05, open since.
- **A durable import takes 2 to 3.5 times notmuch's wall time on less CPU.** 517k: 2,527 s (431 s CPU) against 719 s (547 s); 1M: 3,320 s (902 s) against 1,719 s (1,097 s); without flushing 380 s at 517k. Three causes are the standard's (pimdir cairn/changes/the-write-path-before-the-freeze, prototyped here); the index refresh is the larger CPU half (251 s at 517k, 525 s at 1M, 0.5 ms a message).
- **A sync that finds nothing costs 1 ms per collection** (3.7 s at 3,311, notmuch 0.07 s by directory mtimes); part is the bench's connector, which lists every folder.

## What

Four phases, each gated on the bench, which moves here as a standing suite:

1. **The bench.** pimdir compared-with-notmuch/bench (the Maildir connector, the tiers, the query table, the incremental and tagging runs, the fsync split) as an ignored test or an example under this crate, with a quiet-machine protocol: CPU time and bytes written per phase beside wall time, since a shared disk moves fsync-bound wall time by 2. The notmuch numbers are the reference line, rerun with each notmuch release taken.
2. **The planner** (performance-audit F1, its task on src/search/plan.rs, in the I/O-free core so the Android bridge shares it): the query's scope first; each term's set read once; the smallest set drives (an address seek, a text match, a flag, a collection or date range), the others probed per candidate; a count without materialising; a page by walking the store's order with probes when the driver is broad, so page 1 of `kind:mail` costs a page; a threaded page grouping only the threads it shows. Then coverage (F8): once per query, kept per collection until its stamp moves; `COUNT_UNINDEXED` seeking `changed > :since`; the temporal axis in one statement. Coordinated with the-query-split-by-domain, which moves the same modules.
3. **The write path.** Prototypes of the-write-path-before-the-freeze's items (a page of bodies flushed together, integer object ids, interned addresses, the transactions a page takes), measured, handed to the owner; the index refresh profiled and sped up (batch size, FTS5 merge settings, extraction in parallel as SEARCH §4 allows, the summary rewrite F6 names); the no-op sync's fixed cost per collection.
4. **What the spec adds.** an-item-keeps-local-state, the-query-reserves-its-later-forms (its `reserved-form` refusal and the open extraction status), then a-query-reads-across-domains, each implemented as pimdir lands it.

## Gates

On the 517k and 1M Enron tiers, warm cache, one core:

- every query of the bench's table (count, page of 50, threaded page of 50) no slower than notmuch's;
- no query whose cost grows with the store while its answer does not (a rare word, a one-collection scope, an address);
- a durable import no slower than `notmuch new` on a quiet machine, CPU and bytes written per message reported;
- a no-op sync within an order of notmuch's no-op `new` for the same folders.

## Decided (owner, 2026-11-21)

- **System flags read from the store's `marks`** (pimdir performance-audit-2026-11-05 F6): `is:unread`, `is:read`, `is:flagged`, `is:answered`, `is:draft` answered from the store as it stands, keywords (`tag:`) still from the index; a query's flag terms may then be fresher than its text terms when the index lags. Expected: the unread count at 517k from about 583 ms to about 45 ms (notmuch 14).
- **Integer object ids** in the store and the index (pimdir the-write-path-before-the-freeze, item 2).
