---
cairn: delta
change: store-compaction
---

# Delta

A refactor: no behaviour moves, so the requirements that describe behaviour do not. What moves is one requirement about where the diagnostics live, because folding them onto the store is a change of seam rather than of code.

## MODIFIED Requirements

### Requirement: Diagnostics are a library read
The store SHALL expose what a consistency check asks *about* the index: the object figures (`object_stats`, `live_bytes`, `object_size`), the retention preview (`retained_before`), the index's hash set (`indexed_hashes`), the refcount drift, the ambiguous bindings and the dangling rows. They run on the handle the caller already holds, read-only or owning.

This replaces the rule that they be read from the operator tool's own connection. That rule was sound while the library only *maintained* those invariants: publishing an observation of something nobody could act on would have published an internal. It stopped being sound when the library gained the repairs (`recompute_refcounts`, `clear_dangling_bindings`), since a repair whose findings a caller cannot read is a worse seam than a diagnostic beside it. The cost the rule was paying was two SQLite connections open at once for one command, and a second implementation of reads the library already had.

The drift check SHALL still count exactly what the write path maintains: an item's body, an item's conflict copy, each source's stored base, and each queue row pinning a body.

## REMOVED Requirements

### Requirement: Diagnostics may read the raw index
Replaced by the above. No verb bypasses the library API any more, which is what that requirement was carving an exception out of.

## ADDED Requirements

None.
