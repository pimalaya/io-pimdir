# Drops

Each drop: the io-pimdir commit, what consumers feel, the archive under ~/code/pimalaya/.lanes/io-pimdir/drops/<n>, and the pushed (re-signed) hash once the owner has pushed it. App lanes pin pushed hashes only.

## Drop 1, CI green and Android's build

- Consumers feel: `sql::search` (statements, `MIGRATIONS`, `FORMAT_VERSION`, `all()`) builds under `indexer` without `client` (AN1); the Windows build and the per-feature builds without `client` compile again. No API or behaviour change otherwise.
- Commit: pending the lead. Archive: $L/drops/1 once committed. Pushed hash: pending.
