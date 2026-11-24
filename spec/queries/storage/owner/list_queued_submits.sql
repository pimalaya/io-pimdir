-- Runs beside rename_collection (§14): every queued submit, pending or
-- parked, whose payload's copy the rename may name. A submit's copy is the
-- one collection a payload names that no column holds, read from the payload
-- by the crate (PimdirSubmit::from_payload), since no statement calls a JSON
-- function.
SELECT id, payload FROM queue WHERE action = 'submit' ORDER BY id;
