---
cairn: log
change: format-conformance
date: 2026-08-25
---

# The format moved and half of it arrived, because nothing was watching

pimdir's `retained-page-by-seq` moved the trash listing onto the public `seq` and re-ordered `items_retained` to match. This crate took the statement and left the index, so every trash page sorted every retained row of its collection to return fifty: the exact regression that change was written to remove, reintroduced by taking half of it. The pimdir log named whose entry the other half was.

`spec_drift` caught it. Nothing ran `spec_drift`.

## What landed

- **`items_retained` is `(collection, seq)`**, and `RESHAPED_INDEXES` drops an index whose columns moved before the ensure batch runs. `CREATE INDEX IF NOT EXISTS` keys on the *name*, so the batch could not have repaired one: it finds an index already there, and the store keeps planning the read the way the schema no longer says. Checked rather than dropped unconditionally, since rebuilding a large store's index on every open is the cost this avoids.

- **`tests/objects.rs`**, against `vectors/objects.json`. The one vector file the format makes a **MUST** was read by nobody, while the `SHOULD` one was checked: every body, under both `hash_algo` values, whole and streamed in seven-byte pieces, with the shard path each name derives. `PimdirBlobs::path` is public so that path is reachable at all, which SPEC §14 wants anyway for a consumer streaming a body straight to it.

- **`every_canonical_statement_prepares`**, against the canonical schema. The name check said the crate reaches each statement and never loaded one; this repository holds the only toolchain that ever reads the format's SQL, so a spec edit naming a column the migration lacks would have been found by whichever consumer ran it first. Verified by injecting a typo into `queries/objects.sql` and watching it fail.

- **CI, in both repositories, having had none.** io-pimdir's checks out pimdir and io-replica so its three spec suites run; pimdir's checks out io-pimdir and runs them against the pull request. Each asserts the suites *ran*: they skip silently without the sibling checkout, so a green run would otherwise prove nothing. That silence is the whole story of this change.

- Two stale claims went with it: `sql.rs` and `tests/spec_drift.rs` both described the `seq` cursor as a deliberate §4.4 substitution, and it is the reference form now. The `Cargo.toml` note explaining the io-replica path dep is back, with what it costs (this crate cannot be published while it stands).

## Verification

104 tests green, `cargo clippy --all-targets --all-features` clean, `cargo fmt`. The reshape check was verified by emptying `RESHAPED_INDEXES` and watching the new test fail.

Capabilities moved: `store`, `spec-fidelity`.
