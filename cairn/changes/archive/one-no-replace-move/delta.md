---
cairn: delta
change: one-no-replace-move
---

## ADDED Requirements

## MODIFIED Requirements

### Requirement: A blob write never replaces a file
A blob SHALL move onto its name by `PimdirBlobMove` (STORAGE §5): `renameat2` with `RENAME_NOREPLACE` (`renameatx_np` with `RENAME_EXCL` on Apple); where unavailable or refused, `link(2)` then the temporary unlinked; on Unix where the link is denied or unsupported, an exclusive create and copy. `PimdirBlobMove` is public, so an app writing bodies itself moves them by the same code.

## REMOVED Requirements
