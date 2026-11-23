---
cairn: delta
change: a-queued-edit-names-what-it-changes
---

## ADDED Requirements

### Requirement: A flag push names what it adds and removes
`PimdirChangeKind::SetFlags { handle, add, remove }` SHALL carry two known sets in place of a replacement set, so a flag the server changed after the listing survives the push (pimdir SYNC §4, vectors/sync/02, 03, 09, 26, 27, 42). The engine SHALL derive both from the flag merge against the set the source holds, the one the run listed, else the base: `add` what the merge carries and that set lacks, `remove` what that set carries and the merge lacks; a placement whose merged flags are unknown pushes none, and one deriving two empty sets pushes nothing. A connector SHALL apply both as deltas (an IMAP `STORE +FLAGS` and `-FLAGS`, a JMAP keyword patch), never as a replacement. The change key's `set-flags` fields are `add` then `remove`, each a known flag set.

#### Scenario: A local addition beside a server's own flags
- GIVEN a placement adding `\Flagged` to a member the server lists with `\Seen`
- WHEN the collection is synced
- THEN the push adds `\Flagged` and removes nothing, keyed `892471aec94fc225` (vectors/sync/02, src/change.rs `the_derivation_reproduces_the_vectors_keys`)

### Requirement: The push window a source leaves unguarded
A precondition guards a push only where the source enforces it (an `If-Match`, a `STORE UNCHANGEDSINCE`); a source enforcing none leaves a window between the listing and the push in which a server change under an `Update`, a `Remove` or the flags a `SetFlags` names is lost (pimdir SYNC §4). The seam documents it; no row says which sources leave it open.
