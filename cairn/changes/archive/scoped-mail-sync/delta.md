---
cairn: delta
change: scoped-mail-sync
---

# Delta

## ADDED Requirements

### Requirement: A run chooses its listing from the coverage
(sync) A sync SHALL read the source's checkpoint, coverage and round first (a `Handles` load naming none) and ask for one listing (pimdir SYNC §5): the open round resumed from its cursor when it lists the scope asked for; else a round, opened in the write of its first page, when no round ever closed (a store from an earlier draft included), a round is open over another scope, `full` is set, or the scope reaches outside the coverage, over the band alone when the connector's checkpoint is bound to no scope (`PimdirRemote::scope_bound`, `PimdirSync::scope_bound`) and the band adjoins the coverage; else a delta from the checkpoint, the narrower coverage recorded (`SetCoverage`) when the scope lies strictly inside it. A connector answering a round to a delta request opens one; one answering a delta to a round request is merged as a delta. A rejected cursor restarts the round under a new id; a rejected delta checkpoint opens a round.

#### Scenario: A widening on IMAP
- GIVEN a coverage since September and a connector bound to no scope
- WHEN the scope widens to July
- THEN the round lists July to September alone, keeps the checkpoint, and closes with the coverage since July

### Requirement: A round lands page by page
(sync) Every page SHALL land in one write: its members named and merged, its listed handles stamped with the round's id after its upserts (`Stamp`), and, after its last push chunk, its cursor and any checkpoint it carries (`SetRoundCursor`) or, on the last page, the round closed (`CloseRound`) with the checkpoint it carried, else the one a page carried, else the source's own, and the coverage of its scope or of the band's span. A page before the last merges only its members and vanished handles; a delta and a round's last page merge as a delta. An interrupted round keeps what landed; `PimdirSync::report` answers what the pages that landed did.

#### Scenario: An interrupted round resumes
- GIVEN a round whose first page landed with the cursor `p1` before the connector failed
- WHEN the next sync runs under the same scope
- THEN it asks for the round from `p1`, and the last page closes it with the coverage of its scope

### Requirement: Absence means deleted in scope only
(sync) The deletes a round infers SHALL be the based bindings of its source that no page of the round stamped and whose item's summary `date` falls in its scope or is unknown, read when the last page lands (`PimdirLoaded::unstamped`, or for a round that page opens, the based placements it did not list), handled as a vanished member is. A placement out of scope is neither dropped, pulled nor pushed on the evidence of its absence; a vanished handle applies whatever the date; the engine filters no page by date. A `Remove` is derived from a tombstone the consumer staged and from nothing else.

#### Scenario: A message older than the scope
- GIVEN a member dated August, bound and unlisted, under a scope since September
- WHEN the round's last page lands
- THEN the member stays as it was, and an undated unlisted one is dropped

### Requirement: A page names every member it carries
(sync, replaces "A pulled member is a probe") Every member a page lists SHALL be named in the page's write from its `PimdirRemoteMeta`, in handle order and against the whole collection: a hint a pending create of this source holds lands that create (a `Superseded` drop of the provisional handle, the create under the listed handle, based on what the listing reported, reported `Created`); a free hint keys it, a held one is minted; the item is inserted at `Meta`, or `Full` with the body the member carried, based on the reported flags, revision and body, reported `Added`. A bound member refreshes its summary and sort key from its meta when they moved and its revision did not, keeping a walked attachment mark when it holds its body; a mutable one stating another hint is keyed afresh. A pull takes the member's meta and the body it carried, the level `Meta` when it carried none; under `Manual` a carried diverging body lands in `conflict_object`. A tombstone a remote edit revives is pulled the same way.

### Requirement: A create waits for the page that lands it
(sync, replaces "A create waits for the probes") A `Created` placement SHALL derive no `Add` from a page before the last of a round, nor from a delta while a round is open: a page may carry its arrival, which lands it. A pending create carrying no origin whose summary `date` the run's scope excludes SHALL wait for a round whose scope holds it, counted in `PimdirSyncReport::waiting`.

#### Scenario: An old message relocated into a scoped target
- GIVEN a pending create dated August and a run over a scope since September
- WHEN the round's last page lands without it
- THEN no `Add` is derived and the report counts it waiting

### Requirement: A scope bounds mail only
(sync) `PimdirSyncOptions::scope` is unbounded by default; `PimdirSourceStore::sync` and `prepare_sync` SHALL refuse a bounded scope on a collection whose kind is not `message/rfc822`, naming the kind (`PimdirError::Scope`).

### Requirement: A rebuild reads every page before its one batch
(rekey, replaces "The meta fetch goes in chunks") A rekey SHALL list a round over the whole collection whatever the scope, every page read before its one batch, a rejected cursor relisting it from the start, and name each member from its meta, fetching nothing. The batch lands the last checkpoint a page carried, none keeping the old one, and leaves the coverage alone.

### Requirement: Nothing reaches the store unnamed
(seam, store) A write SHALL refuse an upsert carrying no link id for a handle no binding holds (`PimdirError::Unnamed`); one for a bound handle folds into its binding as before. The store holds no probe rows.

### Requirement: The store holds coverage and rounds per source
(store) `load` SHALL answer the source's coverage (`PimdirCoverage`) and open round (`PimdirRound`) beside the checkpoint, and on an `All` load while a round is open the unstamped bindings in its scope. A write SHALL apply `OpenRound`, `SetRoundCursor`, `CloseRound` and `SetCoverage` with the canonical statements, `Stamp` after the batch's upserts, and draw the public ids of the items a batch inserts in the order it upserts them. Open SHALL reconcile an earlier draft's store: the coverage and round columns of `sources` and `bindings.round` added from the canonical DDL, `sources_stamp_coverage` created, `probes` dropped; a reader of one not reconciled reads no coverage. A row at level 0 SHALL load as `Meta` with no summary, a claim the next `Meta` upgrade revisits.

### Requirement: A collection is listed with its coverage
(store) `PimdirCollection::coverage` SHALL be the narrowest of the collection's sources' (the latest floor, the earliest ceiling, the oldest closing), `None` while one of them never closed a round; `list_coverage` answers it per source with the round each has under way.

### Requirement: The owner collects below a date
(store) `PimdirStore::collect_before(collection, before)` SHALL run `collect_before` and `recompute_refcounts` in one transaction, answer the public ids collected, and refuse to run while a verb of the process is between two chunks (`PimdirError::InFlight`). No tombstone, no push: the remote keeps every member.

### Requirement: The mail reads count, page and search across collections
(store) The reader SHALL answer `count_mail`, `count_mail_by_day`, `count_unread`, `list_mail_page_filtered` and `search_mail` over a set of collections under `PimdirMailFilter` (the read and attachment chips), pages on `PimdirMailCursor` `(sort_key, seq, collection)` with summaries and addresses, from the committed rows. `like_pattern` builds the search pattern, escaping `%`, `_` and `\`.

### Requirement: The attachment mark is read without the body
(summaries) `summary::mail::derive_meta(header, size, attachment)` SHALL derive from a header block what `derive` does from the body, the attachment mark aside: the source's own flag where it states one, else `meta_attachment`, `true` for a top-level `multipart/mixed`. A summary read without the body SHALL NOT replace a mark the walk of the parts gave (`summary::without_body`, for a listing, a `Meta` fetch and a rekey). tests/summaries.rs checks `meta_attachment` and that the two derivations agree on every other column.

## MODIFIED Requirements

### Requirement: A re-listed probe is not a pull
(sync) Renamed "A re-listed member is not a pull": a member a page lists with the flags and meta the store holds SHALL derive no write, no event and no count. A base-less row holding a body whose member a listing no longer carries is resurrected as a `Created` placement; one holding none is dropped.

### Requirement: A move delivers exactly one copy
(sync) A relocated member is listed by the target's next listing under a new handle and the page naming it lands the create. The source-first scenario lands the create in the target's page, not in a `Meta` fetch.

### Requirement: The checkpoint lands in the last write
(sync) The checkpoint is a delta's or the one a round closes with; a page's cursor and the checkpoint an open round carries land with their page and are not the source's checkpoint.

### Requirement: Events report what the remote changed
(sync) `Added` reports a member a page named, `Created` also a create a page landed under the listed handle. `Vanished` reports a member a round found absent in its scope as well as a vanished handle.

### Requirement: Naming a probe gives it a base
(upgrade) Renamed "A Meta upgrade revisits a claim the row does not hold": the `Meta` tier SHALL fetch only placements holding no summary, a level of 0 loading as one; a fetch naming a placement with no link id still gives it a base of what the source reported. A body-less fetch over a placement holding its body keeps the walked attachment mark.

### Requirement: A pending create is landed by its arrival
(upgrade) A hint a page or a fetch carries lands the pending create holding it; the page's landing is the sync's, on the same terms.

### Requirement: A rekey never writes a base it never reconciled
(rekey) The pull a rekey carries lowers the level to `Meta`, the revision being the listed one.

### Requirement: A hub-backed store owns the rows the hub cannot key
(hub) There are no such rows: the store refuses an unnamed upsert of an unbound handle, every listed member arriving named.

### Requirement: A mutation of a probe is refused
(mutate) `PimdirMutateError::Probed` is `PimdirMutateError::Unnamed`: a placement no link id names refuses a mutation.

### Requirement: The verb surface
(cli) `collection list` shows the coverage (`COVERED`: `all`, `since …`, `-` while a source never closed a round, and `coveredSince`, `coveredUntil`, `coveredAt` in JSON) instead of the probe count.

## REMOVED Requirements

### Requirement: A pulled member is a probe
(sync) Replaced by "A page names every member it carries".

### Requirement: A create waits for the probes
(sync) Replaced by "A create waits for the page that lands it".

### Requirement: The meta fetch goes in chunks
(rekey) Replaced by "A rebuild reads every page before its one batch"; `PimdirRekey::FETCH_CHUNK` is gone.
