-- Removes a collection with everything under it: items, bindings, summaries,
-- addresses and sources cascade (§14), a receipt stays and names no
-- collection (§15.4), collections_count_delete counts the delete in `purges`
-- (§4.5), and the queue's foreign key refuses it while a queue row names it
-- (collection_holds_actions, run first, says so typed). Retention does not
-- apply, the operator having removed the collection itself, and the pins the
-- cascade drops are settled by recompute_refcounts in the same transaction,
-- the one write that cannot know its net change. Children keep their rows
-- with `parent` set NULL.
DELETE FROM collections WHERE id = :collection;
