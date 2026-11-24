-- The keys of a collection minted from the link ids of the `links` slot, a
-- retained row aside: each other member of the collection stating the same
-- hint (§9), which a Handles load reads beside a member under the bare hint so
-- an upgrade never links a body by agreement into such a collection (SYNC.md
-- §6). A key minted from k begins with `dup:`, k and `#`, and sorts strictly
-- between that prefix and `dup:`, k and `$`, `$` following `#`, so each link
-- id is one range seek on the (collection, link_id) index.
SELECT i.link_id FROM temp.param p
JOIN items i ON i.collection = :collection
  AND i.link_id > 'dup:' || p.value || '#'
  AND i.link_id < 'dup:' || p.value || '$'
WHERE p.slot = 'links' AND i.retained_at IS NULL;
