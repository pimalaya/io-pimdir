---
cairn: log
change: the-crate-follows-draft-15
landed: 2026-11-20
---

# The vendored spec carries the generator

spec/REVISION names pimdir 5371b73: storage-draft-15, sync-draft-15 and search-draft-15 as 476c6a0 opened them (spec/migrations/, spec/queries/, spec/vectors/ and spec/payloads/ unchanged from 1460f07), and pimdir's generator crate (D1 of pimdir the-freeze-plan), vendored as spec/generator/ without its target/. pimdir's checks/implementation.sh copies it the same way, so the crate can take it as a path dev-dependency once the differential tests start; none is declared yet.

- tests/spec_drift.rs compares generator/ with the revision as it compares the four other directories: a byte appended to spec/generator/README.md fails `the_vendored_copy_is_the_spec_byte_for_byte` ("re-vendor it: [\"generator/README.md\"]"), and the copy passes.
- Cargo.toml leaves spec/generator/ out of the package.
- build.rs, tests/spec_drift.rs, README.md and cairn/spec/spec-fidelity.md name the five directories. spec-fidelity.md named pimdir da7e01a beside the draft-15 names, a slip of 1460f07; it now names 5371b73.

Capabilities moved: spec-fidelity.
