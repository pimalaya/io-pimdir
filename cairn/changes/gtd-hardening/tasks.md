---
cairn: tasks
change: gtd-hardening
---

# Tasks

Waits on pimdir cairn/changes/gtd-hardening.

- [x] Reader and query client probe `task_summary`'s `priority`, `categories`, `parent`, `mid`; tests/older_task_summary.rs fails before the fix ("no such column: s.priority", "no such column: s.categories")
- [x] `PimdirStore::open_current` and `PimdirError::NotCurrent`; tests/open_current.rs
- [x] Re-vendor spec/ from pimdir
- [x] `PimdirSupersedes`, `PimdirAction::Add::supersedes`, the codec
- [x] The gate (add and the superseded remove), the drain (one transaction, park first), the overlay
- [x] The engine's hold (`PimdirLoaded::held`, `list_superseded_pending`); vectors/sync 71 and 72
- [x] tests/engine/supersedes.rs: a parked add and a refused push leave the source in place (each failing before)
- [x] `capability::X_VTODO`, the gate, `PimdirReader::accepts_vtodo`; tests
- [x] `PimdirReader::replied`; tests/replied.rs
- [x] fmt, clippy, `cargo test --all-features`
- [x] CHANGELOG (Unreleased)
- [x] `PimdirSuperseded`, `PimdirLoaded::superseded` (`list_superseded_bindings`), `PimdirChangeKind::Add::supersedes`; tests/engine/supersedes.rs (a moving remote keeping one identity, and one that cannot move), vectors/sync 73; the old engine fails both
- [x] Fold into cairn/spec/, log, status landed (2026-10-29)
