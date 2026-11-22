-- An item's summary (Annex A), whatever its component: every column is
-- written, a column another component owns as NULL, so a calendar resource
-- rewritten as another component is this one statement. An unknown
-- invitation (NULL) never clears a known one while the row stays a mail's:
-- the message is immutable, so what its body once said it still says
-- (Annex A.1).
INSERT INTO summary(item, component, uid, title, sender, sender_name, date, size, attachment,
                    invitation, card_kind, org, location, dtstart, dtstart_tzid, dtstart_value,
                    dtend, recurring, until, due, due_tzid, due_value, status, completed,
                    percent, priority, parent, mid, created, media_type, part)
VALUES((SELECT id FROM items WHERE collection = :collection AND link_id = :link_id), :component,
       :uid, :title, :sender, :sender_name, :date, :size, :attachment,
       :invitation, :card_kind, :org, :location, :dtstart, :dtstart_tzid, :dtstart_value,
       :dtend, :recurring, :until, :due, :due_tzid, :due_value, :status, :completed,
       :percent, :priority, :parent, :mid, :created, :media_type, :part)
ON CONFLICT(item) DO UPDATE SET
    component = excluded.component, uid = excluded.uid, title = excluded.title,
    sender = excluded.sender, sender_name = excluded.sender_name, date = excluded.date,
    size = excluded.size, attachment = excluded.attachment,
    invitation = CASE WHEN component = 'mail' AND excluded.component = 'mail'
                      THEN coalesce(excluded.invitation, invitation)
                      ELSE excluded.invitation END,
    card_kind = excluded.card_kind, org = excluded.org, location = excluded.location,
    dtstart = excluded.dtstart, dtstart_tzid = excluded.dtstart_tzid,
    dtstart_value = excluded.dtstart_value, dtend = excluded.dtend,
    recurring = excluded.recurring, until = excluded.until,
    due = excluded.due, due_tzid = excluded.due_tzid, due_value = excluded.due_value,
    status = excluded.status, completed = excluded.completed, percent = excluded.percent,
    priority = excluded.priority, parent = excluded.parent, mid = excluded.mid,
    created = excluded.created, media_type = excluded.media_type, part = excluded.part;
