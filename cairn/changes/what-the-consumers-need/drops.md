# Drops

Each drop: the io-pimdir commit, what consumers feel, the archive under ~/code/pimalaya/.lanes/io-pimdir/drops/<n>, and the pushed (re-signed) hash once the owner has pushed it. App lanes pin pushed hashes only.

## Drop 1, CI green and Android's build

- Consumers feel: `sql::search` (statements, `MIGRATIONS`, `FORMAT_VERSION`, `all()`) builds under `indexer` without `client` (AN1); the Windows build and the per-feature builds without `client` compile again. No API or behaviour change otherwise.
- Commit and pushed hash (signed): cc3021bbd0d08506f1ea55b24603dd92f9496b92. Archive: $L/drops/1.

## Drop 2, the engine follows SYNC, and the accepted requests

- Consumers feel: the rekey lands creates, keeps conflicts and owed flags, stores listed bodies and reads a repeated handle once, as a page does (pimdir vectors 201 to 213, sync-draft-15 edited in place at pimdir 0401aab); a rekey reports `Created`, `Disagreed` and `Overruled` besides `Added`. The drain of a `copy` or `move` joins another source's item instead of minting a `dup:` one. `pending_actions` lists an undecodable row as `PimdirAction::Unknown` instead of failing (no signature change); `pimdir queue list` prints its error. `summary::DERIVATION` is public. Rekey outcomes moved: BREAKING lines in CHANGELOG.
- Commit and pushed hash (signed): 2314084b5d8e6b4b9044c9f0351352068d4080db, with pimdir 0401aab (vectors) and 2f51d96 (checks/io-pimdir.rev), all pushed. Archive: $L/drops/2.

## Drop 3, the release candidate

- Consumers feel: version 0.20.0 (untagged, unpublished: the owner's tag publishes it), so the patch requirement can read `version = "0.20.0"` once released; rust-version 1.89 builds again (drop 2 needed a later compiler); a `search` build without `indexer` reads the hit columns (himalaya N6) and its `PimdirIndex` is no longer `Send`, as with `indexer`; `accounts_of` skips the coverage join (himalaya N7). No store, index or sync rule moves.
- Commit and pushed hash (signed): 17d4ea8008996bc94d014c1ff925a5e609b89722, version 0.20.0 untagged, with pimdir 087410f pinning it. Archive: $L/drops/3.
