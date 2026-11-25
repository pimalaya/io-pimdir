-- The retained row holding a link id, if any: its public id, the objects
-- it pins, which revive releases and purge reclaims (§11), and its sort key,
-- which the body a revive sets aside keeps (§11.1).
SELECT seq, (SELECT o.hash FROM objects o WHERE o.id = items.object) AS object_hash, (SELECT o.hash FROM objects o WHERE o.id = items.conflict_object) AS conflict_object, sort_key FROM items
WHERE collection = :collection AND link_id = :link_id AND retained_at IS NOT NULL;
