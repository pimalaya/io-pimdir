-- The summaries of a collection, whatever their component, so a diff over all
-- of it upserts a row only when it moved (§4.5); load_summaries_by_link reads
-- a batch's. The lists are load_values_by_collection's.
SELECT i.link_id, s.component, s.uid, s.title, s.sender, s.sender_name, s.date, s.size, s.attachment,
       s.invitation, s.card_kind, s.org, s.location, s.dtstart, s.dtstart_tzid, s.dtstart_value,
       s.dtend, s.recurring, s.until, s.due, s.due_tzid, s.due_value, s.status, s.completed,
       s.percent, s.priority, s.parent, s.mid, s.created, s.media_type, s.part
FROM items i JOIN summary s ON s.item = i.id WHERE i.collection = :collection;
