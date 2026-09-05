---
cairn: log
change: operator-cli
date: 2026-08-07
---

# The `pimdir` operator CLI

Added a binary target to this crate: `pimdir`, the operator front-end over a
store. Second lib+CLI hybrid in the ecosystem after io-pim-discovery, whose
packaging it copies. New capability `cli`; the `store` capability is untouched.

**Packaging.** `[[bin]] name = "pimdir"` with `required-features = ["cli"]`, and
a `cli` feature outside `default` pulling `client`, clap, humantime and the
pimalaya-cli terminal, table and prompt pieces. A library consumer compiles
none of it. `build.rs` bakes the feature, target and git metadata behind the
same feature, and is a no-op without it. The package gained the release profile
and the `command-line-utilities` category; the description became the hybrid
one, matching the README.

**Where the code lives.** The command modules hang off `src/main.rs`
(`mod cli;`), not off `src/lib.rs`. io-pim-discovery publishes its command
structs as library API; here they stay private to the binary crate, so the
library's public surface and its `#![deny(missing_docs)]` rustdoc are unchanged
by the CLI's existence. The binary consumes `io_pimdir` as an external crate,
exactly as any other consumer does.

**Framing.** `sqlite3`, not a mail client. The tool never interprets item
content: it prints `seq`, link id, flags, level, object hash and the raw meta,
and exports bodies byte for byte. That is the one rule the whole verb surface
is built around, and it is what lets one binary serve a mail store, an address
book and a calendar.

**Roles.** Reads open the store read-only, so inspecting a store mid-sync is
always safe. `item restore` goes through the queue as a producer, then the CLI
takes the owner role and drains that collection itself, reporting *applied*,
*queued (applies at next sync)* when the lock is held, or a pointer to
`queue list --parked` when the drain ran and the item is still not live. The
outcome is read back from the store rather than from the drain's counters,
which describe the whole collection's queue and not this one row. The write
source is resolved *before* anything is enqueued, so an ambiguous multi-source
store refuses instead of creating an item for the wrong side. Purge, queue
cancel and the orphan sweep have no action kind, so they take the owner role
directly; `PimdirError::Busy` reports as "another writer holds the store lock
(a sync is running?)" everywhere.

**Verbs.** `collection list`; `item list` (live or `--retained`, keyset-paged);
`item show` (every placement of one `seq`, retained included); `item export`
(raw bytes to stdout or `--output`); `item restore`; `item purge`
(`<SEQ>` / `--older-than <DURATION>` / `--all`); `queue list [--parked]`;
`queue cancel <ID>`; `store info`; `check [--fix]`; `export <DIR>`; plus
`completions` and `manuals`. `--json` switches every command, logs stay on
stderr.

**Two deliberate deviations from a pure library skin.** First, `check` and the
object figures of `store info` open their own read-only SQLite connection: they
ask about the index's internal consistency (orphan blobs, refcount drift,
dangling rows), which the library maintains rather than observes, so publishing
them as an API would publish the invariants themselves. Second, `check --fix`
deletes orphan blob files without the store's help, guarded by a grace period
(default `1h`) because a body is written before the row referencing it, so a
just-created orphan may belong to a write in flight.

**Safety rails.** Destructive verbs confirm, stating item counts and bytes when
the store can price them, and refuse to prompt into a pipe or under `--json`
(pass `--yes`). Purge refuses a live item outright. Listings never truncate
silently: a short page names the cursor to continue from, and a collection
whose queue payloads could not be decoded is named rather than dropped.

**Left out: `import`.** `cairn/spec/import.md` describes a Maildir / m2dir /
vdir conversion whose every step reads item content (a `Message-ID` out of RFC
822 bytes, Maildir flag letters, a vCard `UID`). That is precisely what this
tool must not do, so it belongs in a per-kind importer. `export` ships alone,
and a future `import` may restore its dumps, which carry only store metadata.

Verified against a demo store (three live items, one retained, a pending queue
action and a planted orphan blob): every verb exercised end to end, including
the restore round-trip (retained → applied → live), the single and time-based
purges, the live-item refusal, the queue cancel, the orphan sweep, the dump and
the JSON output. fmt and clippy clean.

Spec: new `cairn/spec/cli.md` (ADDED: operator tool not a client, feature-gated
packaging, read-only reads, enqueue-then-drain, write source resolved first,
terminal operations take the owner role, confirmation before destruction, no
silent truncation, the verb surface, diagnostics may read the raw index, the
dump's shape, import is not the CLI's job).
