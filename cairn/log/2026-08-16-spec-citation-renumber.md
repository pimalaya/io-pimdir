---
cairn: log
change: spec-citation-renumber
date: 2026-08-16
---

# Follow the pimdir spec renumbering

pimdir reordered its sections and moved the per-kind `meta` conventions to an annex ([its log entry](https://github.com/pimalaya/pimdir/blob/master/cairn/log/2026-08-16-spec-restructure.md) carries the old-to-new table). Nothing about the format changed, but every citation of a moved section now names the wrong one, and this crate cites the spec heavily: the read API, the queue, retention and the encodings are all documented against it.

## What landed

The citations in src/, tests/ and cairn/spec/ follow the new numbering: `§7` and `§8` swapped (concurrency and integrity), `§11` became `§13` (encodings), `§12` became `§14` (operations), `§14` became `§15` (queue, with its subsections), `§15` became `§12` (collection generation), `§16` became `§11` (retention), and `§13` became Annex A (the meta conventions, now marked informative).

Doc comments and test comments only. No statement, no schema and no behaviour moved, and `tests/spec_drift.rs` still checks the inlined schema and the statement set against the canonical checkout, which it passes unchanged: the restructure touched no SQL.

## What was left alone

`cairn/log/` and `cairn/changes/`. A log entry is immutable by the convention, and a landed proposal records what was argued at the time; both are resolved by pimdir's mapping table rather than by rewriting them.
