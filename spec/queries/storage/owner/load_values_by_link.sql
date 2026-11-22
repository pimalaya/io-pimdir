-- The lists of the link ids in the `links` slot, in document order within
-- each role, a seek per id: what the diff of one write batch, or a page,
-- reads (§14); load_values_by_collection reads all of a collection's.
SELECT i.link_id, v.role, v.position, v.value, v.name
FROM items i JOIN summary_value v ON v.item = i.id
WHERE i.collection = :collection
  AND i.link_id IN (SELECT value FROM temp.param WHERE slot = 'links')
ORDER BY i.link_id, v.role, v.position;
