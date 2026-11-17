//! What a rekey costs as the collection grows (SYNC §8): the coroutine's own
//! steps timed apart from the store's write, at sizes `PIMDIR_SCALE` names
//! (comma-separated, 2,000, 4,000 and 8,000 members by default). A rebuild
//! finds each old copy of an identity through one index and the hub drops
//! each handle through one lookup, so both grow with the collection, not
//! its square.
//!
//! ```sh
//! cargo test --release --all-features --test engine rekey_scale \
//!     -- --ignored --nocapture
//! ```
//!
//! Ignored by default: it measures, and asserts only that every member is
//! carried. The numbers are indicative, one machine's.

use std::time::{Duration, Instant};

use io_pimdir::{
    coroutine::{PimdirArg, PimdirCoroutine, PimdirCoroutineState, PimdirYield},
    rekey::PimdirRekey,
    remote::PimdirRemote,
    sync::PimdirSyncOptions,
};

use crate::common::{Client, MemRemote};

/// Rekeys a mailbox of `count` distinct messages, renumbered, timing the
/// coroutine's own steps apart from the store's write.
fn rekey(count: usize) -> (Duration, Duration, Duration, usize) {
    let mut remote = MemRemote::default();
    for n in 0..count {
        remote.seed(
            "inbox",
            &format!("{n:06}"),
            &format!("msg-{n}"),
            &[],
            b"body",
        );
    }
    let mut client = Client::new(remote);
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    client.remote_mut().renumber("inbox", 1);

    let mut coroutine = PimdirRekey::new("inbox");
    let (mut engine, mut write, mut other) = (Duration::ZERO, Duration::ZERO, Duration::ZERO);
    let mut arg: Option<PimdirArg> = None;
    let report = loop {
        let start = Instant::now();
        let state = coroutine.resume(arg.take());
        engine += start.elapsed();
        let yielded = match state {
            PimdirCoroutineState::Complete(report) => break report.unwrap(),
            PimdirCoroutineState::Yielded(yielded) => yielded,
        };
        let writing = matches!(yielded, PimdirYield::WantsWrite(_));
        let start = Instant::now();
        arg = Some(match client.store.service(yielded).unwrap() {
            Ok(arg) => arg,
            Err(PimdirYield::WantsEnumerate {
                collection,
                request,
            }) => PimdirArg::Enumerate(client.remote.enumerate(&collection, request).unwrap()),
            Err(PimdirYield::WantsFetch {
                collection,
                handles,
                tier,
            }) => PimdirArg::Fetch(client.remote.fetch(&collection, handles, tier).unwrap()),
            Err(_) => unreachable!(),
        });
        match writing {
            true => write += start.elapsed(),
            false => other += start.elapsed(),
        }
    };
    assert_eq!(report.rekeyed, count);
    (engine, write, other, report.rekeyed)
}

#[test]
#[ignore]
fn a_rekey_grows_with_the_collection() {
    let sizes: Vec<usize> = std::env::var("PIMDIR_SCALE")
        .unwrap_or_else(|_| "2000,4000,8000".into())
        .split(',')
        .map(|size| size.parse().unwrap())
        .collect();
    for count in sizes {
        let (engine, write, other, rekeyed) = rekey(count);
        println!(
            "{count:>6} members: coroutine {:>9.1} ms, store write {:>9.1} ms, load+listing {:>8.1} ms (rekeyed {rekeyed})",
            engine.as_secs_f64() * 1e3,
            write.as_secs_f64() * 1e3,
            other.as_secs_f64() * 1e3,
        );
    }
}
