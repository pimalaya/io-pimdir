---
cairn: log
change: as-fast-as-notmuch
landed: 2026-11-21
---

# Text counts match whole documents

The floor left by 2026-11-21-text-counts-off-the-index: a count of several text terms read each term's placements, one `placement_by_object` seek per matched body, and combined them per hit. FTS5's own `AND` matches a document, a body or a bodiless placement's summary, while SEARCH §8.2 matches a hit; the two agree for every hit whose placements name one document. pimdir now records the others (search-draft-16 edited in place, pimdir log 2026-11-21-a-split-hit-is-recorded): the `split` table, `placement_by_seq`, and the statements `split_forget`, `split_mark` and `split_seqs`, vendored under spec/ (spec/REVISION left `UNSET` until pimdir's commit).

- **The upkeep** (src/client/search/refresh.rs): after each `Writes` yield's ops and full-text writes, `split_forget` then `split_mark` for every public id a `PutPlacement` or a `DropPlacement` named, the reconciliation of keys included. The coroutine is unchanged: the consumer derives the public ids from the ops.
- **The joint count** (`PimdirIndex::marked_jointly`, `Compiled::joint`, `Compiled::text_of`): on a current index, for the shapes `Compiled::bitwise` admits and while `split` holds at most `SPLIT_MAX` (1,024) public ids (`SPLIT_HELD`, the crate's own), one FTS5 expression over the bodies and one over the summaries read through `match_objects` and `match_summaries`, each matching document's placements marked per account; then each public id `split_seqs` answers unmarked and evaluated per account through `match_object` and `match_summary`, combined by `holds`. Past the cap, each term's placements are read as before.
- **Exact.** A placement never holds both a body and summary text (SEARCH §4), so a hit whose public id `split` does not hold names one document and meets the expression exactly when that document matches; a document with no full-text row matches no term, and every admitted shape answers no hit then. A split hit is evaluated term by term as before.

The crate-own prototype the numbers below were measured on (`SPLIT_SCHEMA` beside the migration, its own statements, `SHAPE_DECLARED` over both) is gone: the migration carries the table, so the shape check and `init` read `MIGRATIONS` alone, and an index built before is rebuilt, its schema text differing.

## Proof

- refresh.json's case "a hit holding two bodies holds a text term through either" (refresh.json `v` 5, a step's `split` compared on the refreshed and the rebuilt index in `every_refresh_vector_answers`): without `split_mark`, steps 1 and 3 fail on their `split` rows (`[]` for `[1]`), and with that comparison skipped on `quince rhubarb` counted 0 for 1.
- tests/search.rs `a_hit_holding_two_bodies_counts_its_terms_across_them` (a body-only field, a negated `or`, the split rows through a repoint, back, and a collection removal): fails at `apple banana` counted 1 for 2 without `split_mark`, and likewise without the per-placement correction.
- tests/search_plan.rs caught `split_seqs` aliasing `split` as `s`, which its guard reads as the store's items: the statement now names the table whole, its plan walking `split` and seeking `placement_by_seq`.

## Measured

On the 517k Enron bench (2026-11-21, hyperfine `-N -w 3 -r 15` under `taskset -c 28`, medians; core 28 shares a physical core with 12, where another bench ran, so single runs are noisy): no public id is split there, each seq holding one placement. Every count equal to the earlier build's. `power california` 43 to 13 ms (notmuch 5.5), `california not gas` 47 to 17 (6.4), `california or texas` 37 to 33; `enron`, `california`, `kiwi`, `"natural gas"`, `deriv*`, `from:dasovich` and `subject:meeting` unchanged within noise (40-run reruns: the phrase 19.0 and 18.3, `california` 20.3 and 23.0, then 21.3 and 21.7). index.db 1,887 MB against 1,881 (+6.7 MB, `placement_by_seq`); a full build 212 s against 222 (an earlier pair 216 and 348, I/O-bound); a refresh of 11,834 re-flagged placements 1.65 to 1.73 s either way.

## Left

The seek from each matched body to its placement remains, about 0.2 µs a row: `power california` spends about 3 ms in FTS5 and the rest joining and starting the process, against notmuch's 5.5 ms whole.

Capabilities moved: search.md (a new requirement, "A text count matches whole documents, a split hit per placement"; "A count of text terms reads the index alone once it is current" names it), spec-fidelity.md is unchanged until spec/REVISION is set.
