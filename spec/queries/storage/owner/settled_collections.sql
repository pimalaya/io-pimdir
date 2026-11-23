-- The collections where a run of a source that lists nothing new derives
-- nothing, with the coverage its checkpoint serves: the source holds a
-- checkpoint and a coverage there and no round is open (else a run lists a
-- round, SYNC.md §5), no binding of it owes a push or holds a refusal
-- (bindings_owing, bindings_refused), no live item lacks it (items_lacking,
-- the delta's propagation, SYNC.md §3), and no supersession names an item
-- there (the hold of SYNC.md §5). A caller whose source states no change
-- since that checkpoint, asking that coverage's scope with no full listing,
-- may leave such a collection unsynced. Walks the source's rows of sources
-- against the collections owing anything, read once: each arm scans a
-- partial index empty at rest, the supersessions a handful of rows.
SELECT s.collection, s.covered_since, s.covered_until, s.covered_at
FROM sources s
WHERE s.source = :source
  AND s.checkpoint IS NOT NULL AND s.covered_at IS NOT NULL
  AND s.round_started_at IS NULL
  AND s.collection NOT IN (
    SELECT b.collection FROM bindings b WHERE b.source = :source AND b.owes = 1
    UNION ALL
    SELECT b.collection FROM bindings b WHERE b.source = :source AND b.refusal IS NOT NULL
    UNION ALL
    SELECT i.collection FROM items i
    WHERE i.lacks = 1
      AND NOT EXISTS (SELECT 1 FROM bindings b WHERE b.item = i.id AND b.source = :source)
    UNION ALL
    SELECT i.collection FROM supersessions x JOIN items i ON i.id = x.item OR i.id = x.by_item)
ORDER BY s.collection;
