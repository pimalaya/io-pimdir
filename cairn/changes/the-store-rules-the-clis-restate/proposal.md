---
cairn: change
id: the-store-rules-the-clis-restate
status: draft
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
