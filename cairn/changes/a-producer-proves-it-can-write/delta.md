---
cairn: delta
change: a-producer-proves-it-can-write
---

## ADDED Requirements

### Requirement: A producer proves it can write without writing
`PimdirProducer::probe` SHALL take the staging lock shared without waiting and release it, and take the database's write lock and roll it back, so an app's check proves the store writable by its producer, the staging lock included, before its first write, writing nothing into the store (objects/ untouched). A collector holding the staging lock SHALL be reported at once as `PimdirError::Busy` (`store-busy`), never waited out. A producer whose staging window is open already holds the lock and probes the database alone.

#### Scenario: A collector holds the staging lock
- GIVEN a store whose objects.lock another process holds exclusively
- WHEN a producer probes, THEN it answers `Busy` at once; once the lock is released it answers Ok, and objects/ holds no file (tests/staging.rs `a_probe_takes_the_staging_lock_and_writes_nothing`)
