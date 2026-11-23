---
cairn: log
change: as-fast-as-notmuch
landed: 2026-11-23
---

# The hit columns patched, the no-op sync writes nothing

A refresh that finds index.hits stamped as the index stood before it now patches the cache where the placements it fed or wrote moved, rather than reading every placement joined to the store's items again: on the 517k Enron store the cache step of a refresh went from about 290 ms to about 12 ms, the file byte for byte what a whole read gives (tests/search_plan.rs `a_patched_cache_is_the_one_read_whole`). Refresh medians, base against patched: 300 new messages 617 to 189 ms, 300 removed 582 to 220 ms, 1,000 flag changes 368 to 84 ms (notmuch new takes 725 ms for the 300). `ensure_collection` reads the row first and writes nothing when it already holds the kind: the bench's no-op sync of 3,312 folders 124 to 107 ms (notmuch new 52 ms). Capabilities moved: search (spec/search.md, the hit columns requirement), store (spec/store.md, collection kind).
