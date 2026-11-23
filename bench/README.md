# io-pimdir against notmuch

The standing bench of cairn/changes/archive/as-fast-as-notmuch: io-pimdir imports, indexes and queries the Enron corpus beside notmuch, on the same Maildirs, on one core. It started as pimdir's cairn/changes/compared-with-notmuch/bench, whose proposal.md (Part 2) holds the first numbers and the reasoning behind this protocol.

This directory is a crate of its own (an empty `[workspace]`, `publish = false`), built apart from io-pimdir and never packaged with it. The corpus is never vendored: scripts/fetch.sh downloads it.

## What is here

- src/main.rs: `pimbench`, a Maildir connector (`PimdirRemote`) driving io-pimdir's real sync engine, one round per folder in pages of 1,000, each member listed with its body; then `PimdirIndex::refresh`; then queries timed in process. A delta skips a folder whose `cur/` and `new/` modification times are those of its last listing, as `notmuch new` does, and hands back the checkpoint it was given when it finds nothing, as a server whose state did not move does (below).
- src/measure.rs: what each phase costs, read from the process itself.
- scripts/lib.sh: the paths every script shares, each overridable from the environment, and `measure`, which wraps a command with GNU time and the bytes it wrote.
- scripts/setup.sh: the tools rooted under `$B` and the two builds.
- scripts/fetch.sh: the Enron archive downloaded, checked and unpacked.
- scripts/corpus.sh: the Maildirs and the tiers.
- scripts/tier.sh: one import, notmuch then pimdir, optionally under eatmydata.
- scripts/queries.sh and scripts/queries.tsv: the query table through both CLIs.
- scripts/incr.sh: the incremental runs (nothing changed, 1,000 new, 1,000 flags, tagging).
- scripts/chain.sh: the whole protocol in order.
- scripts/summary.sh: runs turned into markdown tables.
- scripts/maildirify.py and scripts/nmconf.sh: the Enron tree as Maildir folders, the notmuch configuration.

## The directory `B`

Every script reads `B` (default /home/soywod/code/pimalaya/.bench-notmuch) and keeps everything under it: the tools, the builds, the archive, the corpus and the runs. `B` must be on ext4: /tmp is a tmpfs shared by every session, so a store or a build there measures memory, not a disk, and fills a space others need.

```text
$B/tools, tools-1, tools-2, tools-eat, tools-jq-bin   notmuch, GNU time, hyperfine, libeatmydata, jq (GC roots)
$B/target/release/pimbench              this crate
$B/target-cli/release/pimdir            the pimdir CLI (--features cli)
$B/versions                             what setup.sh built and with which tools
$B/dl/enron.tar.gz                      the archive
$B/enron-raw/maildir                    the archive unpacked
$B/corpus/c1, c2                        copy 1 and copy 2 as Maildirs
$B/corpus/x1, x2                        the tiers, trees of hardlinks
$B/runs/NAME                            one run: log, queries.txt, incr.log, nm/, pd/
```

Other paths are environment variables too: `NOTMUCH`, `GNUTIME`, `HYPERFINE`, `EATMYDATA`, `PIMBENCH`, `PIMDIR`, `JQ`, `CPU` (default 12), `DL`, `RAW`, `TARGET`, `TARGET_CLI`.

## Running it

```sh
cd bench/scripts
./setup.sh tree        # or: ./setup.sh snapshot, a git archive of HEAD
./fetch.sh
./corpus.sh x1 x2
./chain.sh 12          # the whole protocol on core 12, hours
```

Or step by step:

```sh
export B=/home/soywod/code/pimalaya/.bench-notmuch
EAT=1 TOOLS=pimdir ./tier.sh x1-eat "$B/corpus/x1" 12
./tier.sh x1 "$B/corpus/x1" 12
./queries.sh x1 12
./incr.sh x1 "$B/corpus/x1" 12
./corpus.sh x1         # incr.sh changed x1 in place
./tier.sh x2 "$B/corpus/x2" 12
RUNS=5 ./queries.sh x2 12
./summary.sh x1-eat x1 x2 > "$B/runs/summary.md"
```

`setup.sh tree` builds the working tree as it stands and records its commit with `-dirty` when it has changes; `setup.sh snapshot` builds `git archive HEAD` unpacked under `$B/snapshot/io-pimdir`, so a run is not moved by edits made while it lasts. Both use io-pimdir's own flake (`nix develop`).

`pimbench` alone:

```sh
pimbench sync  STORE MAILDIR_ROOT STATE_DIR [PAGE]
pimbench index STORE
pimbench query STORE REPEATS QUERY...
```

`STATE_DIR` is the connector's own state, what a server would remember for a client: a file per folder naming the members it last listed (the folder's path with `/` as `%`), and `%mtimes`, a line per folder with the modification times, in nanoseconds, of its `cur/` and `new/` when last listed. A delta whose folder still shows those times answers nothing without reading the directory: a delivery, a deletion and a flag change each add, remove or rename a file, which moves its directory's time. The times are taken before the listing and kept only once the folder's sync landed, and only when at least 2 s old, so a change made in the same clock tick as a listing is caught next time. Like notmuch, it misses an edit of a message file in place (Maildir never does one) and a tree restored with its old times over other content; delete `%mtimes` to list every folder again.

A delta finding nothing hands back the checkpoint it was given, so the engine writes nothing for that folder. On the 517k tier's 3,311 folders a no-op sync went from 5.0 s wall, 2.6 s of CPU and 3,325 `fsync`s (io-pimdir 78483f0) to 0.49 s, 0.48 s and none (notmuch's no-op `new`: 0.07 s), of which the engine's delta loads took 0.43 s, almost all `list_owing_bindings` and `list_lacking_items` walking every binding and item of each collection. Since pimdir's the-owing-rows-are-marked both seek a mark the schema's triggers keep, and the same no-op takes 0.12 s wall and 0.12 s of CPU (0.39 s and 0.39 s before it, on the same day).

`pimbench sync` reads `PIMBENCH_FLUSH`, how a page's bodies reach the disk before the transaction naming them commits (`PimdirSourceStore::body_flush`): `each` (the default, the library's) flushes each body's temporary as it is written, then moves it onto its name; `deferred` writes every temporary of the page first, 256 at most open at once, then flushes each, then moves each; `writeback` is `deferred` with each temporary's writeback started as it is written (`sync_file_range` with `SYNC_FILE_RANGE_WRITE`, Linux), so a flush finds it mostly done. Every mode flushes a body before its move and each shard directory once a page, then commits, and the `phase=sync` line ends with `flush=` naming the mode:

```sh
PIMBENCH_FLUSH=each      taskset -c 12 pimbench sync STORE MAILDIR_ROOT STATE_DIR
PIMBENCH_FLUSH=deferred  taskset -c 12 pimbench sync STORE MAILDIR_ROOT STATE_DIR
PIMBENCH_FLUSH=writeback taskset -c 12 pimbench sync STORE MAILDIR_ROOT STATE_DIR
```

`pimbench index` reads three knobs from the environment, so a refresh setting is measured before it is changed in the crate: `PIMBENCH_BATCH` the rows per transaction (`PimdirIndexRefreshOptions::batch`), `PIMBENCH_FTS` FTS5 settings written into both full-text tables once index.db is built (`automerge=8,crisismerge=64`, `pgsz=8000`), and `PIMBENCH_END` what follows the refresh inside the phase: `optimize`, a `merge` of that many pages repeated until done, or `checkpoint`, each ending with a `TRUNCATE` checkpoint.

## What a phase prints

Each phase ends with one line of `key=value` pairs, here from a 14-message smoke Maildir:

```text
phase=sync wall_s=0.014 user_s=0.003 sys_s=0.003 write_bytes=339968 cancelled_write_bytes=0 wchar=259484 read_bytes=0 rchar=144079 items=14 peak_rss_kib=8864 folders=2 unreadable=0 mail_bytes=3771
```

- `wall_s`: the phase's wall time, the store already open.
- `user_s`, `sys_s`: CPU time of the process during the phase (`getrusage`, `RUSAGE_SELF`).
- `write_bytes`, `cancelled_write_bytes`, `read_bytes`, `wchar`, `rchar`: the phase's share of /proc/self/io. `write_bytes` is what the process sent toward the disk (counted as page cache pages are dirtied, `cancelled_write_bytes` the dirty pages it truncated before writeback), `wchar` what went through `write(2)` and its kin, so `wchar` over the mail's size is the write amplification SQLite causes and `write_bytes` the part the disk sees.
- `items`: the phase's own unit, the messages pulled for `sync`, indexed for `index`, the hits counted for a `query`.

`pimbench query` prints a line per query (`phase=query`, its count, page and threaded page medians in ms, the query last on the line), then `phase=queries` for the whole run.

The scripts add, for every command, the same counters taken from outside (`io ...`, the counters of a shell that reaped the command, so notmuch gets them too) and GNU time's `wall=..,user=..,sys=..,rss_kib=..`, both in the run's log.

## The protocol

- **One idle core.** Pick a core nothing runs on and its SMT sibling idle too: `ps -eo pid,pcpu,psr,args --sort=-pcpu | head`, and two reads of /proc/stat a few seconds apart for the per-core counters. On 2026-11-21 core 12 and its sibling 28 were free. Every command runs under `taskset -c CPU`.
- **One tool at a time**, never notmuch and pimdir together, never two runs together, and `sync` before each import step so one step's writeback is not charged to the next.
- **Warm cache** for queries: hyperfine runs 2 warm-ups, then 10 runs (5 at 1M), and takes the median.
- **CPU time and bytes written beside wall time.** A durable import waits on fsync, and on this machine a shared disk moved such a wall time by a factor of 2 with other sessions' builds. CPU time and bytes written are the numbers that compare across runs; wall time is read with them, never alone.
- **Import gates on a quiet machine only.** Check the I/O pressure (/proc/pressure/io) and the other sessions before and after an import; a gate is judged on a run where neither moved.
- **The fsync split.** `EAT=1 ./tier.sh` runs the import under eatmydata, which turns fsync and its kin into no-ops: its wall time is the work, the durable run's surplus the waits.
- **No perf.** `perf_event_paranoid` is 2 on this machine, so no hardware counters; CPU time comes from `getrusage` and GNU time, waits from /proc (`wchan`, /proc/pressure/io) read while a run lasts.
- **The notmuch line is rerun with each change.** Every run measures notmuch again beside pimdir on the same machine state, and `$B/versions` records its version; pimdir numbers are never compared with notmuch numbers from another day, nor with a notmuch release other than the one recorded.

## The tiers

| tier | content | messages |
| --- | --- | --- |
| x1 (517k) | copy 1 of the Enron corpus | 517,401 |
| x2 (1M) | copies 1 and 2, hardlinked into one tree | 1,034,802 |

Copy 1 hardlinks the original files. Copy 2 rewrites every Message-ID domain (`@thyme` to `@thyme2`, In-Reply-To and References with it) and suffixes each user directory with `~2`, so it is another set of messages. Flags come from a hash of each path, the same for both tools and both copies: 5% in new/ (unread), else cur/ with `S` 75%, `R` 8%, `F` 4%. Enron carries almost no In-Reply-To and no attachments, so threads are mostly single messages.

The archive is the CMU release of 2015-05-07, https://www.cs.cmu.edu/~enron/enron_mail_20150507.tar.gz. fetch.sh checks its sha256, b3da1b3fe0369ec3140bb4fbce94702c33b7da810ec15d718b3fadf5cd748ca7.

## The gates

From cairn/changes/archive/as-fast-as-notmuch/proposal.md, on the 517k and 1M tiers, warm cache, one core:

- every query of the table (count, page of 50, threaded page of 50) no slower than notmuch's: the `worst pd / nm` column of summary.sh at or below 1.0;
- no query whose cost grows with the store while its answer does not (a rare word, a one-collection scope, an address): compare `kiwi`, `folder:dasovich-j/inbox` and the address between x1 and x2;
- a durable import no slower than `notmuch new` on a quiet machine, CPU and bytes written per message reported;
- a no-op sync within an order of notmuch's no-op `new` for the same folders (incr.sh's `noop` steps).
