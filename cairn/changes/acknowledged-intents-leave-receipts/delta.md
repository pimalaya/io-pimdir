## ADDED Requirements

### Requirement: A performed intent is acknowledged with a receipt
`PimdirStore::acknowledge_action(id, seq)` SHALL delete one queue row, pending or parked, release its object pin and record its receipt (`record_receipt`: the row's id, the collection it was queued on, `seq`) in one transaction, and report whether the row existed, recording nothing when it did not. `action_status(id)` then answers `Applied`.

## MODIFIED Requirements

### Requirement: A queued action can be cancelled or acknowledged
`drop_action(id)` withdraws a row by request and records no receipt; acknowledging a performed intent is `acknowledge_action`'s.

### Requirement: A performed intent is replaced by the change it leaves
`replace_action` also records the intent's receipt with no `seq`, in the same transaction.

## REMOVED Requirements
