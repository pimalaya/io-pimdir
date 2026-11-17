---
cairn: delta
change: a-comment-closes-on-any-run-of-dashes
---

## ADDED Requirements

### Requirement: An HTML comment closes on any run of dashes
The `text/html` reduction (`search::html::to_text`) SHALL close a comment at the first `>` after two dashes or more, so a decorated separator (`<!----------- SECTION ----------->`) hides itself and not the rest of the body. The change bumps `EXTRACTOR` to `io-pimdir 2`, so every index another revision built is extracted again once.

#### Scenario: A separator comment
- GIVEN `<p>before</p><!----------- SECTION -----------><p>after</p>`
- WHEN it is reduced
- THEN it reads `before` then `after` (src/search/html.rs `a_comment_closes_on_any_run_of_dashes`)

## MODIFIED Requirements

### Requirement: An index records the revisions of its extraction
The crate SHALL state the revision of SEARCH §6 it extracts and expands under, `search::extract::EXTRACTION` (1), and its own revision of what §6 leaves to an indexer, `search::extract::EXTRACTOR` (`io-pimdir 2` since an HTML comment closes on any run of dashes and an expansion stepping past year 9999 is not cut; `io-pimdir 1` before), an opaque text bumped in the change that alters what the extraction of any body yields: the reduction of `text/html`, the charsets decoded, what a field holds, or a bump of mime-rs, vcard-rs, ical-rs or encoding_rs reading a body otherwise. `init_index_meta` SHALL record both (SEARCH §2). `PimdirIndex::open`, and `refresh` under index.lock, SHALL rebuild an index recording another of either, the texts compared byte for byte (`PimdirIndexHeld::Extracted`): the rebuild, which SEARCH §2 allows in place of extracting every held body again, leaves what a rebuild writes by construction, and its `init_index_meta` records the crate's own, so the crate runs no `set_extraction`, whose constant (`sql::search::SET_EXTRACTION`) stands for a consumer extracting again in place. `PimdirIndex::open_query` SHALL answer from such an index and never refuse it: the hits are the index's, and a snippet is cut from the chunks of this build's own extraction (`PimdirExtract::chunks`) rather than the stored offsets, which index another extraction's text (SEARCH §6). The refresh coroutine reads neither revision: a consumer of another shape compares the two constants with what its index records before it runs one, and rebuilds or extracts again itself, as it judges the version and the tokenizer.

#### Scenario: An index another extractor wrote
- GIVEN an index of three bodies and a calendar item, its `index_meta.extractor` then restated `another 7` (`set_extraction`), or its `extraction` 2
- WHEN `open_query` opens it, THEN it answers its hits and a snippet; WHEN `open` opens it or `refresh` folds into it, THEN the index is rebuilt, every body indexed and the calendar item expanded again, and it records `io-pimdir 2` under revision 1 (tests/search.rs `an_index_under_another_extraction_is_rebuilt_and_every_body_indexed_again`, which rebuilds nothing without the comparison)


## REMOVED Requirements
