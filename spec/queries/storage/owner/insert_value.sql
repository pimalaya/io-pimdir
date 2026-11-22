INSERT INTO summary_value(item, role, position, value, name)
VALUES((SELECT id FROM items WHERE collection = :collection AND link_id = :link_id), :role, :position, :value, :name);
