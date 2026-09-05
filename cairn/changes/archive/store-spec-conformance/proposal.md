---
cairn: change
id: store-spec-conformance
status: landed
created: 2026-08-24
---

# Close three gaps against the pimdir specification

## Why

An audit of this crate against the pimdir specification checked out beside it found three places where the store does not do what the format says, all of them invisible until a store written by one version is read by another.

**A 0.2.0 store cannot be opened.** `items.sort_key` and its `items_by_sort` index were folded into version 1 after 0.2.0 shipped, but the §6 draft reconciliation on open never learned about them, so every paged read of an older store fails with `no such column: sort_key`. §6 requires reconcile-on-open or a refusal; this did neither.

**The two schema stamps were never compared.** §4.2 has `PRAGMA user_version` and `store_meta.version` mirror one another and calls a store where they disagree corrupt. This crate read the pragma alone, so a half-applied schema change opened as a store at whichever version the pragma held.

**Unknown flags were written as known-empty.** §13 keeps `NULL` (nothing has read the markers) apart from `'[]'` (the item carries none). io-replica had no unknown state, so a probed placement claimed to carry no markers, which element-wise reads as a removal of every flag another source holds.

## What

Reconcile `sort_key` and its index with the other folded-in columns, and cover the whole set with a test that derives an earlier-draft store from the current schema rather than pasting an old one in, so the next fold is covered without a rewrite.

Compare the stamps on both opens and refuse a disagreement with `PimdirError::VersionMismatch`, leaving a store with no `store_meta` row alone so a missing stamp stays repairable.

Encode `ReplicaFlags::Unknown` (new upstream) as `NULL` and decode `NULL` back to it, in the item row and in a queue payload alike.

## The fourth gap, which is refused rather than fixed

A 0.2.0 store also carries no `ON UPDATE CASCADE`, which `ALTER TABLE` cannot add: reconciling it means rebuilding every table holding a key onto a renamable parent. §6 offers a second branch for exactly this, refusing the store with a message telling the operator to recreate it, and that is what the open now does. The cost is a resync of what the format calls a derived cache; the alternative is a store that silently can never follow a server-side collection rename, failing one dependent row down at the moment a rename is attempted.
