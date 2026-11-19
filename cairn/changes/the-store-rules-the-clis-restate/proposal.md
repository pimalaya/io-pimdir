---
cairn: change
id: the-store-rules-the-clis-restate
status: landed
created: 2026-11-19
---

# The store rules the CLIs restate

## Why

pimdir audit-2026-11-12 C5 (medium, any): himalaya, cardamum and calendula each carry a src/pimdir tree restating store rules, and the copies differ. Re-read on 2026-11-19 at their working trees:

- **Account resolution.** `resolve_account` in himalaya src/pimdir/client.rs, cardamum src/pimdir/client.rs and calendula src/pimdir/client.rs: the configured account checked against the store's, else the one account holding collections of the app's kind, else `store-account-ambiguous`. They differ: himalaya refuses a configured account on a store holding no collection at all (`store-account-unknown`), cardamum and calendula accept it; cardamum and calendula count an empty `kind` as theirs, himalaya does not.
- **The kind filter.** `is_mail` (himalaya src/pimdir/backend.rs), `is_card_kind` (cardamum src/pimdir/backend.rs), `is_calendar_kind` (calendula src/pimdir/client.rs): the bare media type before `;`, the rule `collection::role_of_kind` and `summary::derive` already apply inside io-pimdir, written three more times.
- **Query scoping.** cardamum src/pimdir/index.rs `scope`, and the same in calendula and himalaya: the query wrapped in a top-level `And` with `kind:`, `account:` and `collection:` terms, top-level because that is where io-pimdir reads coverage (SEARCH §8.2), a constraint only io-pimdir knows.
- **Completeness.** Each CLI turns `PimdirSearchCoverage` into an `Incomplete:` line, counting a bodiless, missing or unparseable item only while a term reads a body of its kind (O5), and reporting the index lag (`behind`, `unindexed`); io-pimdir's own CLI (src/cli/search.rs, `Coverage:`) reports the lag too, so the line's "without lag" no longer holds, but the reading rule is still written four times.
- **The quiet overlay.** himalaya src/pimdir/backend.rs `overlay_quiet`: whether no queued action changes what a read of a collection shows, a rule of STORAGE §15 and of `PimdirReader`'s overlay, which only himalaya restates.

## What io-pimdir would expose

All additive, under `client` (pure functions over rows io-pimdir already returns), each CLI keeping its policy and every message:

1. `collection::media_type(kind) -> &str`: the bare media type, the one reading of a kind.
2. `PimdirReader::accounts_of(kind: Option<&str>) -> Vec<Option<String>>`: the accounts holding collections of a media type, sorted, deduplicated, `None` for the ungrouped. The choice (configured, only, ambiguous) and its refusals stay in each CLI.
3. `PimdirQuery::scoped(self, kind, account, collection) -> PimdirQuery`: the top-level conjunction io-pimdir reads coverage from.
4. `PimdirSearchCoverage::unread_counted(&self) -> PimdirSearchUnread` (or a `gaps()` list): the counts a query's terms actually read, so every frontend states the same incompleteness.
5. `PimdirReader::overlay_quiet(collection) -> Result<bool, PimdirError>`: the overlay's own rule, beside the overlay.

## Decision asked

- Which account rule is right for a configured account on a store holding nothing yet: refused (himalaya) or accepted (cardamum, calendula)? And does an empty `kind` count as calendar or contacts (it is allowed by the schema, `collections.kind IN ('', ...)`)?
- Recommendation: accept a configured account on an empty store (a first sync has not run; the refusal reads as a misconfiguration that is not one), and count an empty kind as no kind of any app. Then land 1 to 5 together in one io-pimdir minor and move the three CLIs in one pass, after v1: C5 is class "any", nothing frozen moves.

Not implemented on 2026-11-19: each helper only pays off when the three apps, edited by other sessions, drop their copy, and the account rule needs the ruling above first.

## Ruled (2026-11-19)

The owner took the recommendation whole: a configured account is accepted on a store holding no collection yet, an empty `kind` belongs to no application's kind, and the five helpers land in io-pimdir now, additive, each CLI moving in its own session once a release carries them.

## What landed (2026-11-19)

- `collection::media_type(kind)`, unconditional (no `client` needed), now the one reading inside the crate too: `role_of_kind`, `summary::derive`, the index extraction, the reader's summary join and the four kind tests of src/client/write.rs and src/client.rs call it. `kind` being a closed vocabulary (STORAGE §13), a stored kind never carries a parameter; the split stays for a kind a caller hands in.
- `PimdirReader::accounts_of(kind: Option<&str>)`, `None` for every kind.
- `PimdirQuery::scoped(kind: Option<PimdirQueryKind>, account: Option<&str>, collection: Option<&str>)`.
- `PimdirSearchCoverage::unread_counted() -> PimdirSearchUnread`, which `is_complete` now reads, so the completeness rule and a frontend's count are one function.
- `PimdirReader::overlay_quiet(collection) -> Result<bool, PimdirError>`, read from the overlay's own fold (`pending`), whether or not the reader overlays. It differs from himalaya's copy in two places, both the overlay's truth: a `copy` out of a collection leaves it quiet (the overlay changes nothing there), and a row whose payload does not decode is skipped as the overlay skips it, where himalaya answered "not quiet" for a queue it could not read; an unreadable queue is now an error.

## What each CLI deletes once a release carries them

Read on 2026-11-19 at himalaya aa8b71be, cardamum fe536ed and calendula 3da4010, working trees; nothing there was edited.

- **himalaya**: `is_mail` (src/pimdir/backend.rs:91, two callers: src/pimdir/client.rs:389 and src/pimdir/backend.rs:184) becomes `collection::media_type(kind) == MAIL_KIND`; `resolve_account` (src/pimdir/client.rs:367) keeps its messages and codes but reads `accounts_of(None)` for the configured check, accepting a configured account on an empty store (today `store-account-unknown`), and `accounts_of(Some(MAIL_KIND))` for the default; `PimdirClient::scope` (src/pimdir/index.rs:247) becomes `query.scoped(Some(PimdirQueryKind::Mail), account, None)`; `From<&PimdirSearchCoverage> for PimdirMailCoverage` (src/pimdir/index.rs:611) takes its four counts from `unread_counted()` and drops `reads`, `reads_mark` and `reads_content` with the conditions `summary` puts on them; `overlay_quiet` (src/pimdir/backend.rs:1455) goes, its caller (src/pimdir/backend.rs:475) asking the reader, its tests (src/pimdir/backend.rs:3835 to 3939) kept against the reader's answer.
- **cardamum**: `is_card_kind` (src/pimdir/backend.rs:951, callers src/pimdir/client.rs:136 and src/pimdir/backend.rs:679, its test at src/pimdir/backend.rs:1639) becomes `collection::media_type(kind) == CARD_KIND`, no longer counting an empty kind (the ruling); `resolve_account` (src/pimdir/client.rs:128) loses its `accounts_of` closure for `PimdirReader::accounts_of`; `scope` (src/pimdir/index.rs:103) becomes `scoped(Some(PimdirQueryKind::Contact), account, addressbook)` after its `known_collection` check; `From<&PimdirSearchCoverage> for PimdirCardCoverage` (src/pimdir/index.rs:267) reads `unread_counted()` in place of its `read` and `bodiless` filters.
- **calendula**: `is_calendar_kind` (src/pimdir/client.rs:169, caller src/pimdir/client.rs:121) goes, no longer counting an empty kind; `resolve_account` (src/pimdir/client.rs:113) as cardamum's; `scope` (src/pimdir/index.rs:155) becomes `scoped(Some(PimdirQueryKind::Event), account, calendar)`; `From<&PimdirSearchCoverage> for PimdirCalendarCoverage` (src/pimdir/index.rs:414) reads `unread_counted()` and drops `reads` and `counts_bodiless`.

Behaviour moves for users in two places only: himalaya accepts a configured account on an empty store, and cardamum and calendula stop picking an undeclared collection's account as their default.
