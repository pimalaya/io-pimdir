---
cairn: log
change: the-binary-follows-pimalaya-cli-0-5
landed: 2026-11-15
---

# The binary follows pimalaya-cli 0.5, the extraction mime-rs 0.2

mime-rs moved to 0.2, which reads a `Content-Type`, `Content-Transfer-Encoding` or `Content-Disposition` an entity states twice from its first occurrence. The summaries and the search extraction already read the message as `summary::mail::first_occurrences` hands it, so nothing they produce changes: the summary, search and walk-agreement tests, the fixture mail-repeated-part-fields.eml included, pass unchanged. No mime-rs type is in the public API.

The `cli` feature's pimalaya-cli moved to 0.5 from its git pin, so the `pimdir` binary follows the family's exit contract: a command printing its result short of full success exits 3, where it exited 2, now the status of a refused command line. The `--help` of check, export, store backup and restore, quarantine empty, item restore and tag says 3. Tests: tests/cli_json.rs `a_check_leaving_a_problem_exits_3`, tests/cli_copies.rs `a_deep_check_reports_a_flipped_byte` and tests/search_cli.rs `a_tag_stages_each_placement_as_the_source_syncing_its_collection` assert 3.

Capabilities moved: cli (cairn/spec/cli.md, the partial-outcome requirement, its scenario and the store backup and restore line).
