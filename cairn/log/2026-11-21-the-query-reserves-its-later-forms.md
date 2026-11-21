---
cairn: log
change: as-fast-as-notmuch
landed: 2026-11-21
---

# The query reserves its later forms

Phase 4's first item of as-fast-as-notmuch: the crate follows pimdir f22d990 (cairn/log/2026-11-21-the-query-reserves-its-later-forms there), search-draft-16 edited in place, spec/ re-vendored (coverage.sql, the search vectors, their schema and the manifest) and spec/REVISION moved.

- `PimdirQuery::parse` refuses `reserved-form` (`PimdirQueryErrorCode::ReservedForm`, src/search/query.rs `reserved`): a text or a value beginning with `~`, one of two characters or more beginning with `/`, a value beginning with `{`, after the field and an empty value, before the wildcard; a phrase is never one. Until now `~meeting` answered as `meeting`, `/meet/` as `meet` and `thread:{from:jane}` as nothing.
- `PimdirQueryErrorCode` loses `#[non_exhaustive]` (owner, 2026-11-21: never `#[non_exhaustive]`). Five other types keep it (src/remote.rs twice, src/client.rs, src/sync.rs twice), left for their own change.
- Coverage counts an unknown extraction status as `unparseable` through the vendored canonical `coverage`; the crate reads `object.status` back nowhere else, so no Rust changed for it.

Capabilities moved: search.md (the refusal requirement, the coverage requirement). Checks: tests/search.rs over the 22 new cases, which fail with `reserved` disabled (seen 2026-11-21: `kind:~mail` answered `invalid-value`, the rest no refusal); tests/spec_drift.rs against pimdir f22d990.
