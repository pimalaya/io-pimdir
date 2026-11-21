---
cairn: log
change: the-crate-follows-draft-15
landed: 2026-11-21
---

# The replay findings lifted

spec/ re-vendors pimdir after the owner's decision on G23 (2026-11-21: a rekey runs the flag axis on the flags its listing states, SYNC §8) and the generator following the spec text on G14, G24, G25 and G26 (pimdir cairn/log/2026-11-21-a-rekey-runs-the-flag-axis.md). spec/REVISION reads `PENDING` until the pimdir commit is made; the drift test compares nothing while no pimdir checkout holds it.

## What moved

- spec/: SYNC §8's vector vectors/sync/195-a-rekey-runs-the-flag-axis-on-its-listed-flags.json, vectors/manifest.json, and spec/generator/src/engine.rs and history.rs. io-pimdir passes vector 195 unchanged: it already merged a rekey's listed flags over the old base (src/rekey.rs).
- tests/generated/replay.rs: G14, G23, G24, G25 and G26 leave the known findings, with the readings that named them (`diverges`, `readings`, `rekey_pulls`, and the upgrade and shared-conflict arms of the event comparison). Every history replays clean of them; the `#[ignore]` twin now stands for G12, G13 and G15 to G17 alone.

No production code moved. G12 (bodies step 18) stays as it was.

Capabilities moved: none (tests and the vendored spec only).
