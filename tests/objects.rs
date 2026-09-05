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
//! The spec is a sibling checkout rather than a vendored copy, so the
//! test skips when it is absent and runs when the two sit side by side.

use std::{fs, io::Write, path::PathBuf};

use io_pimdir::object::PimdirHash;
use io_pimdir::{
    client::{PimdirStore, blobs::PimdirBlobs},
    hash::PimdirHashAlgo,
};
use serde_json::Value;

/// The canonical spec checkout, beside this one.
fn spec_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .join("pimdir");

    dir.join("vectors/objects.json").is_file().then_some(dir)
}

fn vectors() -> Option<Value> {
    let spec = spec_dir()?;
    let text = fs::read_to_string(spec.join("vectors/objects.json")).unwrap();

    Some(serde_json::from_str(&text).unwrap())
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

/// Object naming is the format's one MUST: every body names the same
/// under both algorithms, and lands at the same sharded path.
#[test]
fn every_body_names_what_the_format_says() {
    let Some(vectors) = vectors() else {
        eprintln!("skipped: no pimdir spec checkout beside this one");
        return;
    };

    let cases = vectors["objects"].as_array().unwrap();
    assert!(!cases.is_empty(), "the vectors carry no object");

    for case in cases {
        let label = case["label"].as_str().unwrap();
        let body = body(case);

        for (algo, spelling) in [
            (PimdirHashAlgo::Blake3, "blake3"),
            (PimdirHashAlgo::Sha256_128, "sha256-128"),
        ] {
            let expected = &case[spelling];
            let name = algo.hash(&body);
            assert_eq!(
                name.0,
                expected["name"].as_str().unwrap(),
                "{label} under {spelling}",
            );

            // the shard path §5 derives from the name is as normative as
            // the name: two writers agreeing on the name and disagreeing
            // on where it lives still never find each other's bodies.
            // Rooted at an empty store directory, so the path reads
            // exactly as the vectors write it.
            let blobs = PimdirBlobs::open("", algo);
            assert_eq!(
                blobs.path(&PimdirHash(name.0.clone())),
                PathBuf::from(expected["path"].as_str().unwrap()),
                "{label} under {spelling} lands elsewhere",
            );
        }
    }
}

/// A body fed in pieces names what the same body fed whole names.
///
/// The streamed path is what §14's byteless `StoreObject` rides, and
/// where a hasher most often breaks: the vectors carry 1023, 1024 and
/// 1025 bytes because that is BLAKE3's chunk boundary.
#[test]
fn a_streamed_body_names_what_a_whole_one_names() {
    let Some(vectors) = vectors() else {
        eprintln!("skipped: no pimdir spec checkout beside this one");
        return;
    };

    for case in vectors["objects"].as_array().unwrap() {
        let label = case["label"].as_str().unwrap();
        let body = body(case);

        for (algo, spelling) in [
            (PimdirHashAlgo::Blake3, "blake3"),
            (PimdirHashAlgo::Sha256_128, "sha256-128"),
        ] {
            let mut hasher = algo.hasher();
            for chunk in body.chunks(7) {
                hasher.update(chunk);
            }
            assert_eq!(
                hasher.finish().0,
                case[spelling]["name"].as_str().unwrap(),
                "{label} under {spelling}, streamed in 7-byte pieces",
            );
        }
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
    let mut writer = blobs.writer().unwrap();
    writer.write_all(b"orphan").unwrap();
    let orphan = writer.commit(&blobs.hash(b"orphan")).unwrap();
    assert_eq!(orphan, 6);
    let orphan = blobs.path(&blobs.hash(b"orphan"));

    // a temporary, a foreign file at the root and one inside a shard
    let objects = dir.path().join("objects");
    let tmp = objects.join(".tmp-9-9");
    fs::write(&tmp, b"in flight").unwrap();
    let readme = objects.join("README");
    fs::write(&readme, b"not a body").unwrap();
    let stray = orphan.parent().unwrap().join("notes.txt");
    fs::write(&stray, b"not a body either").unwrap();

    assert_eq!(blobs.files().unwrap().len(), 1, "only the orphan is a body");
    let report = store.collect_garbage().unwrap();
    assert_eq!((report.objects, report.blobs, report.bytes), (0, 1, 6));
    assert!(!orphan.is_file(), "the orphan is taken");
    assert!(tmp.is_file(), "a writer's temporary is not");
    assert!(readme.is_file() && stray.is_file(), "nor a foreign file");
}
