---
cairn: log
change: show-the-bindings
landed: 2026-08-28
---

# `item show` names the handles, and the first store it ran against answered a question

The gap was found in use, not in review. A real store came back from `check` with four frozen calendar identities, each one link id a source held twice. The report named the collection, the link id, the source and a count, and stopped there. The next question is always *which two resources*, and nothing in the CLI could answer it: `item show` printed the item in full and said nothing about the binding, so the only ways forward were querying the server or opening `pimdir.db` by hand.

A binding is where a source's own view of an item lives, and none of it was readable. The handle it is addressed by, the base the last sync agreed on, whether it diverged from its own remote, and the other handles a frozen identity is held under: a placement that has stopped moving looked exactly like one that has not. `PimdirReader::item_bindings` returns the same `BTreeMap<ReplicaSourceId, ReplicaSourceBinding>` a hub carries per item, read for one item instead of a collection, so no new type and no second row mapper. `item show` prints one block per source, with `also holds` and `conflicted at revision` appearing only when they apply, so an exception reads as one rather than as another usually-empty line.

`item list` was deliberately left alone. Its rows are one query for a page; a binding lookup per row makes a listing cost what a listing must not, and the verb that names a single item is the one that can afford to say everything about it.

Run against the store that prompted it, the new output answered the question in one command and the answer was not the guess. Both handles are the same UID:

```
 - binding caldav: 2b3vq8jeh2cumenpreo0fm8ssp%2540google.com.ics
    - also holds: 2b3vq8jeh2cumenpreo0fm8ssp%40google.com.ics
```

`%40` is `@`; `%2540` is `%40` encoded again, so the server holds one resource whose name contains an `@` and another whose name literally contains the characters `%40`. Some client wrote a percent-encoded name as a literal resource name, and both resources carry the same `UID`, which RFC 4791 §4.1 forbids within a collection. All four frozen identities have that shape. The freeze is correct and the data is the server's; what was missing was any way to see it.

Capabilities moved: cli, one new requirement and one line of the verb surface.
