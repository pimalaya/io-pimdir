---
cairn: change
id: operator-cli
status: landed
created: 2026-08-07
---

# The `pimdir` operator CLI

## Why

A pimdir store is a SQLite index plus a blob directory, and until now the only
way to look inside one was to write a Rust program against the library or to
open `pimdir.db` with `sqlite3` and know the schema by heart. Three concrete
gaps follow from that:

- **Parked actions have no user-facing surface.** The store records them and
  never deletes them, on purpose, but nothing shows them to a human.
- **Orphan blobs have no cleaner.** The write path deliberately unlinks blob
  files after the commit, so a crash leaves at worst an orphan; nothing
  detects or reclaims one.
- **A queued action cannot be cancelled.** Only the owner pops rows, so a
  producer can enqueue a mutation and never take it back.

Retention adds a fourth: once a removal retains the row instead of deleting
it, an operator needs to see what is retained, restore one item, and reclaim
the space on purpose.

## What (design)

A binary target `pimdir` in this same crate, behind a `cli` feature that is not
in `default`, so a library consumer never compiles clap. This is the second
lib+CLI hybrid in the ecosystem after io-pim-discovery, whose packaging is
copied verbatim (a `[[bin]]` with `required-features`, a build script baking
version metadata, the pimalaya-cli printer, logger, `--json` flag, `manuals`
and `completions` subcommands).

**The framing is `sqlite3`, not a mail client.** pimdir is kind-agnostic, so
the CLI never interprets item content: it prints `seq`, `link_id`, flags,
level and the raw meta JSON, and exports raw bytes. Rendering a message or a
vCard belongs to himalaya and cardamum.

**Roles.** Reads open the store read-only, so inspecting a store while a sync
runs is always safe. Item mutations go through the queue (the producer role),
then the CLI drains that collection itself when the owner role is free,
reporting "applied" or "queued, applies at next sync". Purge, queue cancel and
the orphan-blob sweep are terminal store operations with no queue action kind,
so they take the owner role directly and report a clear message when another
writer holds the lock.

**One deliberate deviation from the library-skin rule.** `check` and the object
figures in `store info` (object count, bytes live versus retained, refcount
drift, dangling references, orphan blobs) have no library API and should not
get one: they are diagnostics over the raw index, exactly what an operator
tool is for. The CLI opens its own read-only SQLite connection for those, and
only those.

**Where the code lives.** The command modules hang off `src/main.rs`
(`mod cli;`), not off `src/lib.rs`. io-pim-discovery publishes its command
structs as library API; here they stay private to the binary crate, so the
library's public surface (and its `#![deny(missing_docs)]` rustdoc) is
untouched by the CLI.

## Scope / non-goals

- **`import` is out.** `cairn/spec/import.md` documents a Maildir / m2dir /
  vdir conversion: reading a `Message-ID` out of RFC 822 bytes, decoding
  Maildir flag suffixes, reading a vCard `UID`. Every one of those *interprets
  item content*, which is the one thing this tool must not do. It belongs in
  a per-kind importer (himalaya, cardamum, or a dedicated tool), not here.
  `export` ships alone.
- **No repair beyond orphan blobs.** `check` reports refcount drift and
  dangling references; it does not rewrite refcounts or delete rows, because a
  wrong repair is worse than a reported inconsistency.
- **The CLI drains a whole collection's queue, not only its own row.** The
  queue is per collection, not per producer, so a drain triggered by a restore
  also applies whatever else was pending there. That is safe because the store
  gained the third drain outcome in this same release (an action kind this
  build cannot perform is *skipped* and left pending, never parked), so a
  foreign intent survives the pass untouched. The CLI still drains only the
  collection it just enqueued into, and reads the item back rather than
  trusting the drain's collection-wide counters.
