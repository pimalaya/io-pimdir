-- The same for one domain. A suffix match is a scan, which is why the search
-- index tokenises the domain on its own (SEARCH.md). The suffix is compared,
-- not matched as a pattern, so a `_` or a `%` in :domain stands for itself;
-- ASCII letters compare in either case, as a domain reads.
SELECT a.value AS address, a.role, i.collection, c.account, c.kind, i.seq, i.sort_key
FROM summary_value a
JOIN items i ON i.id = a.item
JOIN collections c ON c.id = i.collection
WHERE lower(substr(a.value, -length(:domain) - 1)) = '@' || lower(:domain)
  AND (:role IS NULL OR a.role = :role)
  AND a.role NOT IN ('in-reply-to', 'category')
  AND i.deleted = 0 AND i.retained_at IS NULL
ORDER BY i.sort_key DESC, i.seq DESC;
