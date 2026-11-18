---
cairn: delta
change: the-default-enqueue-strips
---

## ADDED Requirements

## MODIFIED Requirements

### Requirement: A resource the user did not write is queued without its instructions
`summary::strip_instructions(body)` SHALL remove every property `X-PIMDIR-` names (any case, any group, its folds included) from a vCard or iCalendar resource, keeping every other line byte for byte. `PimdirProducer::enqueue_with_body(collection, body, action)`, the default enqueue of a body, SHALL strip a calendar or address-book body so before it runs, the action built from the stripped body's hash, and queue a body of another kind whole (STORAGE §18). `PimdirProducer::enqueue_user_body` SHALL queue the body whole, for a resource the user wrote as it stands or whose instructions they asked for. `stage_body` and `enqueue` take the body as given. The crate's own enqueues (a restore of a retained item, a tag's flags, a performer's replacing `add`) carry no resource from elsewhere.

#### Scenario: An invitation asking for an online meeting
- GIVEN a source declaring `calendar.online-meeting` `none`
- WHEN a producer enqueues an event carrying `X-PIMDIR-ONLINE-MEETING:TRUE` through `enqueue_user_body`, then through `enqueue_with_body`
- THEN the first is refused and the second queued without the property (tests/capabilities.rs `a_resource_is_queued_without_its_instructions_unless_the_user_asks`, src/summary.rs `an_instruction_is_stripped_whole_and_nothing_else`)

## REMOVED Requirements
