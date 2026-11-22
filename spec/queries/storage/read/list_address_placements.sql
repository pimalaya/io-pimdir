-- Every live placement naming one address, store-wide, once per role it names
-- it in (its first position there); :role NULL for any address role. Newest
-- first, a keyset page on (sort_key, seq, collection, role), a NULL :after_key
-- the first page, coalesced to the empty blob above every text. A seek on
-- summary_value_by_value however large the store: a page reads the address's
-- placements, which the index names without reading a row, and keeps the
-- :limit newest, holding no more than those. The two roles that hold no
-- address, `in-reply-to` and `category`, are never answered.
SELECT a.role, i.collection, c.account, c.kind, i.seq, i.sort_key
FROM summary_value a
JOIN items i ON i.id = a.item
JOIN collections c ON c.id = i.collection
WHERE a.value = :address AND (:role IS NULL OR a.role = :role)
  AND a.role NOT IN ('in-reply-to', 'category')
  AND NOT EXISTS (SELECT 1 FROM summary_value d
                  WHERE d.value = a.value AND d.role = a.role AND d.item = a.item AND d.position < a.position)
  AND i.deleted = 0 AND i.retained_at IS NULL
  AND (i.sort_key, i.seq, i.collection, a.role)
      < (coalesce(:after_key, x''), coalesce(:after_seq, 0), coalesce(:after_collection, ''),
         coalesce(:after_role, ''))
ORDER BY i.sort_key DESC, i.seq DESC, i.collection DESC, a.role DESC LIMIT :limit;
