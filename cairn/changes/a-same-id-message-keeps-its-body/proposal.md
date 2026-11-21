---
cairn: change
id: a-same-id-message-keeps-its-body
status: active
created: 2026-11-21
---

# A same-ID message keeps its body

## Why

neverest cairn/changes/a-move-into-a-same-id-message-keeps-both (d23c699, Stalwart v0.16, io-pimdir 0.19.1): two different messages stating one `Message-ID`, A in Left and B in Right, A moved to Right. The server keeps both bodies; the store does not.

- **B listed before the move**: the upgrade links B's body from A's row by link id (src/upgrade.rs, `State::LookingUp`), so B's item holds A's body and B's is never fetched.
- **B unlisted when the move is pushed**: the relocation delivers A's pending create, and the next listing lands it on UID 1, B, the first member in handle order stating the link id (src/sync.rs `land`), then pulls UID 2, A, as `dup:…#2`. The item holding A's body is bound to B, so a later delete or flag of it reaches B.

## What the spec says today

The first case is SYNC §6 ("Linking instead of fetching") working as written: a body is linked from a row agreeing on `subject`, `date` and `sender` byte for byte, and on `size` where the listing states one. The repro's two messages share all four (the markers `MOVEDMARKER` and `OTHERMARKER` have one length), and nothing an IMAP listing states tells them apart without fetching. "Link only on proven equal bytes" cannot be met from what IMAP lists: it means fetching every body a link would have saved, every copy of a message filed under several Gmail labels among them.

The second case is a gap: no rule says which member a delivered create lands on when two members state its link id, and the relocation knows the answer (`COPYUID`, RFC 4315, which `UID MOVE` and `UID COPY` answer and neverest's `move_messages` drops).

## What

1. **A relocation names the member it made.** An accepted `Remove` carrying `to` that relocated MAY state the target handle; the delivered create then lands on that member alone, never on another stating its link id (SYNC §4, §6). Without one, today's rule stands. neverest passes the `COPYUID` it already receives. This fixes the second case outright.
2. **Linking: a decision for the owner**, three options:
   - (a) Keep SYNC §6 as it stands, and state the accepted risk in its text: two messages agreeing on `Message-ID`, `subject`, `date`, `sender` and size are one message to the store. The repro's first variant then asserts what the spec rejects, and is restated with a size, subject or date that differs.
   - (b) Link only where the source states a server object id equal on both rows (RFC 8474 `EMAILID`, Gmail's `X-GM-MSGID`, JMAP's `blobId`), else fetch. Exact on servers serving one (Stalwart, Gmail, Fastmail), and costs a fetch per duplicate on others (Dovecot without `OBJECTID`). Needs a listing field through io-imap, neverest and the meta io-pimdir reads.
   - (c) Never link over a source with no content hash: always exact, always the fetch.
   Recommended: (b), falling back to (a)'s rule where the server states no object id, so Gmail and Stalwart are exact and no server pays more fetches than today.

## Not in scope

`targets_holding` (src/sync/remote.rs) took no part in either run.
