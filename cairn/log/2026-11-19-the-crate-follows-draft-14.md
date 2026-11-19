---
cairn: log
change: the-crate-follows-draft-14
landed: 2026-11-19
---

# The sync vectors state a file with no row

spec/ re-vendored from the pimdir working tree of 2026-11-19 (on 4f2d8fc, uncommitted), still storage-draft-14, sync-draft-14 and search-draft-14, untagged: pimdir's sync vectors move to format 8 for an object's `row` (false: a file under the object's name and no row), and vector 172 states the crash's own state, a partial file under a name with no row, which a fetch sets aside and files again (pimdir two-silent-losses-fold-into-draft-08, last line, ruled on 2026-11-19 to land before the draft-14 tags).

tests/vectors_sync.rs reads format 8: it seeds no row for an object stating `row` false and compares the rows after against it. Vector 172 passes with no change to the engine, whose write verifies a held name from the file alone (src/client/write.rs `write`, through the blob stage); the runner at format 7 refuses every case. spec/REVISION still names da7e01a: it moves to the pimdir commit that carries this change once it is made.

Capabilities moved: none.
