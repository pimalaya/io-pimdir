-- Whether a queue row, pending or parked, names a collection (§14): its
-- delete is refused before it runs, the foreign key's ON DELETE RESTRICT
-- refusing it as well, untyped, should the check be skipped.
SELECT EXISTS (SELECT 1 FROM queue WHERE collection = :collection) AS holds;
