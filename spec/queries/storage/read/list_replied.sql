-- Which of the message ids the `ids` slot names (bare ids, as Annex A.1
-- stores a message's In-Reply-To and A.4 a task's mid) some live mail replies
-- to, its `in-reply-to` values naming the id: the mail a task came from
-- answered. Mail filed under the sent or drafts role is the person's own and
-- answers nothing. Each id answered once, in code point order. A seek per id
-- on summary_value_by_value.
SELECT DISTINCT r.value AS message_id
FROM summary_value r
JOIN items i ON i.id = r.item
JOIN collections c ON c.id = i.collection
WHERE i.deleted = 0
  AND (c.role IS NULL OR c.role NOT IN ('sent', 'drafts'))
  AND r.role = 'in-reply-to'
  AND r.value IN (SELECT value FROM temp.param WHERE slot = 'ids')
ORDER BY r.value;
