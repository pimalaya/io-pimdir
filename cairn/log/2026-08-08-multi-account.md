---
cairn: log
date: 2026-08-08
change: multi-account
---

# One store, several accounts: the column groups, and decides nothing

A pimdir store has always held any item kind (`collections.kind` is a media
type), and never had an account dimension: `grep -i account` over the format
spec returned nothing. That is fine for one account and is the wall for a merged
view, whose defining operation is "everything, minus this account". Without a
column that is a `LIKE 'work/%'` over collection ids or a map the client keeps on
the side, both requiring the reader to know the owner's naming convention.

`collections` gained `account` (TEXT, nullable) plus the partial
`collections_by_account` index, folded into **version 1** (the format spec is a
draft, `sql::VERSION` stays `1`), structurally identical to
`pimdir/migrations/0001_init.sql`, with the column and index added to
`reconcile_draft_shape` so an earlier-draft store heals on open rather than
failing on a missing column.

`collections.id` stays unique store-wide, so owners still namespace
(`work/INBOX`, `home/INBOX`); the column makes that grouping an indexed query
instead of a convention readers reverse-engineer. A composite primary key would
have rippled an account column into `items`, `bindings`, `sources` and `queue`,
which is a version-2-sized change rather than an in-place edit.

**The account partitions nothing, and that is the substance of the change.** An
earlier draft scoped link-id identity per account, so two accounts holding one
`Message-ID` drew two `seq`s. It was rejected before landing: that is a
mail-shaped policy compiled into a generic store, and it made the `seq`
incoherent with its own definition, since one link id would have carried two
short forms. Link ids, hashes and `seq`s therefore keep their store-wide
meaning; two accounts holding one identity share a `seq`, and one body reaching
both is one object refcounted twice.

What replaced the policy is two reads that report the fact and resolve nothing:
`link_placements(link_id)` on the identity axis, `object_placements(hash)` on the
dedup axis, each returning every live placement with the collection and account
it sits in. A mail view lists them, because two receipts of a newsletter have two
read states and two servers; a contact view may offer to merge them, because one
person in two address books is usually one person. Both read the same rows, so
neither behaviour is in the store and a kind the spec has not anticipated is not
pre-judged. This is the discipline the store already applied to `kind` (declared,
never derived), `conflict` (carried, not assumed) and `meta` (opaque).

A handle now speaks for one account the way it already speaks for one source
(`PimdirStore::open(dir, source).for_account("work")`), because the lazy
`ENSURE_COLLECTION` inside the io-replica write seam has no account argument to
take and must take the handle's. `PimdirProducer` gained the same builder for its
enqueue transaction. §7's single-owner rule is untouched: how many handles a
process holds is unrelated to how many processes may own a store, and an owner
wanting to sync accounts in parallel processes still gives each its own store.

No `accounts` table: the store records which account a collection belongs to and
nothing else, so credentials, endpoints and display names stay with whatever
configures the owner. The consequence is deliberate and documented, namely that
`list_accounts` is not a configured roster and an account with no collection yet
does not appear in it.

io-replica is untouched. The format spec (`pimdir/SPEC.md` §9.2, plus the
terminology entry, §4.3, §12.1, the migration and the two query files) landed
first, since it is the source of truth this crate inlines.

Seven tests cover it: the seq stays shared across accounts, a body reaching two
accounts is one object, both multiplicity reads, by-account listing including the
`NULL` bucket that `IS`-matching exists for, regrouping disturbing no identifier,
and a sync declaring a kind never stealing another account's collection.
