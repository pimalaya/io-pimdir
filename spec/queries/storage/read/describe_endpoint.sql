-- A reference's endpoint (§14.2) as a link shows it: its first live
-- placement under the kind, by collection id, and the title of its summary:
-- a subject, a name, a summary, a file's name. No row when no live placement
-- holds it.
SELECT i.collection, i.seq, s.title
FROM items i
JOIN collections c ON c.id = i.collection AND c.kind = :kind
LEFT JOIN summary s ON s.item = i.id
WHERE i.link_id = :link_id AND i.deleted = 0
ORDER BY i.collection LIMIT 1;
