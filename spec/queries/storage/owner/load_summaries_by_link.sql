-- The summaries of the link ids in the `links` slot, a seek per id: what the
-- diff of one write batch reads (§14), however large the collection. The
-- lists are load_values_by_link's.
SELECT i.link_id, s.component, s.uid, s.title, s.sender, s.sender_name, s.date, s.size, s.attachment,
       s.invitation, s.card_kind, s.org, s.location, s.dtstart, s.dtstart_tzid, s.dtstart_value,
       s.dtend, s.recurring, s.until, s.due, s.due_tzid, s.due_value, s.status, s.completed,
       s.percent, s.priority, s.parent, s.mid, s.created, s.media_type, s.part
FROM items i JOIN summary s ON s.item = i.id WHERE i.collection = :collection
  AND i.link_id IN (SELECT value FROM temp.param WHERE slot = 'links');
