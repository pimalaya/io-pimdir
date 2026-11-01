//! The crate's object naming against the format's own vectors (pimdir SPEC
//! §16).
//!
//! The one vector file the format makes a MUST, and the reason is the
//! failure mode rather than the importance: a store whose two writers
//! name the same body differently reports nothing. It does not error and
//! no read returns a wrong answer; it silently never deduplicates and
//! never finds the blob the other side wrote. Prose cannot close that,
//! two readers of the same prose being what produced it.
//!
//! The expected values were derived from the algorithm and prose rather
//! than by running any implementation, so this crate can genuinely
//! disagree with them.
//!
//! The vectors are the vendored copy under spec/vectors/, so the suite
//! always runs; a case a later phase fixes is on tests/pending_vectors.txt.

#![cfg(feature = "client")]

mod pending;

use std::{fs, io::Write, path::PathBuf};

use io_pimdir::{
    client::{PimdirStore, blobs::PimdirBlobs},
    hash::{self, PimdirHasher},
    object::PimdirHash,
};
use serde_json::Value;

/// The vectors of vectors/objects.json, vendored at the revision
/// spec/REVISION names.
fn vectors() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("spec/vectors/objects.json");
    let text = fs::read_to_string(path).unwrap();

    serde_json::from_str(&text).unwrap()
}

/// The body a case describes: its bytes verbatim, or the pattern the long
/// ones are generated from (byte `i` is `i mod 251`, the BLAKE3
/// project's own convention, so those cases check against its published
/// vectors).
fn body(case: &Value) -> Vec<u8> {
    let len = case["body_len"].as_u64().unwrap() as usize;

    match case["body_hex"].as_str() {
        Some(hex) => {
            let bytes: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            assert_eq!(
                bytes.len(),
                len,
                "case {} declares a length its bytes do not match",
                case["label"]
            );
            bytes
        }
        None => {
            assert!(
                case["body_pattern"].is_string(),
                "case {} carries neither bytes nor a pattern",
                case["label"]
            );
            (0..len).map(|i| (i % 251) as u8).collect()
        }
    }
}

/// Object naming is the format's one MUST: every body names what the
/// vectors say under BLAKE3, and lands at the same sharded path.
#[test]
fn every_body_names_what_the_format_says() {
    let vectors = vectors();
    assert_eq!(vectors["v"], 2, "objects.json at another version");

    let cases = vectors["objects"].as_array().unwrap();
    assert!(!cases.is_empty(), "the vectors carry no object");

    let outcomes = cases
        .iter()
        .map(|case| {
            let label = case["label"].as_str().unwrap().to_string();
            (label, pending::check(|| names(case)))
        })
        .collect();
    pending::settle("objects", outcomes);
}

/// One case's body names what it states, and lands at its path.
fn names(case: &Value) {
    let label = case["label"].as_str().unwrap();
    let expected = &case["blake3"];
    let name = hash::hash(&body(case));
    assert_eq!(name.0, expected["name"].as_str().unwrap(), "{label}");

    // the shard path §5 derives from the name is as normative as the name:
    // two writers agreeing on the name and disagreeing on where it lives
    // still never find each other's bodies. Rooted at an empty store
    // directory, so the path reads exactly as the vectors write it.
    let blobs = PimdirBlobs::open("");
    assert_eq!(
        blobs.path(&PimdirHash(name.0.clone())).unwrap(),
        PathBuf::from(expected["path"].as_str().unwrap()),
        "{label} lands elsewhere",
    );
}

/// A body fed in pieces names what the same body fed whole names.
///
/// The streamed path is what §14's byteless `StoreObject` rides, and
/// where a hasher most often breaks: the vectors carry 1023, 1024 and
/// 1025 bytes because that is BLAKE3's chunk boundary.
#[test]
fn a_streamed_body_names_what_a_whole_one_names() {
    let vectors = vectors();

    for case in vectors["objects"].as_array().unwrap() {
        let label = case["label"].as_str().unwrap();
        let body = body(case);

        let mut hasher = PimdirHasher::new();
        for chunk in body.chunks(7) {
            hasher.update(chunk);
        }
        assert_eq!(
            hasher.finish().0,
            case["blake3"]["name"].as_str().unwrap(),
            "{label}, streamed in 7-byte pieces",
        );
    }
}

/// The collector owns a file by its position (STORAGE §3, §5): one sitting
/// at the shard path its name derives is a body, and anything else under
/// objects/ is a writer's temporary or somebody else's file, left alone.
#[test]
fn the_collector_unlinks_only_the_files_at_their_shard_path() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap();
    let blobs = store.blobs();

    // an orphan at its path: a body a crash left with no row
    let mut writer = store.blob_stage().writer().unwrap();
    writer.write_all(b"orphan").unwrap();
    let orphan = writer.commit().unwrap();
    assert_eq!(orphan.size, 6);
    let orphan = blobs.path(&blobs.hash(b"orphan")).unwrap();

    // a temporary, a foreign file at the root and one inside a shard
    let objects = dir.path().join("objects");
    let tmp = objects.join(".tmp-9-9");
    fs::write(&tmp, b"in flight").unwrap();
    let readme = objects.join("README");
    fs::write(&readme, b"not a body").unwrap();
    let stray = orphan.parent().unwrap().join("notes.txt");
    fs::write(&stray, b"not a body either").unwrap();

    assert_eq!(
        blobs.scan().unwrap().files.len(),
        1,
        "only the orphan is a body"
    );
    let report = store.collect_garbage().unwrap();
    assert_eq!((report.objects, report.blobs, report.bytes), (0, 1, 6));
    assert_eq!(report.strays.len(), 3, "the three are reported");
    assert!(!orphan.is_file(), "the orphan is taken");
    assert!(tmp.is_file(), "a writer's temporary is not");
    assert!(readme.is_file() && stray.is_file(), "nor a foreign file");
}
