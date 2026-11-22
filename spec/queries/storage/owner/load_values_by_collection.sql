-- The lists of a collection's items, in document order within each role: the
-- diff's read over all of it (§4.5), load_values_by_link's for a batch.
SELECT i.link_id, v.role, v.position, v.value, v.name
FROM items i JOIN summary_value v ON v.item = i.id
WHERE i.collection = :collection
ORDER BY i.link_id, v.role, v.position;
