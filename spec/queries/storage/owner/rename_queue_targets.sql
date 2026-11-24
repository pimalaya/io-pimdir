-- Runs beside rename_collection (§14): a queued move or copy names its
-- target, and a queued add the collection of the item it supersedes, in
-- target_collection, which no foreign key cascades. A parked row follows as a
-- pending one does, so a retry after the rename lands where it was meant to.
UPDATE queue SET target_collection = :new_id
WHERE target_collection = :collection;
