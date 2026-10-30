---
cairn: delta
change: sync-audit-2026-10-30
---

## ADDED Requirements

### Requirement: A restated identity keeps the local edit of the old one
The old placement holding a local edit is re-staged as a pending create before the handle names the new identity.

### Requirement: The report says what a run left pending
`unanswered`, `rejected_creates`, and the round closed unless `parked`.

### Requirement: A full fetch answering no body counts nothing
Neither `fetched` nor `upgraded`.

### Requirement: A re-staged edit takes a provisional handle
A rebuild keys a re-staged edit under its provisional handle.

### Requirement: Every old handle is dropped before any upsert
A reused handle is still dropped, `Deleted` when gone, and counted by the guard.

## MODIFIED Requirements

### Requirement: A rebuild keys two copies of one hint apart
The body match the requirement already stated is implemented.
