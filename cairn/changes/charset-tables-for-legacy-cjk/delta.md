---
cairn: delta
change: charset-tables-for-legacy-cjk
---

## ADDED Requirements

## MODIFIED Requirements

### Requirement: Decoding follows Annex A.0
A mail header SHALL be decoded (RFC 2047 encoded words, adjacent ones joined) and every address made canonical (the addr-spec alone, lowercased whole, `mailto:` stripped); a vCard or iCalendar text value SHALL be unescaped, a content line split on the first colon outside a quoted parameter, a structured value on the semicolons no backslash escapes. Invalid UTF-8 is replaced, never refused, once the octets are decoded: a transfer encoding is undone on the octets, never on text read from them. Unfolding a header removes the CRLF alone and keeps the whitespace (RFC 5322 §2.2.3); unfolding a vCard or iCalendar line takes the one leading character with it.

An encoded word's charset is decoded by the byte for `iso-8859-1` and `us-ascii`, by its table for `windows-1252` (the 0x80 to 0x9F row is not latin1), and as UTF-8 otherwise. Without the `charset-tables` feature (the default) the crate depends on no charset library, so any other 8-bit charset reads its bytes as UTF-8, lossily: a known deviation from Annex A.0, bounded to the headers of messages in charsets the format's vectors do not carry. With the feature, a charset a WHATWG label names (gbk, big5, euc-jp, iso-2022-jp, shift_jis, euc-kr, the windows-125x and iso-8859-x rows, utf-16 among them) is decoded by that encoding before the fallback: the label holds, a byte-order mark among the octets decoding as the character it is rather than overriding the label, and a label the table names with no encoding behind it (iso-2022-kr among them) or does not name at all still reads as UTF-8, lossily. The derivation is otherwise unchanged: the same key, row shape and sort key over the decoded text.

#### Scenario: A GBK subject where the tables are compiled
- GIVEN a message whose header reads `Subject: =?gbk?B?...?=` and the `charset-tables` feature is compiled in
- WHEN its summary is derived
- THEN the subject row holds the decoded text and no replacement character

#### Scenario: The same message without the feature
- GIVEN the same header and a default build
- WHEN its summary is derived
- THEN the octets read as UTF-8, lossily, exactly as before the feature existed

## REMOVED Requirements
