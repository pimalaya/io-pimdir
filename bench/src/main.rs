//! pimdir side of the notmuch comparison: a Maildir connector driving
//! io-pimdir's sync engine, the index refresh, and in-process query timing.
//!
//! ```sh
//! pimbench sync  STORE MAILDIR_ROOT STATE_DIR [PAGE]
//! pimbench index STORE
//! pimbench query STORE REPEATS QUERY...
//! ```
//!
//! Each phase ends with one `phase=NAME key=value ...` line: wall, user and
//! system seconds, the bytes of `/proc/self/io` and the items it handled.

mod measure;

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs, io,
    path::{Path, PathBuf},
    process::ExitCode,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use io_pimdir::{
    change::PimdirChange,
    client::{
        PimdirSourceStore, PimdirStore,
        blobs::PimdirBodyFlush,
        search::{PimdirIndex, PimdirIndexRefreshOptions, PimdirSearchOptions},
    },
    collection::{PimdirCheckpoint, PimdirCollectionId, PimdirCursor, PimdirScope},
    hash,
    placement::{PimdirFlags, PimdirHandle},
    remote::{
        PimdirEnumerate, PimdirEnumerated, PimdirFetchedBody, PimdirFetchedItem, PimdirListing,
        PimdirPushOutcome, PimdirPushResult, PimdirRemote, PimdirRemoteItem, PimdirRemoteMeta,
        PimdirRemoteSnapshot, PimdirTier,
    },
    search::query::PimdirQuery,
    summary,
    sync::PimdirSyncOptions,
};

use crate::measure::Phase;

/// One message file of a Maildir folder.
#[derive(Clone)]
struct Entry {
    handle: String,
    name: String,
    path: PathBuf,
}

/// A Maildir as a pimdir source: handles are the unique part of the file
/// name, flags its info suffix, every listed member carries its body.
///
/// A delta skips a folder whose `cur/` and `new/` kept the modification
/// times they had when it was last listed, as `notmuch new` does: a
/// delivery, a deletion and a flag change each add, remove or rename a
/// file, which moves its directory's time. Those times live in the state
/// directory's `%mtimes`, taken before a listing and kept only for a
/// folder whose sync landed, and only when older than [`SETTLED`]: a
/// change in the same clock tick as a listing would leave the time
/// unmoved, so a folder changed that recently is listed again next time.
/// A delta finding nothing hands back the checkpoint it was given, as a
/// server whose state did not move does.
struct Maildir {
    root: PathBuf,
    state: PathBuf,
    page: usize,
    listings: BTreeMap<String, Vec<Entry>>,
    read_bytes: u64,
    mtimes: BTreeMap<String, Mtimes>,
    seen: BTreeMap<String, Option<Mtimes>>,
}

/// The modification times of a folder's `cur/` and `new/`, in nanoseconds
/// since the epoch, `None` for a directory missing.
type Mtimes = (Option<i128>, Option<i128>);

/// How old a directory's time must be to be trusted as unmoved later.
const SETTLED: Duration = Duration::from_secs(2);

fn handle_of(name: &str) -> &str {
    name.split(':').next().unwrap_or(name)
}

fn flags_of(name: &str) -> PimdirFlags {
    let mut flags = BTreeSet::new();
    if let Some((_, info)) = name.split_once(":2,") {
        for c in info.chars() {
            let flag = match c {
                'S' => "\\Seen",
                'F' => "\\Flagged",
                'R' => "\\Answered",
                'D' => "\\Draft",
                _ => continue,
            };
            flags.insert(flag.to_string());
        }
    }
    PimdirFlags::Known(flags)
}

impl Maildir {
    fn list(&self, collection: &str) -> io::Result<Vec<Entry>> {
        let mut entries = Vec::new();
        for sub in ["cur", "new"] {
            let dir = self.root.join(collection).join(sub);
            let Ok(read) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in read {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                entries.push(Entry {
                    handle: handle_of(&name).to_string(),
                    path: entry.path(),
                    name,
                });
            }
        }
        entries.sort_by(|a, b| a.handle.cmp(&b.handle));
        entries.dedup_by(|a, b| a.handle == b.handle);
        Ok(entries)
    }

    /// The times of `collection`'s `cur/` and `new/`, and whether both are
    /// old enough to be trusted unmoved next time ([`SETTLED`]).
    fn stat(&self, collection: &str) -> (Mtimes, bool) {
        let now = SystemTime::now();
        let mut settled = true;
        let mut time = |sub: &str| {
            let modified = fs::metadata(self.root.join(collection).join(sub))
                .and_then(|meta| meta.modified())
                .ok()?;
            let age = now.duration_since(modified).unwrap_or_default();
            settled &= age >= SETTLED;
            let since = modified.duration_since(UNIX_EPOCH).ok()?;
            Some(since.as_nanos() as i128)
        };
        let mtimes = (time("cur"), time("new"));
        (mtimes, settled)
    }

    /// Keeps the times `collection` was listed at, once its sync landed;
    /// forgets them when they were too recent to trust, or not taken.
    fn settle(&mut self, collection: &str) {
        match self.seen.remove(collection).flatten() {
            Some(mtimes) => self.mtimes.insert(collection.to_string(), mtimes),
            None => self.mtimes.remove(collection),
        };
    }

    fn mtimes_path(state: &Path) -> PathBuf {
        state.join("%mtimes")
    }

    /// The `%mtimes` file, named apart from every sidecar (a folder's path
    /// never starts with `/`): a line per folder, its name then the two
    /// times, tab-separated, `-` for a directory missing.
    fn load_mtimes(state: &Path) -> BTreeMap<String, Mtimes> {
        let text = fs::read_to_string(Self::mtimes_path(state)).unwrap_or_default();
        let time = |field: &str| field.parse().ok();
        text.lines()
            .filter_map(|line| {
                let mut fields = line.split('\t');
                let (folder, cur, new) = (fields.next()?, fields.next()?, fields.next()?);
                Some((folder.to_string(), (time(cur), time(new))))
            })
            .collect()
    }

    /// Writes `%mtimes` whole, through a rename, so a run cut short leaves
    /// the last one.
    fn save_mtimes(&self) -> io::Result<()> {
        let time = |t: Option<i128>| t.map_or_else(|| "-".to_string(), |t| t.to_string());
        let mut text = String::new();
        for (folder, (cur, new)) in &self.mtimes {
            text.push_str(&format!("{folder}\t{}\t{}\n", time(*cur), time(*new)));
        }
        let path = Self::mtimes_path(&self.state);
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, text)?;
        fs::rename(tmp, path)
    }

    fn sidecar(&self, collection: &str) -> PathBuf {
        self.state.join(collection.replace('/', "%"))
    }

    fn save_names(&self, collection: &str, entries: &[Entry]) -> io::Result<()> {
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        fs::write(self.sidecar(collection), names.join("\n"))
    }

    fn load_names(&self, collection: &str) -> io::Result<BTreeMap<String, String>> {
        let text = fs::read_to_string(self.sidecar(collection)).unwrap_or_default();
        Ok(text
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| (handle_of(l).to_string(), l.to_string()))
            .collect())
    }

    fn item(&mut self, entry: &Entry) -> io::Result<PimdirRemoteItem> {
        let bytes = fs::read(&entry.path)?;
        self.read_bytes += bytes.len() as u64;
        let derivation = summary::mail::derive(&bytes);
        let body = PimdirFetchedBody::Inline {
            hash: hash::hash(&bytes),
            bytes,
        };
        Ok(PimdirRemoteItem {
            handle: PimdirHandle(entry.handle.clone()),
            flags: flags_of(&entry.name),
            revision: None,
            meta: PimdirRemoteMeta::new(derivation).with_body(body),
        })
    }
}

impl PimdirRemote for Maildir {
    type Error = io::Error;

    fn enumerate(
        &mut self,
        collection: &PimdirCollectionId,
        request: PimdirEnumerate,
    ) -> Result<PimdirEnumerated, io::Error> {
        let name = collection.0.clone();
        match request.listing {
            PimdirListing::Round { cursor, .. } => {
                let start = match &cursor {
                    None => {
                        let (mtimes, settled) = self.stat(&name);
                        self.seen.insert(name.clone(), settled.then_some(mtimes));
                        let listing = self.list(&name)?;
                        self.listings.insert(name.clone(), listing);
                        0
                    }
                    Some(PimdirCursor(bytes)) => {
                        if !self.listings.contains_key(&name) {
                            let listing = self.list(&name)?;
                            self.listings.insert(name.clone(), listing);
                        }
                        String::from_utf8_lossy(bytes).parse().unwrap_or(0)
                    }
                };
                let listing = self.listings[&name].clone();
                let end = (start + self.page).min(listing.len());
                let mut items = Vec::with_capacity(end - start);
                for entry in &listing[start..end] {
                    items.push(self.item(entry)?);
                }
                let (cursor, checkpoint) = match end < listing.len() {
                    true => (Some(PimdirCursor(end.to_string().into_bytes())), None),
                    false => {
                        self.save_names(&name, &listing)?;
                        (None, Some(PimdirCheckpoint(b"1".to_vec())))
                    }
                };
                Ok(PimdirRemoteSnapshot::page(items, cursor, checkpoint).into())
            }
            PimdirListing::Delta(PimdirCheckpoint(old)) => {
                let (mtimes, settled) = self.stat(&name);
                if self.mtimes.get(&name) == Some(&mtimes) {
                    self.seen.insert(name, Some(mtimes));
                    let snapshot =
                        PimdirRemoteSnapshot::delta(vec![], vec![], PimdirCheckpoint(old));
                    return Ok(snapshot.into());
                }
                self.seen.insert(name.clone(), settled.then_some(mtimes));
                let known = self.load_names(&name)?;
                let listing = self.list(&name)?;
                let mut items = Vec::new();
                let mut present = BTreeSet::new();
                for entry in &listing {
                    present.insert(entry.handle.clone());
                    if known.get(&entry.handle) != Some(&entry.name) {
                        items.push(self.item(entry)?);
                    }
                }
                let vanished: Vec<PimdirHandle> = known
                    .keys()
                    .filter(|h| !present.contains(*h))
                    .map(|h| PimdirHandle(h.clone()))
                    .collect();
                let checkpoint = match items.is_empty() && vanished.is_empty() {
                    true => old,
                    false => {
                        self.save_names(&name, &listing)?;
                        let next = String::from_utf8_lossy(&old).parse().unwrap_or(0u64) + 1;
                        next.to_string().into_bytes()
                    }
                };
                self.listings.insert(name, listing);
                let snapshot =
                    PimdirRemoteSnapshot::delta(items, vanished, PimdirCheckpoint(checkpoint));
                Ok(snapshot.into())
            }
            PimdirListing::Head(_) => Ok(PimdirEnumerated::NoHead),
        }
    }

    fn fetch(
        &mut self,
        collection: &PimdirCollectionId,
        handles: Vec<PimdirHandle>,
        tier: PimdirTier,
    ) -> Result<Vec<PimdirFetchedItem>, io::Error> {
        let name = collection.0.clone();
        if !self.listings.contains_key(&name) {
            let listing = self.list(&name)?;
            self.listings.insert(name.clone(), listing);
        }
        let by_handle: BTreeMap<String, Entry> = self.listings[&name]
            .iter()
            .map(|e| (e.handle.clone(), e.clone()))
            .collect();
        let mut out = Vec::new();
        for handle in handles {
            let Some(entry) = by_handle.get(&handle.0) else {
                continue;
            };
            let bytes = fs::read(&entry.path)?;
            self.read_bytes += bytes.len() as u64;
            let d = summary::mail::derive(&bytes);
            let body = match tier {
                PimdirTier::Full => Some(PimdirFetchedBody::Inline {
                    hash: hash::hash(&bytes),
                    bytes,
                }),
                PimdirTier::Meta => None,
            };
            out.push(PimdirFetchedItem {
                handle,
                link_id: d.link_id,
                summary: d.summary,
                sort_key: d.sort_key,
                body,
                revision: None,
            });
        }
        Ok(out)
    }

    fn push(
        &mut self,
        _collection: &PimdirCollectionId,
        changes: Vec<PimdirChange>,
    ) -> Result<Vec<PimdirPushResult>, io::Error> {
        Ok(changes
            .iter()
            .map(|change| PimdirPushResult {
                handle: change.handle().clone(),
                outcome: PimdirPushOutcome::Accepted,
                assigned: None,
                revision: None,
                relocated: false,
            })
            .collect())
    }
}

/// Every Maildir folder under `root`, as paths relative to it.
fn folders(root: &Path) -> io::Result<Vec<String>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if dir.join("cur").is_dir() {
            let rel = dir
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            out.push(rel);
        }
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name();
            if matches!(name.to_str(), Some("cur" | "new" | "tmp")) {
                continue;
            }
            if entry.file_type()?.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The body flush `PIMBENCH_FLUSH` names: `each` (the default), a flush
/// per body as it is written; `deferred`, a page's bodies written, then
/// each flushed, then each moved; `writeback`, `deferred` with each
/// body's writeback started as it is written.
fn body_flush() -> PimdirBodyFlush {
    match env::var("PIMBENCH_FLUSH").as_deref() {
        Err(_) => PimdirBodyFlush::default(),
        Ok("each") => PimdirBodyFlush::Each,
        Ok("deferred") => PimdirBodyFlush::Deferred { writeback: None },
        Ok("writeback") => PimdirBodyFlush::Deferred {
            writeback: Some(writeback),
        },
        Ok(other) => panic!("PIMBENCH_FLUSH: each, deferred or writeback, not {other}"),
    }
}

/// Starts the writeback of a written body without waiting for it.
fn writeback(file: &fs::File) {
    use std::os::fd::AsRawFd;

    // SAFETY: the descriptor is open for as long as `file` is borrowed,
    // and the call reads no memory of this process.
    unsafe {
        libc::sync_file_range(file.as_raw_fd(), 0, 0, libc::SYNC_FILE_RANGE_WRITE);
    }
}

/// The folders `PIMBENCH_SKIP=1` leaves unsynced when their times did not
/// move since their last sync: those the store calls settled under the
/// unbounded scope `sync` asks. None without it.
fn settled_folders(store: &PimdirSourceStore) -> BTreeSet<String> {
    if env::var("PIMBENCH_SKIP").as_deref() != Ok("1") {
        return BTreeSet::new();
    }
    store
        .settled_collections()
        .unwrap()
        .into_iter()
        .filter(|(_, coverage)| coverage.scope == PimdirScope::default())
        .map(|(collection, _)| collection.0)
        .collect()
}

fn cmd_sync(args: &[String]) {
    let store_dir = PathBuf::from(&args[0]);
    let root = PathBuf::from(&args[1]);
    let state = PathBuf::from(&args[2]);
    let page = args.get(3).and_then(|p| p.parse().ok()).unwrap_or(1000);
    fs::create_dir_all(&state).unwrap();
    let folders = folders(&root).unwrap();
    let mut store = PimdirStore::open(&store_dir).unwrap().for_source("maildir");
    store.body_flush = body_flush();
    let flush = env::var("PIMBENCH_FLUSH").unwrap_or_else(|_| "default".into());
    let mut remote = Maildir {
        mtimes: Maildir::load_mtimes(&state),
        root,
        state,
        page,
        listings: BTreeMap::new(),
        read_bytes: 0,
        seen: BTreeMap::new(),
    };
    let phase = Phase::start();
    let (mut pulled, mut unreadable, mut skipped) = (0, 0, 0);
    let settled = settled_folders(&store);
    for folder in &folders {
        if settled.contains(folder) && remote.mtimes.get(folder) == Some(&remote.stat(folder).0) {
            skipped += 1;
            continue;
        }
        store.ensure_collection(folder, "message/rfc822").unwrap();
        let report = store
            .sync(
                PimdirCollectionId(folder.clone()),
                PimdirSyncOptions::default(),
                &mut remote,
            )
            .unwrap_or_else(|err| panic!("{folder}: {err:?}"));
        pulled += report.pulled;
        unreadable += report.unreadable;
        remote.listings.remove(folder);
        remote.settle(folder);
    }
    remote.save_mtimes().unwrap();
    println!(
        "sync folders={} skipped={skipped} pulled={pulled} unreadable={unreadable} read_mib={:.1} secs={:.2} peak_rss_mib={:.0}",
        folders.len(),
        remote.read_bytes as f64 / 1048576.0,
        phase.wall_secs(),
        measure::peak_rss_kib() as f64 / 1024.0,
    );
    println!(
        "{}",
        phase.line(
            "sync",
            pulled as u64,
            &[
                ("folders", folders.len().to_string()),
                ("unreadable", unreadable.to_string()),
                ("mail_bytes", remote.read_bytes.to_string()),
                ("flush", flush),
            ],
        )
    );
}

fn cmd_index(args: &[String]) {
    let store_dir = PathBuf::from(&args[0]);
    let phase = Phase::start();
    let mut index = PimdirIndex::open(&store_dir).unwrap();
    if let Ok(fts) = env::var("PIMBENCH_FTS") {
        let conn = rusqlite::Connection::open(store_dir.join("index.db")).unwrap();
        for setting in fts.split(',').filter(|s| !s.is_empty()) {
            let (key, value) = setting.split_once('=').unwrap();
            for table in ["object_text", "summary_text"] {
                conn.execute(
                    &format!("INSERT INTO {table}({table}, rank) VALUES (?1, ?2)"),
                    rusqlite::params![key, value.parse::<i64>().unwrap()],
                )
                .unwrap();
            }
        }
    }
    let mut options = PimdirIndexRefreshOptions::default();
    if let Some(batch) = env::var("PIMBENCH_BATCH").ok().and_then(|b| b.parse().ok()) {
        options.batch = batch;
    }
    let report = index.refresh(&options).unwrap();
    if let Ok(end) = env::var("PIMBENCH_END") {
        let conn = rusqlite::Connection::open(store_dir.join("index.db")).unwrap();
        for table in ["object_text", "summary_text"] {
            match end.as_str() {
                "optimize" => conn
                    .execute(
                        &format!("INSERT INTO {table}({table}) VALUES ('optimize')"),
                        [],
                    )
                    .map(|_| ())
                    .unwrap(),
                "checkpoint" => (),
                _ => {
                    let pages: i64 = end.parse().unwrap();
                    loop {
                        let before = conn.total_changes();
                        conn.execute(
                            &format!("INSERT INTO {table}({table}, rank) VALUES ('merge', ?1)"),
                            [pages],
                        )
                        .unwrap();
                        if conn.total_changes() - before <= 1 {
                            break;
                        }
                    }
                }
            }
        }
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
    }
    println!(
        "index folded={} indexed={} secs={:.2} peak_rss_mib={:.0}",
        report.folded,
        report.indexed,
        phase.wall_secs(),
        measure::peak_rss_kib() as f64 / 1024.0,
    );
    println!(
        "{}",
        phase.line(
            "index",
            report.indexed,
            &[
                ("folded", report.folded.to_string()),
                ("expanded", report.expanded.to_string()),
                ("bounded", report.bounded.to_string()),
                ("removed", report.removed.to_string()),
            ],
        )
    );
}

fn median(times: &mut [Duration]) -> f64 {
    times.sort();
    times[times.len() / 2].as_secs_f64() * 1000.0
}

/// Each query `REPEATS` times as a count, a page and a threaded page, each
/// printed with its medians and its phase line, the query last on it.
fn cmd_query(args: &[String]) {
    let store_dir = PathBuf::from(&args[0]);
    let repeats: usize = args[1].parse().unwrap();
    let all = Phase::start();
    let index = PimdirIndex::open_query(&store_dir).unwrap();
    let options = PimdirSearchOptions::default();
    let mut answered = 0;
    for raw in &args[2..] {
        let query = match PimdirQuery::parse(raw) {
            Ok(query) => query,
            Err(err) => {
                println!("query {raw:?} refused: {err:?}");
                continue;
            }
        };
        let phase = Phase::start();
        let (mut page_t, mut count_t, mut threads_t) = (Vec::new(), Vec::new(), Vec::new());
        let (mut hits, mut count, mut threads) = (0, 0, 0);
        for _ in 0..repeats {
            let at = Instant::now();
            hits = index.search(&query, &options).unwrap().hits.len();
            page_t.push(at.elapsed());
            let at = Instant::now();
            count = index.count(&query, &options.clock).unwrap().count;
            count_t.push(at.elapsed());
            let at = Instant::now();
            threads = index.threads(&query, &options).unwrap().threads.len();
            threads_t.push(at.elapsed());
        }
        let (count_ms, page_ms, threads_ms) = (
            median(&mut count_t),
            median(&mut page_t),
            median(&mut threads_t),
        );
        println!(
            "query {raw:?} count={count} count_ms={count_ms:.1} page={hits} page_ms={page_ms:.1} threads={threads} threads_ms={threads_ms:.1}",
        );
        println!(
            "{}",
            phase.line(
                "query",
                count,
                &[
                    ("repeats", repeats.to_string()),
                    ("count_ms", format!("{count_ms:.1}")),
                    ("page_ms", format!("{page_ms:.1}")),
                    ("threads_ms", format!("{threads_ms:.1}")),
                    ("query", format!("{raw:?}")),
                ],
            )
        );
        answered += 1;
    }
    println!("{}", all.line("queries", answered, &[]));
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let arity = match args.first().map(String::as_str) {
        Some("sync") => 3,
        Some("index") => 1,
        Some("query") => 2,
        _ => usize::MAX,
    };
    if args.len() <= arity {
        eprintln!("usage: pimbench sync STORE MAILDIR_ROOT STATE_DIR [PAGE]");
        eprintln!("       pimbench index STORE");
        eprintln!("       pimbench query STORE REPEATS QUERY...");
        return ExitCode::from(2);
    }
    match args[0].as_str() {
        "sync" => cmd_sync(&args[1..]),
        "index" => cmd_index(&args[1..]),
        _ => cmd_query(&args[1..]),
    }
    ExitCode::SUCCESS
}
