-- One item's lists (Annex A.1, A.4, A.6): its people, its In-Reply-To ids and
-- its categories, in document order within each role.
SELECT role, position, value, name FROM summary_value
WHERE item = (SELECT id FROM items WHERE collection = :collection AND link_id = :link_id)
ORDER BY role, position;
