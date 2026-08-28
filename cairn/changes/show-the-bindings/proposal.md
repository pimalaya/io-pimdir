---
cairn: change
id: show-the-bindings
status: landed
created: 2026-08-28
---

# `item show` names the handles a source holds an item under

## Why

`check` reports an identity a source holds more than once, and says those items
stop syncing until it holds them once again. It names the collection, the link
id, the source and a count. It does not name the handles, and neither does
anything else the CLI offers, so an operator who reads that report has nowhere
to go: the next question is always *which two resources*, and answering it means
going back to the server, or opening `pimdir.db` by hand.

Found while triaging a real store, where four calendar identities were frozen
under one `UID` apiece. `item show` printed the item in full and said nothing
about why it had stopped moving.

The gap is wider than the frozen case. A binding is where a source's view of an
item lives: its handle, the base it last agreed on, and whether it is
conflicted. Every one of those is invisible today, so a placement that is not
syncing looks exactly like one that is.

## What

`item show` prints, after each placement, one block per source holding it: the
handle, the base the last sync agreed on, and the two exception lines,
`also holds` for a frozen identity and `conflicted` for a diverged one, printed
only when they apply.

The read is a library one, on the handle the verb already holds, like every
other diagnostic. `PimdirReader` gains `item_bindings`, returning a
`PimdirBinding` per source.

`item list` is untouched. Its rows are a page of a collection, one query for all
of them, and a binding lookup per row would make a listing cost what a page
should not. The verb that names one item is the one that can afford to say
everything about it.

## Not in scope

**No repair.** `item show` reports; nothing about a frozen identity is resolved
here. Which of two resources to keep is the operator's call, and it is made on
the server, not in the store.

**No handle lookup verb.** Resolving a handle back to an item is a different
question (`item show --handle`), and nothing has asked for it yet.
