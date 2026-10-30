---
cairn: delta
change: the-engine-follows-sync-draft-05
---

## ADDED Requirements

### Requirement: An add accepted without a handle waits for its page
An `Add` answered `Accepted` with no assigned handle (an IMAP `APPEND` on a server without UIDPLUS) SHALL leave the create pending: its provisional binding, staged body and flags kept, no `Superseded` drop written, no `Created` reported, and no second push in the run; the page naming the member lands it (pimdir SYNC §4, vectors/sync/92).

#### Scenario: An APPEND answered with no UID
- GIVEN a pending create the remote accepts with no assigned handle
- WHEN the outcome is settled
- THEN nothing is written for it and nothing is reported (src/sync.rs `an_add_accepted_without_a_handle_stays_pending`)

### Requirement: Another message under a held Message-ID is minted apart
A member whose hint the collection holds under an item another source binds SHALL name that item only when its mail summary agrees with the item's on `subject`, `date` and `sender`, and SHALL otherwise be keyed by the identity minted over its handle (pimdir SYNC §6, §9, vectors/sync/93). The page's own naming applies it to an offered copy it would land; the write applies it to every upsert binding a new source to an item another source binds, since a page before the last and an upgrade name their members against this source's holders alone (pimdir SYNC §10).

#### Scenario: Another message reusing the identifier
- GIVEN source a holding `basic-1@example.org`, the stand-up notes
- WHEN source b writes a member under that key whose summary names another subject and sender
- THEN b's member is keyed `dup:basic-1@example.org#77` and a's item is left as it was; the same message served by b joins a's item (tests/another_message.rs)

### Requirement: A body is verified before it is pushed
Before a chunk is pushed, the sync SHALL yield `PimdirYield::WantsVerify` with every body its changes upload (an `Add` holding no origin, an `Update`), answered by `PimdirArg::Verified`, the bodies whose blob reads whole and hashes to its name (`PimdirSourceStore::verify`, pimdir STORAGE §7). A change whose body is missing or corrupt SHALL be withheld, its placement left as the merge wrote it, and counted in `PimdirSyncReport::unreadable` (vectors/sync/94).

### Requirement: An event follows its commit
A sync SHALL report an event only once the write recording it has been answered, and every event of an answered write (pimdir SYNC §5): the events derived for a batch wait in the coroutine and move to the report when the write is answered, so a run whose write fails reports nothing of it.

#### Scenario: A write never answered
- GIVEN a page naming a new member
- WHEN the run yields the write and it is not answered
- THEN the report carries no event, and `Added` once it is (src/sync.rs `an_event_waits_for_the_write_recording_it`)

### Requirement: A carried body equal to the edit settles it
A member listed at a revision past the base and carrying the very body the placement's local edit holds SHALL settle as the push whose record was lost: the base adopts the revision and that body, nothing conflicts and nothing is pushed (pimdir SYNC §5; sync-audit S13).

#### Scenario: A lost push record
- GIVEN a local edit `h2` over `h1@r1` and a listing at `r2` carrying `h2`
- WHEN the collection is synced
- THEN the placement is `Clean` at `h2@r2` with no conflict (src/sync.rs `a_carried_body_equal_to_the_edit_settles_it`)

### Requirement: A stated key is named on a server without its escape
`PimdirLinkId::stated` SHALL answer the identity a stated key names on a server, the key with its one leading `esc:` stripped (pimdir STORAGE §9): what a connector compares with or hands its server for the `link_id` of an `Add` or a `Remove`. The engine names no identity to a server itself.

## MODIFIED Requirements

### Requirement: An edit naming the body it replaced is guarded
`PimdirMutation::Edit` SHALL carry `if_match: Option<PimdirHash>`, the body the producer read; an edit stating one SHALL fail as `PimdirMutateError::Replaced { handle, read, held }`, staging nothing, when the placement holds another body (pimdir SYNC §7, vectors/sync/87). The drain passes a queued `update`'s `if_match` through after its own check.

### Requirement: The flag axis with no base merges the union
A placement present on both sides with no base SHALL merge its flags as the union of the two known sets, no side being known to have removed anything, pushing what its server lacks (pimdir SYNC §5; sync-audit S15); one whose flags agree is left alone.

#### Scenario: A base-less placement on both sides
- GIVEN a placement with no base holding `flagged` and its member listing `seen`
- WHEN the collection is synced
- THEN both flags are kept and `flagged` is pushed as an addition (src/sync.rs `no_base_merges_the_union_of_both_sets`)

## REMOVED Requirements

### Requirement: A level of 0 reads as an unheld claim
The owner's read no longer special-cases `level` 0: the reconciliation repairs it to 1 (`repair_levels`, STORAGE §6) before any owner reads the store, and the column is closed to 1 and 2 (STORAGE §13).
