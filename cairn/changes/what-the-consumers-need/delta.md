# Delta: what the consumers need

Capabilities: rekey.md, mutate.md, store.md and search.md. sync.md, hub.md and upgrade.md are unchanged.

## ADDED Requirements

### Requirement (rekey.md): A repeated handle keeps its first entry
A rekey listing a handle twice SHALL read only its first entry: its flags, link id, summary, sort key and revision alike (pimdir SYNC §4). Check: vectors/sync/208.

### Requirement (rekey.md): A delivered create is landed as a page lands it
A delivered create a member resolves to SHALL be landed as a page lands it (SYNC §6, §8). That means a `Superseded` drop of its provisional handle, its staged flags kept, its base the listed flags with the staged body, and `Created` reported once the batch commits. Check: vectors/sync/209.

### Requirement (rekey.md): A listed body is stored as a page stores it
A body the listing carries, inline or persisted, SHALL be stored (`StoreObject`) at level `Full`. It becomes an arrival's body, a pull's body and base at the listed revision, or the diverging body of a conflict the rekey records (SYNC §5, §8). Check: vectors/sync/206, 207.

### Requirement (rekey.md): A base-less conflict under a voided handle
A conflicted binding with no base under a handle the renumbering voids SHALL be carried onto the member resolving to it, with its conflict and a base of the listed flags naming no revision and no body. When no member resolves to it, it SHALL be re-staged as a pending create, as SYNC §5 re-stages a base-less body whose member went (D1). Check: vectors/sync/213, 214.

### Requirement (store.md): The drain reads a target as a load of its links answers it
A drained `copy` or `move` SHALL read its target through the projection a `Links` load answers. A copy the store offers this source for an item it lacks is no live placement there: the create joins that item, as `store.mutate` does (SYNC §7, §10; STORAGE §14). Check: tests/drain_copy.rs, failing before.

### Requirement (store.md): A drained action leaves the rows its mutation leaves
For an action whose item one source binds, the drain SHALL leave the rows `store.mutate` leaves for the mutation it maps to (SYNC §7). Check: tests/drain_agrees.rs over the `mutate` vectors.

### Requirement (store.md): A pending listing names a row it cannot decode
`pending_actions` SHALL list a pending row whose payload does not decode as an opaque action naming its decode error, never fail the collection's listing (STORAGE §15.4; G1, cardamum CD1). Check: tests/generated/reader.rs, the G1 twin no longer ignored.

### Requirement (store.md): The Annex A revision is public
The revision summaries are derived under (`store_meta.derivation`, STORAGE §6) SHALL be a public constant outside `client`, the one the store stamps (android AN3).

### Requirement (search.md): The index statements build without client
`sql::search` (statement texts, `FORMAT_VERSION`, `MIGRATIONS`, `all()`) SHALL compile under `indexer` alone, so a consumer running the refresh over its own SQLite links no rusqlite (android AN1). Check: tests/indexer_alone.rs, and `cargo tree` naming no rusqlite.

## MODIFIED Requirements

### Requirement (rekey.md): A rekey never writes a base it never reconciled
The R1, R2 and R3 changes:
- A `Conflict` the old binding held SHALL be carried as it is, revision and diverging body kept, unless the listing states another revision; a listing stating no revision states no move (R1).
- A tombstone whose binding is conflicted SHALL keep that conflict, and derives no `Remove` (R2b, R2c).
- A tombstone holding a local edit and meeting a remote edit SHALL be carried as a `Conflict` at the fetched revision, never revived as a pull (R3).

This replaces "a tombstone meeting one is carried as it is ... for the next sync to revive it", which the code no longer does: a tombstone with no edit meeting a remote edit is revived as the pull SYNC §5 makes. Check: vectors/sync/202 to 205.

### Requirement (rekey.md): A rebuild carries state over by link id
Adds: a listed flag set that is unknown SHALL leave the base's known flags as they are, so a flag change still owed is pushed by the next sync (SYNC §3, §8; R4). Check: vectors/sync/201.

### Requirement (rekey.md): A rebuild keys two copies of one hint apart
"The keys of the pending creates a rebuild leaves untouched SHALL count as taken" and its scenario "A pending create keeps its key" are replaced. A member stating the hint of this source's pending create SHALL land it as a page lands it (SYNC §6), or be minted apart and reported `Disagreed` when the summaries disagree (D10, R6a). A copy the store offers (`PimdirLoaded::offered`) takes no key: a member stating its hint joins that item, as a page does (SYNC §6, §8; R6b). Check: vectors/sync/210 and 212.

### Requirement (rekey.md): A rekey reports the members it names
Adds: `Created` for a delivered create it lands, and `Overruled` for each body its write kept (`PimdirWritten::overruled`; SYNC §5). Check: vectors/sync/209 and 205.

### Requirement (mutate.md): A staged create never takes a key its target holds
"a live placement there" now reads: a live placement the run's source binds there. The store's offered copy of another source's item holds no key, and the create joins that item (K4). Check: tests/drain_copy.rs and the existing mutate vectors.

## REMOVED Requirements

None.
