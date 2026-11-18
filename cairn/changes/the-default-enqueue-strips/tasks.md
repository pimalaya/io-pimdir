---
cairn: tasks
change: the-default-enqueue-strips
---

# Tasks

- [x] `enqueue_with_body` strips, `enqueue_user_body` keeps, `enqueue_foreign_body` gone
- [x] tests/capabilities.rs `a_resource_is_queued_without_its_instructions_unless_the_user_asks`, failing on the old code (the default enqueue refused the online meeting instead of stripping it)
- [x] cairn/spec folded, the log written, CHANGELOG.md
- [ ] calendula moves its own online-meeting path to `enqueue_user_body`
