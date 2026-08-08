---
cairn: change
change: multi-account
---

# Delta

## ADDED Requirements

### Requirement: A collection records the account it belongs to
`collections` SHALL carry `account` (TEXT, nullable): an opaque owner-chosen id,
`NULL` in a single-account store. A partial index `collections_by_account ON
collections(account) WHERE account IS NOT NULL` SHALL back the by-account reads,
so a single-account store writes no account and pays for no index.

`collections.id` SHALL remain unique store-wide. An owner filing several
accounts in one store namespaces their collection ids; the column makes that
grouping queryable rather than replacing it.

The store SHALL NOT interpret the value. It is neither parsed, nor validated,
nor matched against configuration.

### Requirement: The account partitions no identifier
Link ids, hashes and `seq`s SHALL keep their store-wide meaning whatever account
a collection belongs to. In particular the `seq` lookup (`SEQ_FOR_LINK_ANY`)
SHALL remain unscoped: two placements sharing a `link_id` share a `seq` whether
or not they sit in the same account, because the `seq` is the short form of the
link id and the link id is equal.

Object deduplication SHALL likewise stay store-wide: one body reaching two
accounts is one object, refcounted per placement.

Regrouping a collection (`SET_COLLECTION_ACCOUNT`) SHALL be safe at any time,
disturbing no `seq`, link id or object.

### Requirement: Multiplicity is reported, never resolved
The store SHALL expose where one identity or one body occurs across every
collection and account, and SHALL take no position on what that means:

- `LIST_LINK_PLACEMENTS(link_id)` returns each live placement of one link id
  with its collection, account, `seq`, `object_hash`, `flags` and `level`.
- `LIST_OBJECT_PLACEMENTS(hash)` returns the same by body, with `link_id` in
  place of `object_hash`, pairing placements two servers gave different link ids.

Both SHALL exclude tombstones and retained rows, and SHALL order by account then
collection with the `NULL` account first.

Merging, hiding or pairing placements is the consumer's decision. The store
SHALL NOT implement any of them.

### Requirement: A handle writes under one account
`PimdirStore` SHALL carry an optional account, `None` by default and set by
`for_account`, and SHALL bind it on every collection it creates, including the
lazy `ENSURE_COLLECTION` inside the io-replica write seam and the producer's
enqueue transaction. A multi-account owner opens one handle per account.

`for_account` SHALL NOT change §7's single-owner rule: how many handles a
process holds is unrelated to how many processes may own a store.

## MODIFIED Requirements

### Requirement: The client read surface exposes accounts
`LIST_COLLECTIONS` SHALL return `account` alongside the existing columns.
`LIST_COLLECTIONS_BY_ACCOUNT(account)` SHALL return one account's collections in
the same shape, matching with `IS` so that binding `NULL` selects the
collections of a single-account store. `LIST_ACCOUNTS` SHALL return the accounts
owning at least one collection.

Because the store learns an account only through its collections, `LIST_ACCOUNTS`
is not a configured roster: an account with no collection does not appear, and a
consumer needing the full roster reads it from its own configuration.

### Requirement: Collection creation binds the account
`ENSURE_COLLECTION` and `SET_COLLECTION_KIND` SHALL bind `:account`. Neither
SHALL overwrite an existing one: `ENSURE_COLLECTION` inserts or does nothing,
and `SET_COLLECTION_KIND` updates the `kind` alone, so a collection cannot
change account as a side effect of a sync declaring its media type. Regrouping
is the deliberate `SET_COLLECTION_ACCOUNT`.

## REMOVED Requirements

None.
