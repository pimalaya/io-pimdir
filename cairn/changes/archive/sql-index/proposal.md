---
cairn: change
id: sql-index
status: landed
created: 2026-08-08
---

# A name-to-statement index, so a non-Rust consumer can run the canonical SQL

## Why

The crate's `sql` module is deliberately dependency-free: `client` (and with it
rusqlite) is the only thing behind a feature gate, so a consumer that already has
a SQLite driver can take the canonical schema and statements and run them itself.

The Pimalaya Android app is exactly that consumer. Android ships SQLite, and its
storage seam already drives `android.database.sqlite` from Java through a JNI
upcall. Bundling rusqlite would compile a second SQLite engine into every ABI of
a binary whose first design goal is to be small. So the app takes `sql` without
`client` and executes the statements on the platform driver.

To do that it has to reach the statements **by name** across the JNI boundary.
Today each is a separate `pub const`, reachable only by writing one accessor per
statement, or by transcribing all sixty into Java and letting them drift from the
spec silently. Neither is acceptable for something whose whole value is being the
canonical copy.

## What (design)

`sql::ALL`, a `&[(&str, &str)]` pairing every statement's constant name with its
text, ordered as the module declares them. A consumer serializes it once and
looks statements up by name.

`MIGRATION_0001` is included: a consumer creating the database needs it exactly
as much as it needs the statements, and excluding it would be a special case to
remember. `VERSION` is excluded, being an integer rather than SQL.

**A drift guard, not a promise.** The index is hand-written, so a statement added
later could miss it. A test reads the module's own source with `include_str!`,
extracts every `pub const` name, and asserts the index covers all of them bar
`VERSION`. Adding a statement without indexing it therefore fails the suite
rather than silently shipping a gap.

## Scope / non-goals

- No macro rewrite of the sixty constants. A macro that defined and registered
  each would also guarantee coverage, but it would obscure sixty readable
  statements to solve a problem one test solves.
- No parameter metadata. Which `:name` a statement binds is discoverable from the
  text, and duplicating it would be a second thing to keep in sync.
- No change to any statement, or to the `client` driver.
