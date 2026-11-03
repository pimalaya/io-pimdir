## ADDED Requirements

### Requirement: An edit over an unread base waits for it
A placement holding a body over a base naming a revision and no body (`PimdirPlacement::awaits_base`) SHALL derive no `Update`, nor the `Remove` a move's staged edit rides ahead of, until an upgrade reads that base (pimdir SYNC §5).

#### Scenario: An edit after a bodiless pull
- GIVEN a card whose base names `r2` and no body, holding a local edit
- WHEN a sync lists it still at `r2`
- THEN nothing is pushed and the rows stay as they were (vectors/sync/132)

### Requirement: A capped listing says so
A connector knowing that its source caps a collection's listing SHALL set `PimdirRemoteSnapshot::capped` on every page of that collection's rounds and on no other (pimdir SYNC §4, §5).

### Requirement: A capped listing proves no absence
A round whose last page states `capped` SHALL find no member absent: it closes as any round does, drops only vanished handles, and counts the based bindings in scope it did not list in `PimdirSyncReport::unlisted` (pimdir SYNC §5).

#### Scenario: A folder under a size limit
- GIVEN a based binding a capped round's last page does not list
- WHEN the page lands
- THEN the binding stays, the item stays live, and the round closes (vectors/sync/134)

## MODIFIED Requirements

### Requirement: A staged create never takes a key its target holds
A `Copy` or a `Move` of a placement holding no body SHALL fail with `PimdirMutateError::Undeliverable` unless its binding is based and the run's source syncs the target (`PimdirMutate::syncs_target`, which `PimdirSourceStore::mutate` reads from `collection_sources`), `false` by default (pimdir SYNC §7; vectors/sync/135).

### Requirement: A fetch moves the base
Over a staged edit, a fetch reporting the base's own revision keeps the local body; a base naming no body takes the fetched body (vectors/sync/133).

### Requirement: An upgrade revisits what it never got
`Full` also revisits a placement holding a body over a base naming a revision and no body.
