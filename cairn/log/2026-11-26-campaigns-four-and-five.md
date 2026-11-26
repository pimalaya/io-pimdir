---
cairn: log
change: campaigns-four-and-five
landed: 2026-11-26
---

# Campaigns four and five

The empty-span reading of 2026-11-25 (cairn/log/2026-11-25-the-empty-span-matches-nothing.md) moved the engine, so the freeze plan's stop rule (pimdir cairn/changes/the-freeze-plan, D5) restarted its count on 654b1fd with campaign-4 and campaign-5 (tests/generated/campaigns.json).

- Campaign-4 on 654b1fd: 4,180 jobs in 8,806 s on 32 workers, no seed failing, 25 differences all of the accepted `sender` class.
- Campaign-5 on 654b1fd: 4,180 jobs in 9,424 s, three jobs failing (structure 5036 drain and search, histories 5109 replay), five differences, each a `references_from` read answering one `sender` reference the harness did not accept. None in the engine, none in the standard: both are the harness's.

## The findings, the harness's

**A `sender` reference is owed only after a write at one end.** Shrunk (`PIMDIR_SHRINK=structure:5036:drain`) to a mail from `lena.moreau@example.net`, a card already holding that address and a queued update giving it to a second card. io-pimdir records the reference to the updated card and none to the other: the rule runs after the write that changed what it reads (STORAGE §14.2), and the generator writes its store with no reference, so nothing wrote the mail or the first card since. The harness demanded, from a mail, every reference the rule states over the current model.

**A tombstone is owed no reference.** Shrunk (`PIMDIR_SHRINK=histories:5109:replay:sources:37`) to a mail pulled from `hugo.bernard@example.org` while a live card and a retained one hold that address. io-pimdir records the reference to the live card alone, as STORAGE §14.2 states (both ends live when recorded). The harness's rule counted the retained card.

tests/generated/scale.rs `owed` states what a store must hold: the pairs the rule states between live rows of the current model and not over the model the store was written from. The drain, search, replay and scale suites check a mail's references against it; what a store may hold (`record`) is unchanged. Each owed set is a subset of the old one, so no run the old harness passed (campaign-4 included) fails on the new.

Pinned: tests/generated/drain.rs `a_sender_reference_is_owed_only_after_a_write_at_one_end` (fails on the old harness) and tests/generated/replay.rs `a_sender_reference_is_owed_to_a_live_card_alone` (fails without the live filter). The three campaign-5 keys shrink to no difference on the new harness.
