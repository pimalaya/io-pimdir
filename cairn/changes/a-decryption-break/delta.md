---
cairn: delta
change: a-decryption-break
---

## ADDED Requirements

### Requirement: Decryption is a break of the refresh
When decryption lands, the refresh SHALL yield `WantsDecryption` for each encrypted part it meets, a runner answering with the plaintext or declining, a declined body recorded `encrypted` with its headers indexed as today, so the crate holds no key and does no cryptography.

#### Scenario: An encrypted message, declined
- GIVEN a store holding a `multipart/encrypted` message
- WHEN the index is refreshed by a runner that declines
- THEN the body is recorded `encrypted`, and `from:` answers it complete while free text counts it under `encrypted`

## MODIFIED Requirements

## REMOVED Requirements
