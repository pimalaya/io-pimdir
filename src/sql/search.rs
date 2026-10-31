//! # Search SQL
//!
//! The index schema and statements of SEARCH.md, generated at build time
//! from the specification vendored under spec/ (migrations/search/ and
//! queries/search/), each prepared on index.db with the store attached
//! read-only as `store` (SEARCH §2).
//!
//! The statements of this crate's own follow: the indexer's bookkeeping
//! around the canonical ones, none of which another implementation needs
//! to run the same index.

include!(concat!(env!("OUT_DIR"), "/search.rs"));

/// The FTS5 `tokenize` string in force (SEARCH §5), recorded in
/// `index_meta.tokenizer`: an index recording another is rebuilt. It folds
/// no diacritic, the text reaching it folded already.
pub const TOKENIZER: &str = "unicode61 remove_diacritics 0";

/// Whether a collection was stamped above `index_meta.store_reconciled`,
/// the last completed refresh's cursor: a rename leaves the old id's
/// placements to the key reconciliation (SEARCH §4).
pub const COLLECTIONS_MOVED: &str =
    r#"SELECT EXISTS (SELECT 1 FROM store.collections WHERE changed > :since);"#;

/// The live placements the index holds without the body the store holds:
/// a blob the indexer could not read when it folded the row, met again on
/// the next pass (SEARCH §4). Files are indexed from their summary alone,
/// so they never wait. The columns are `load_changed_items`'.
pub const LOAD_UNREAD_BODIES: &str = r#"SELECT i.collection, i.link_id, i.seq, i.object_hash,
       CASE WHEN i.flags_known = 1
            THEN coalesce((SELECT group_concat(f.flag, ' ') FROM store.placement_flag f
                           WHERE f.collection = i.collection AND f.link_id = i.link_id), '') END AS flags,
       i.deleted, i.retained_at, c.kind, c.account, i.changed
FROM placement p
JOIN store.items i ON i.collection = p.collection AND i.seq = p.seq
JOIN store.collections c ON c.id = i.collection
WHERE p.hash IS NULL AND i.object_hash IS NOT NULL AND i.deleted = 0
  AND c.kind != 'application/octet-stream'
LIMIT :limit;"#;

/// The live items of the collections of the `collections` slot (STORAGE
/// §4.4) stamped above the index cursor: what an answer may see stale or
/// not at all.
pub const COUNT_UNINDEXED: &str = r#"SELECT count(*) FROM store.items
WHERE changed > :since AND deleted = 0 AND retained_at IS NULL
  AND collection IN (SELECT value FROM temp.param WHERE slot = 'collections');"#;

/// Every live placement of one hit, `(account, seq)` (SEARCH §8.2), with
/// its kind's component and title, in collection order.
pub const LOAD_HIT_PLACEMENTS: &str = r#"SELECT i.collection, i.link_id,
       CASE WHEN i.flags_known = 1
            THEN coalesce((SELECT group_concat(f.flag, ' ') FROM store.placement_flag f
                           WHERE f.collection = i.collection AND f.link_id = i.link_id), '') END AS flags,
       i.object_hash, i.sort_key, i.level, c.kind,
       CASE c.kind
         WHEN 'message/rfc822' THEN 'mail'
         WHEN 'text/vcard' THEN 'contact'
         WHEN 'application/octet-stream' THEN 'file'
         WHEN 'text/calendar' THEN CASE
           WHEN e.link_id IS NOT NULL THEN 'event'
           WHEN t.link_id IS NOT NULL THEN 'task'
           WHEN j.link_id IS NOT NULL THEN 'journal'
         END
       END,
       coalesce(m.subject, k.fn, e.summary, t.summary, j.summary, f.name, '')
FROM store.items i
JOIN store.collections c ON c.id = i.collection
LEFT JOIN store.mail_summary m ON m.collection = i.collection AND m.link_id = i.link_id
LEFT JOIN store.contact_summary k ON k.collection = i.collection AND k.link_id = i.link_id
LEFT JOIN store.event_summary e ON e.collection = i.collection AND e.link_id = i.link_id
LEFT JOIN store.task_summary t ON t.collection = i.collection AND t.link_id = i.link_id
LEFT JOIN store.journal_summary j ON j.collection = i.collection AND j.link_id = i.link_id
LEFT JOIN store.file_summary f ON f.collection = i.collection AND f.link_id = i.link_id
WHERE i.seq = :seq AND c.account IS :account AND i.deleted = 0 AND i.retained_at IS NULL
ORDER BY i.collection;"#;

/// The items an `id:` value names, every kind each is held under (SEARCH
/// §8): the item of `seq`, `NULL` for a value not of digits, and every
/// item of the link id, their live placements' kinds and keys, so
/// `linked` is asked once per pair.
pub const LOAD_ENDS: &str = r#"SELECT DISTINCT c.kind, i.link_id FROM store.items i
JOIN store.collections c ON c.id = i.collection
WHERE (i.seq = :seq OR i.link_id = :link_id)
  AND i.deleted = 0 AND i.retained_at IS NULL;"#;

/// One placement row as the index holds it: its id, the rowid its summary
/// text is keyed on, its link id and the body indexed for it, `NULL` while
/// bodiless (SEARCH §3).
pub const LOAD_PLACEMENT: &str = r#"SELECT id, link_id, hash FROM placement
WHERE collection = :collection AND seq = :seq;"#;

/// One item's `In-Reply-To` ids in document order (STORAGE Annex A.1), the
/// list a feed row's mail summary carries.
pub const LOAD_ITEM_IN_REPLY_TO: &str = r#"SELECT msg_id FROM store.mail_in_reply_to
WHERE collection = :collection AND link_id = :link_id ORDER BY position;"#;

/// One item's categories in document order (STORAGE Annex A.4), the list a
/// feed row's task summary carries.
pub const LOAD_ITEM_CATEGORIES: &str = r#"SELECT value FROM store.task_category
WHERE collection = :collection AND link_id = :link_id ORDER BY position;"#;

/// Every category a task states, each once: what `category:` folds in
/// Rust and compares with its folded value (SEARCH §8), so no SQL function
/// folds the stored side.
pub const TASK_CATEGORY_VALUES: &str = r#"SELECT DISTINCT value FROM store.task_category;"#;

/// The mail placements the index holds for one message id, `count` zero
/// for a message no placement holds: how many, the lowest sort key, the
/// lowest and highest one naming a date (SEARCH §9).
pub const MESSAGE_PRESENCE: &str = r#"SELECT count(*), min(i.sort_key), min(nullif(i.sort_key, '')), max(nullif(i.sort_key, ''))
FROM placement p
JOIN store.collections c ON c.id = p.collection
JOIN store.items i ON i.collection = p.collection AND i.seq = p.seq
WHERE p.link_id = :link_id AND c.kind = 'message/rfc822';"#;

/// A body the index holds for one message id, the source of its
/// `In-Reply-To` and `References` (SEARCH §9).
pub const MESSAGE_HASH: &str = r#"SELECT p.hash FROM placement p
JOIN store.collections c ON c.id = p.collection
WHERE p.link_id = :link_id AND c.kind = 'message/rfc822' AND p.hash IS NOT NULL
ORDER BY p.collection, p.seq LIMIT 1;"#;

/// The first `In-Reply-To` id of a message id's summary (STORAGE Annex
/// A.1), the parent of a message the index holds no body of (SEARCH §9).
pub const MESSAGE_IN_REPLY_TO: &str = r#"SELECT r.msg_id FROM placement p
JOIN store.mail_in_reply_to r ON r.collection = p.collection AND r.link_id = p.link_id
WHERE p.link_id = :link_id
ORDER BY p.collection, p.seq, r.position LIMIT 1;"#;

/// One message row: its thread and its parent.
pub const LOAD_MESSAGE: &str = r#"SELECT thread, parent FROM message WHERE link_id = :link_id;"#;

/// The messages naming one as their parent.
pub const MESSAGE_CHILDREN: &str = r#"SELECT link_id FROM message WHERE parent = :link_id;"#;

/// Drops one message row: a message no placement holds and none names.
pub const DELETE_MESSAGE: &str = r#"DELETE FROM message WHERE link_id = :link_id;"#;

/// Drops a thread row no message points to any more, after a join or a
/// split repointed its members (SEARCH §9); a row still pointed to stays.
pub const DELETE_THREAD_UNUSED: &str = r#"DELETE FROM thread
WHERE id = :id AND NOT EXISTS (SELECT 1 FROM message WHERE thread = :id);"#;

/// The thread of one hit, `(account, seq)`, through any of its live mail
/// placements (SEARCH §9).
pub const HIT_THREAD: &str = r#"SELECT m.thread FROM store.items i
JOIN store.collections c ON c.id = i.collection
JOIN message m ON m.link_id = i.link_id
WHERE i.seq = :seq AND c.account IS :account AND c.kind = 'message/rfc822'
  AND i.deleted = 0 AND i.retained_at IS NULL
ORDER BY i.collection LIMIT 1;"#;

/// A thread as one account holds it (SEARCH §9): its live members there,
/// as `thread_members` names them, counted by `seq`, with their lowest and
/// highest sort key naming a date.
pub const THREAD_SPAN: &str = r#"SELECT count(DISTINCT i.seq), min(nullif(i.sort_key, '')), max(nullif(i.sort_key, ''))
FROM message m
JOIN placement p ON p.link_id = m.link_id
JOIN store.collections c ON c.id = p.collection
JOIN store.items i ON i.collection = p.collection AND i.seq = p.seq
WHERE m.thread = :thread AND c.account IS :account AND c.kind = 'message/rfc822'
  AND i.deleted = 0 AND i.retained_at IS NULL;"#;

/// Every statement of this crate's own, paired with its constant name.
pub const OWN: &[(&str, &str)] = &[
    ("COLLECTIONS_MOVED", COLLECTIONS_MOVED),
    ("LOAD_UNREAD_BODIES", LOAD_UNREAD_BODIES),
    ("COUNT_UNINDEXED", COUNT_UNINDEXED),
    ("LOAD_HIT_PLACEMENTS", LOAD_HIT_PLACEMENTS),
    ("LOAD_ENDS", LOAD_ENDS),
    ("LOAD_PLACEMENT", LOAD_PLACEMENT),
    ("LOAD_ITEM_IN_REPLY_TO", LOAD_ITEM_IN_REPLY_TO),
    ("LOAD_ITEM_CATEGORIES", LOAD_ITEM_CATEGORIES),
    ("TASK_CATEGORY_VALUES", TASK_CATEGORY_VALUES),
    ("MESSAGE_PRESENCE", MESSAGE_PRESENCE),
    ("MESSAGE_HASH", MESSAGE_HASH),
    ("MESSAGE_IN_REPLY_TO", MESSAGE_IN_REPLY_TO),
    ("LOAD_MESSAGE", LOAD_MESSAGE),
    ("MESSAGE_CHILDREN", MESSAGE_CHILDREN),
    ("DELETE_MESSAGE", DELETE_MESSAGE),
    ("DELETE_THREAD_UNUSED", DELETE_THREAD_UNUSED),
    ("HIT_THREAD", HIT_THREAD),
    ("THREAD_SPAN", THREAD_SPAN),
];

/// Every search statement, canonical then this crate's own, paired with
/// its name.
pub fn all() -> impl Iterator<Item = (&'static str, &'static str)> {
    CANONICAL.iter().chain(OWN).copied()
}
