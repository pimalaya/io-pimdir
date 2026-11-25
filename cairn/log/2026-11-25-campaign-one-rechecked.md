---
cairn: log
change: campaign-one-rechecked
landed: 2026-11-25
---

# Campaign one rechecked

Campaign-1 (tests/generated/campaigns.json) ran once on e2aad6e: 4,180 jobs, 6,902 differences, every one traced to the findings cairn/log/2026-11-25-the-quirks-findings-close.md closes, Q1 among them. Run again on b5606f3 over its structure, histories, count and bytes profiles (1,620 jobs, 2,827 s on 32 workers), it left three seeds failing, all the harness's, none the engine's:

- **A writer-derived key is no reference endpoint.** At structure seed 1026 two cards state the address of the mail `report.37@example.net`, one keyed `hash:80c7f51e6f7337aa`. io-pimdir records the `sender` reference to the other card alone, as STORAGE §9 and §14.2 ask; tests/generated/scale.rs `senders`, the rule the harness accepts io-pimdir's automatic references by, named the `hash:` card too and asked for it. It now leaves out an end under `alt:`, `dup:` or `hash:`, as the owner's statements do.
- **The search oracle answers over the references the store records.** At structure seeds 1016 and 1062 io-pimdir answered `linked:` with a card it had related by the `sender` rule, which the oracle's model, holding the profile's references alone, did not hold. tests/generated/search.rs `ruled` gives the oracle's corpus each automatic `sender` reference the store records that the rule states, the reader comparison of the same step having checked them. The rule's whole set would be wrong: an owner applies it to what a write touches (STORAGE §14.2), so a store as the generator wrote it holds none of the rule's references.

Both fail the old harness and pass the new one at the three seeds, drain and search suites alike.

Left open, the shrinker's: a search suite difference's signature is its words up to the first value, "the model does not follow the write:", which any reader difference of that suite shares, so `PIMDIR_SHRINK=structure:1026:search:1` shrank the store to no item while another difference stood. The reader rows' statements belong in that signature.
