---
cairn: change
id: multi-account
status: landed
created: 2026-08-08
---

# One store, several accounts: group collections, decide nothing

## Why

A pimdir store holds any item kind already: `collections.kind` is a media type,
so mail, contacts and calendars coexist by design. What it has never had is an
**account** dimension. `grep -i account` over the spec returns nothing, and the
schema carries no such column.

That is fine while a store serves one account, and it is the wall as soon as a
client wants a merged view. The defining operation of such a view is "everything,
minus this account", and today that is either a `LIKE 'work/%'` scan over
collection ids or a map the client keeps on the side, both of which require the
reader to know the owner's naming convention. The grouping exists in every
multi-account deployment; it is simply not queryable.

The immediate consumer is the Pimalaya Android app, which is being refactored
onto one store for all accounts and all three domains. It needs the filter axis
as a `WHERE`, not as a prefix match.

## What (design)

**Schema, in place at version 1.** The spec is a draft and stores are recreated
rather than migrated, so `VERSION` stays `1`. `collections` gains, right after
`id`:

- `account TEXT`, nullable: an opaque owner-chosen id (an address, a config
  name). `NULL` in a single-account store, which is the shape everything
  degenerates to.

plus a partial index `collections_by_account ON collections(account) WHERE
account IS NOT NULL`, so a single-account store writes no account and pays for
no index. `pimdir/migrations/0001_init.sql` takes the identical DDL.

**`id` stays unique store-wide.** An owner filing two accounts in one store
still namespaces their collection ids (`work/INBOX`, `home/INBOX`) exactly as it
would have without the column. What the column buys is that the grouping becomes
an indexed query instead of a convention a reader must reverse-engineer. Making
the primary key composite would have rippled an `account` column into `items`,
`bindings`, `sources` and `queue`: a version-2-sized change, not an in-place
edit.

**The account partitions nothing, and this is the whole point.** Link ids,
hashes and `seq`s keep their store-wide meaning. Two accounts holding one
`Message-ID` share a `seq`, because `seq` is defined as the short form of the
link id (spec §9.1) and the link id is genuinely equal; that restates a fact the
content carries rather than asserting the two placements are one thing. One body
reaching two accounts is still one object, refcounted twice.

An earlier draft of this change scoped link-id identity per account, so that two
accounts holding one `Message-ID` drew two `seq`s. That was rejected: it is a
mail-shaped policy compiled into a generic store, and it made the `seq`
incoherent with its own definition, since one link id would have carried two
short forms.

**Multiplicity is reported, not resolved.** Two reads expose where an identity
or a body occurs across every collection and account:

- `LIST_LINK_PLACEMENTS(link_id)`, the identity axis
- `LIST_OBJECT_PLACEMENTS(hash)`, the dedup axis, which pairs placements two
  servers gave different link ids

A mail view reads those and lists every placement, because two receipts of a
newsletter have two read states and two servers. A contact view reads the same
rows and may offer to merge them, because one person in two address books is
usually one person. Neither behaviour is in the store, so both stay possible and
a kind the spec has not anticipated is not pre-judged. This is the discipline
the store already follows for `kind` (declared, never derived), `conflict` (a
policy the collection carries) and `meta` (opaque).

**A handle speaks for one account**, the way it already speaks for one source:
`PimdirStore::open(dir, source).for_account("work")`. The lazy
`ENSURE_COLLECTION` inside the io-replica write seam has no account argument to
take, so it takes the handle's; a multi-account owner opens one handle per
account, each a SQLite connection, with §7's single-owner rule untouched by how
many a process holds.

## Scope / non-goals

- No `VERSION` bump and no migration runner: the draft allowance covers this.
- **No `accounts` table.** The store records which account a collection belongs
  to and nothing else about it: no credentials, no endpoints, no display name,
  no enabled flag. Those belong to whatever configures the owner. The
  consequence is deliberate and documented: the store learns an account only
  through its collections, so one with no collection yet does not appear in a
  listing.
- **Single-owner is unchanged.** One store means one writing owner whatever it
  holds, so one store for several accounts means one owner process for all of
  them. An owner wanting to sync accounts in parallel processes still gives each
  its own store. This change does not remove that trade.
- io-replica is untouched.
- No merge, dedup or presentation policy of any kind.
