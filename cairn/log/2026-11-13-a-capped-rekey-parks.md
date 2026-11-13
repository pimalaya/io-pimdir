---
cairn: log
change: a-capped-rekey-parks
landed: 2026-11-13
---

# A capped rekey parks

spec/ is pimdir at e443a65, the drafts 11 as d8800fb carried them with one rule the standard added before the tags: a rekey from a capped listing parks on any absence (SYNC §8). The copy gained one sync case, 159, and its line of the manifest; it was vendored before this pass, at the commit, and nothing under spec/ was edited here. The crate follows the rule, and two cleanups the draft-11 follow left ride with it: `PimdirSearchReads::counts_bodiless` answers the whole rule it answered half of, and a mail `date` the store takes as none feeds neither key. No draft name moved, no schema, no statement, no vector format; tests/pending_vectors.txt stays empty.

## What followed

- **A rekey from a capped listing parks on any absence** (SYNC §8, vector 159). `PimdirRekey` ignored `PimdirRemoteSnapshot::capped`, so a capped collection renumbered dropped `Deleted` every binding its listing left out, under the half of twenty that parks. It now folds the cap off its pages and writes nothing while any based binding of the source resolves to no member, whatever their number, unless the run carries `accept_absences`; `PimdirRekeyReport::parked` carries the counts, the checkpoint and the generation unmoved as for the two parks the rekey already had. Accepted, or with every based binding resolved, the rekey is written as any other. An uncapped rekey is what it was: `PimdirParked::guard` reads the same counts.
- **`counts_bodiless` is the whole rule** (SEARCH §8.2). It answered `mark && kind == mail`, the `has:attachment` half, so `is_complete` joined it to `kinds.contains(kind)`, the CLI's text restated both halves over its own row, and every consumer wrote the `or` itself. It now answers whether a `bodiless` item of a kind counts against the answer: a term reads a body of the kind, or the kind is mail and the query holds `has:attachment`. `is_complete` asks it for the `bodiless` count and reads `kinds` for the `missing` and the `unparseable` ones alone; `CoverageRow`'s text builds a `PimdirSearchReads` from its `ReadsRow` and asks it too. The signature is what it was, and `reads.kinds.contains(kind) || reads.counts_bodiless(kind)` answers the same as the call alone. No answer, no JSON and no printed line moved.
- **A date the store takes as none feeds no key** (STORAGE Annex A.1, "Hint" and "`sort_key`"). `PimdirSummary::stored` writes a mail `date` of another shape than `YYYY-MM-DDTHH:MM:SSZ` as none, but `PimdirMailSummary::sort_key` and the `alt:` key of `link_id` read the stated text, so a row dated none sat under a sort key and an `alt:` key naming it. Both read `summary::mail::usable_date` now: none sorts as an undated message (`''`) and is the empty part of the key (`alt:Hi||a@x.y`). The derivation writes no other shape than the usable one, so no key it derives moved: the 73 summary cases pass whole.

## Behaviour chosen where the text was open

- **One page stating the cap is enough.** §8 says "`capped` on its pages" and §4 has a connector set it on every page of the collection's rounds; §5 reads a round's last page. A rekey reads every page before its batch, so it takes the cap from any of them, which is what checks/vectors.py does (`any(page.get("capped") ...)`). A listing restarted over a rejected cursor is read anew, the cap with its members.
- **An edit the cap left out is an absence.** "Any based binding of this source in the collection resolves to no member": a binding holding an edit no member resolved to is one, though the rebuild would not drop it `Deleted` but re-stage it as a pending create, which the next sync would add beside the member the cap left out. Under a cap the count is every based binding no member resolved to, a delivered create aside (§8), and the park reports that count. Accepted, such a binding is re-staged as it is under no cap.
- **The uncapped guard is not moved.** It counts the `Deleted` drops of based bindings, so an edit no member resolved to is not among its absences, where §8 has a binding absent "when no member resolves to its link id" and checks/vectors.py counts it. Aligning it would park a rekey the crate writes today (src/rekey.rs `an_unmatched_staged_edit_resurrects_as_a_pending_create` states one: a listing naming no member over one edited binding). It is the owner's to decide, listed under Left.

## Problems found

No vector contradicts its prose: 159 expects what §8 says, one absent of three parked with the rows as they were. Read on the way, none changed here:

- **Two counts of a rekey's absences**, above.
- **A tombstone no member resolves to.** checks/vectors.py leaves a binding whose item is held deleted out of a rekey's absences; the crate counts its `Deleted` drop, under a cap or not. No case states one.
- **`summary::same_message` compares the stated date.** src/summary.rs reads `held.date == listed.date`, the held one as the store took it and the listed one as the connector stated it, so a member dated in another shape than the store takes would not agree with its own stored row where another source binds it (src/client/write.rs, the naming of a page) nor with the create it delivers (src/sync.rs `disagreeing_create`, `take_pending_create`). Read off the code, not run, and no case states such a member. `PimdirHeldObject::agrees` reads the usable date on both sides already.

## Guards

Each seen to fail with the behaviour before put back by hand in the working tree for one run, the file then restored from a copy kept beside it (never through git):

- the cap unread (`self.capped |= page.capped` taken out, the crate as it stood): src/rekey.rs `a_capped_rekey_parks_on_one_absent_member`, `a_capped_rekey_parks_on_an_edit_no_member_resolves_to` and `a_cap_stated_on_any_page_caps_the_rekey`, each on "expected a parked rekey"; sync vector 159 failed on the crate as it stood, before any edit ("the park differs", `Some(PimdirParked { absent: 1, bound: 3 })` expected and `None` answered);
- `accept_absences` unread under a cap: `a_capped_rekey_whose_absences_are_accepted_is_written`;
- a cap parking whatever resolves: `a_capped_rekey_every_binding_resolves_in_is_written`;
- every rekey read as capped: `an_uncapped_rekey_missing_one_member_is_written`, with four earlier tests of the module;
- the `Deleted` drops alone counted under a cap: `a_capped_rekey_parks_on_an_edit_no_member_resolves_to`;
- the last page alone read: `a_cap_stated_on_any_page_caps_the_rekey`;
- `counts_bodiless` answering the mark's half: src/search/coverage.rs `a_bodiless_item_counts_under_a_read_of_its_kind_or_mails_mark`, and with it `a_body_out_of_sight_counts_only_against_a_term_that_reads_it`, `a_bodiless_message_counts_against_its_attachment_mark` and src/cli/search.rs `a_coverage_prints_the_counts_its_query_reads` (the `bodiless` line unprinted under a term reading mail), which now reach the rule through it;
- the keys reading the stated date (the crate as it stood, the tests written first): src/summary/mail.rs `a_date_the_store_takes_as_none_sorts_as_no_date` and `a_date_the_store_takes_as_none_is_the_empty_part_of_an_alt_key` (`alt:Hi|2026-08-01T10:00:00.5Z|a@x.y` for `alt:Hi||a@x.y`).

Three of the rekey tests state what the crate did before and pass on it: the accepted one, the one every binding resolves in and the uncapped one. They were seen to fail on the wrong rules above, not on the old code. No test was rewritten: none stated a rule that moved.

## Evidence

Run on the working tree on 2026-11-13, each as `nix develop --command <cmd>`, a pimdir checkout holding e443a65 beside it:

- `cargo fmt --all --check`: clean.
- `cargo clippy --all-features --all-targets -- -D warnings`: clean; so are `--no-default-features --features cli`, `--no-default-features --features search`, `--no-default-features` and the defaults.
- `cargo test --all-features`: 943 passed, 2 ignored (the two scale measurements).
- `cargo test --no-default-features --features cli`, the system SQLite: 943 passed, 2 ignored.
- `cargo test --no-default-features --lib`, the no_std core: 398 passed.
- `cargo test --no-default-features --features search`, search with no SQLite: 446 passed.
- `cargo test`, the defaults: 824 passed, 1 ignored.

## Left

- The pin in pimdir's checks/io-pimdir.rev, the three tags, and spec/REVISION renamed to the commit they name, as the draft-11 follow left them.
- **The uncapped guard's count**, above: whether an edit no member resolved to, and a tombstone, are absences of a rekey no cap bounds.
- **`same_message` over a date of another shape**, above.
- The consumers follow. A connector to a source that caps a collection, neverest's and the CLIs' backends: `capped` on every page of the rekey's listing as on a round's, and a rekey that may now come back `parked` on one member left out, to be run again with `accept_absences` once its caller confirmed. A search client, himalaya, cardamum, calendula and the Android app: `reads.counts_bodiless(kind)` alone where it wrote `reads.kinds.contains(kind) || reads.counts_bodiless(kind)`, the longer form answering the same. A connector building a `PimdirMailSummary` by hand: a `date` in another shape than `YYYY-MM-DDTHH:MM:SSZ` gives an undated sort key and an `alt:` key with an empty date, as its row is stored.
