---
cairn: log
change: the-empty-span-matches-nothing
landed: 2026-11-25
---

# The empty span matches nothing

pimdir SEARCH §8.1 now states that a span whose end is not after its start is empty and matches nothing, whatever the kind (pimdir cairn/log/2026-11-25-the-empty-span-matches-nothing.md, a reading taken on live night 3 for the owner to confirm). d5f2b89 matched an occurrence running across the instant of `date:2w..2w` by the overlap rule.

src/client/search/compile.rs `date` renders an empty span as a term matching nothing, its set empty. The generator's oracle, spec/generator/src/query/date.rs `Span::meets`, answers false for an empty span; `an_empty_span_meets_no_occurrence` pins it. The vectors `collection:home/Agenda date:1d..1d` and `date:1d..2d` failed on d5f2b89 (the first answered the all-day event of 1 January) and pass. The generator's revision stays 1: no seed draws another history.

The change touches the engine, so the freeze plan's stop rule asks for new campaigns on the commit carrying it (campaign-4 and campaign-5, tests/generated/campaigns.json).
