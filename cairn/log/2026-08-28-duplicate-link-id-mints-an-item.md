---
cairn: log
change: duplicate-link-id-mints-an-item
date: 2026-08-28
---

# The second copy gets a row, so the column that described it goes

This **supersedes `duplicate-link-id-freeze`** (landed 2026-08-25), three days old and reversed on the same evidence that motivated it. That change was right that a write must not repoint a binding, and wrong about what to do instead: recording the losing handle on the winner's binding describes a resource rather than storing it, and a description is not a replica.

Two findings, verified 2026-08-28 against a Posteo CalDAV account, made that the defect rather than the compromise. A calendar of 454 items held four `UID`s under two hrefs each, one named `<uid>@google.com.ics` and one `<uid>%40google.com.ics`, both written by the same client. Three pairs differed only in `DTSTAMP` and `LAST-MODIFIED`; the fourth was two genuinely different meetings sharing one `UID`, so no rule about picking a survivor would have been safe. The replica held four events, and the user's other four sat on the server, in no row of this store, invisible to every read it offers. The second finding is why the freeze could not even hold still: that server implements no `sync-collection`, so the collection is enumerated in full on every run, and the frozen twin came back, was fetched whole to resolve its identity, lost the claim again and left its body unreferenced. Four downloads and four orphan blobs per sync, indefinitely, which is the opposite of the "appears in exactly one enumeration" the column's justification rested on.

The engine now mints the second copy a key of its own (`dup:<hint>#<handle>`, pimdir SPEC §9) before it writes, so what reaches this store is an ordinary item and the store has nothing left to describe.

## What landed

- **`bindings.ambiguous_handles` is gone** (capability `store`): the column from the inlined schema, `handles_to_json` / `handles_from_json` from `codec`, the projection out of `load`, the value out of `write`, `AMBIGUOUS_BINDINGS` and `PimdirAmbiguous` from the diagnostics, and the field from the `pimdir item show` JSON. Version 1 is still edited in place while the format is draft, so the reconciliation that adds a folded-in column on open now also drops a folded-out one: a store written with the column loses it on the next open, in the same transaction as the column and index reconciliation. `ALTER TABLE` expresses that whole, no index, key or constraint naming the column, so §6's table rebuild is not needed and no second copy of the canonical `bindings` DDL exists to drift from.

- **The refusal stays, and loses its record.** A write resolving an existing `(collection, link_id, source)` to a different handle fails with `PimdirError::Rebind`, naming the collection, the key, the source, the handle held and the handle carried. The floor is unchanged, no write repoints a binding; what changed is that the caller now has somewhere to put the second copy, so refusing is a complete answer rather than a loss to be described. `UPDATE_BINDING` still carries no `handle` and now cannot: the diff refuses before it reaches a statement.

- **A second refusal, one write earlier.** A batch is checked before it is folded, and refused when two of its upserts claim one link id under two handles. The hub is keyed by link id, so folding both would keep whichever the batch named last and drop the other with nothing failing. This is not hypothetical: `rekey` carries a minted key but mints none, re-resolving every identity from the new spine, so a renumbered collection that genuinely holds a duplicate hands this store exactly that batch. It is refused with the same typed error rather than silently overwritten.

- **The `Superseded` licence is unchanged in shape**, and still per handle: a rebuild's drop licenses the rebind of the handle it names and no other, so a rekey carries its collection over and a rekey batch holding a genuine second copy is refused for that one. Only the else-branch moved, from recording to refusing.

- **`pimdir check` counts minted keys per collection** in place of the ambiguity counter, printed apart from the problems because it is not one. What it is worth saying is the trend: a collection whose count climbs every sync is a source renaming the same duplicate, which is the Posteo shape one enumeration later.

- **The queued `add` is untouched.** It still parks on a duplicate `link_id` (SPEC §15.3). Minting is what reading a source requires, since a replica owes the collection what the collection holds; a producer authoring an item under a key the collection already holds got it wrong, and is told so rather than having its item filed under a key it never asked for.

## Verification

- 119 tests green (15 lib, 104 integration), `cargo clippy --all-targets --all-features` clean, `cargo fmt`.
- `tests/duplicate_link_id.rs` is rewritten to the new outcome rather than deleted, being the regression this is judged on: the colliding write is refused by type and stores nothing; two resources under one hint are two items with their own `seq`, binding and body; a byte-identical pair shares one object with no refcount drift; a minted key round-trips through a page, retirement and revival with its public id intact; a rekey carries its binding over; a superseded handle licenses only its own rebind; and a rebuilt handle space resolving two placements to one key is refused rather than half-written.
- `tests/draft_reconcile.rs` covers both halves of the draft allowance now, a column folded in and a column folded back out, both derived from the current schema rather than pasted in.
- `tests/item_bindings.rs` reads the minted copy's own binding, which is what names the resource each copy came from now that no binding names two.
- The spec-fidelity suite is green against the sibling pimdir checkout: pimdir leads the chain, so the suite was red by design and reported exactly one difference, this crate's inlined `bindings` still carrying the column, and closing it was this step. `tests/conventions.rs` takes the three new minted vectors on the format's own terms, checking the hint this crate derives and the composition the vector's key states, since minting is the engine's and a store never performs it.

Capabilities moved: `store`, `cli`.
