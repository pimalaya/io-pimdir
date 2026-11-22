-- A calendar resource whose body derives no summary any more loses its row;
-- a mail, contact or file row never leaves its item, the collection's kind
-- being fixed. Its lists are replace_values's.
DELETE FROM summary WHERE item = (SELECT id FROM items WHERE collection = :collection AND link_id = :link_id);
