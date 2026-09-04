---
cairn: spec
capability: import
status: planned
---

# Legacy import (Maildir, m2dir, vdir → pimdir)

Not yet implemented. This is the reference mapping for the import scripts that
turn an existing Maildir, m2dir or vdir tree into a valid pimdir store. It has no
counterpart on-disk format: there is one store format (the pimdir SQLite store),
and importing is a one-way, script-driven conversion *into* it. Kept here so the
translation decisions, which are lossy and opinionated, are written down before
the code exists.

## Mapping

Maildir, m2dir and vdir are recovered as *sources* of a pimdir collection:

- A **Maildir** or **m2dir** folder maps to a `collections` row of
  `kind = message/rfc822`. Each message file becomes an object (its bytes,
  content-addressed under `store_meta.hash_algo`, in the blob directory) plus an
  `items` row keyed by `link_id` (the `Message-ID` read from the RFC 822 bytes).
- A **vdir** calendar or address book maps to a `collections` row of
  `kind = text/calendar` or `text/vcard`. Each `.ics` / `.vcf` file becomes an
  object plus an `items` row keyed by the iCalendar / vCard `UID`.

### Mutable state (the only hard part)

Message bytes, `UID`s and `Message-ID`s carry over verbatim; the judgement calls
are all about the state each legacy format keeps *outside* the content, which has
to land in the store's `items.flags` (a JSON array of raw flag strings):

- **Maildir** encodes flags as a filename-suffix letter set after `:2,`. Map the
  standard letters to their IANA "IMAP and JMAP Keywords" equivalents:
  `S`→`$seen`, `R`→`$answered`, `F`→`$flagged`, `T`→`$deleted` (i.e. `\Deleted`
  semantics), `D`→`$draft`, `P`→`$forwarded`. Unknown letters are preserved as
  raw custom keywords rather than dropped. The rename-on-flag-change cost and the
  `:` that makes Maildir unusable on Windows both vanish, because flags are now a
  column, not part of the name.
- **m2dir** keeps flags in a sidecar `.meta/<id>.flags` file instead of the name;
  read it and map the same way. m2dir's merged id+hash token splits into the
  store's separate `link_id` (identity) and object `hash` (content state).
- **vdir** keeps presentation in per-collection sidecar files: `color` →
  `collections.color`, `displayname` → `collections.name`. Per-item state reduces
  to the object hash, because everything else already lives inside the iCalendar
  or vCard item.

### Notes for the implementer

- Import is **lossy-upward**: the legacy formats define fewer flags than pimdir
  knows, so unmapped/custom markers should be carried through as raw keywords,
  never silently discarded.
- Intrinsic content ids (`Message-ID`, `UID`) are the `link_id`; they are read
  from the bytes, not invented, so re-importing the same tree is idempotent on
  identity.
- Deduplication falls out for free: two identical bodies across folders share one
  content-addressed object.
