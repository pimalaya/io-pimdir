# Drops

Each drop: the io-pimdir commit, what consumers feel, the archive under ~/code/pimalaya/.lanes/io-pimdir/drops/<n>, and the pushed (re-signed) hash once the owner has pushed it. App lanes pin pushed hashes only.

## Drop 1, CI green and Android's build

- Consumers feel: `sql::search` (statements, `MIGRATIONS`, `FORMAT_VERSION`, `all()`) builds under `indexer` without `client` (AN1); the Windows build and the per-feature builds without `client` compile again. No API or behaviour change otherwise.
- Commit and pushed hash (signed): cc3021bbd0d08506f1ea55b24603dd92f9496b92. Archive: $L/drops/1.

## Drop 2, the engine follows SYNC, and the accepted requests

- Consumers feel: the rekey lands creates, keeps conflicts and owed flags, stores listed bodies and reads a repeated handle once, as a page does (pimdir vectors 201 to 213, sync-draft-15 edited in place at pimdir 0401aab); a rekey reports `Created`, `Disagreed` and `Overruled` besides `Added`. The drain of a `copy` or `move` joins another source's item instead of minting a `dup:` one. `pending_actions` lists an undecodable row as `PimdirAction::Unknown` instead of failing (no signature change); `pimdir queue list` prints its error. `summary::DERIVATION` is public. Rekey outcomes moved: BREAKING lines in CHANGELOG.
- Commit: pending the lead. Archive: $L/drops/2 once committed. Pushed hash: pending.
