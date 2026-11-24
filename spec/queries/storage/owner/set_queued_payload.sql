-- Rewrites one queued row's payload (§14): a submit whose copy names the
-- collection renamed in the same transaction.
UPDATE queue SET payload = :payload WHERE id = :id;
