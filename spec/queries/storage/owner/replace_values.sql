-- An item's lists are replaced as a set: the derivation yields each whole,
-- and a diff buys nothing on a handful of rows. Every role with :lists 1;
-- with :lists 0 the addresses alone, the In-Reply-To ids and the categories
-- staying with a row that stays. Followed by one insert_value per value, and
-- by stamp_item when nothing else about the item moved (§4.5).
DELETE FROM summary_value
WHERE item = (SELECT id FROM items WHERE collection = :collection AND link_id = :link_id)
  AND (:lists = 1 OR role NOT IN ('in-reply-to', 'category'));
