//! A body served whole by the CLI is verified before its first byte leaves
//! (pimdir STORAGE §7). Apart from tests/blob_store.rs: this test spawns
//! the binary, and a fork inherits every lock descriptor its process holds
//! until it executes, so the lock tests there would meet a lock a dropped
//! handle no longer holds.

#![cfg(all(feature = "client", feature = "cli"))]

use std::{fs, path::Path};

use io_pimdir::client::PimdirStore;

/// A card longer than a copy's buffer, so a stream would hand on its
/// first bytes before the read reaching its end checks it.
fn long_card() -> Vec<u8> {
    let note = "x".repeat(64 * 1024);
    format!("BEGIN:VCARD\r\nVERSION:4.0\r\nUID:long\r\nFN:Long\r\nNOTE:{note}\r\nEND:VCARD\r\n")
        .into_bytes()
}

/// Flips the first bit of a body on disk.
fn flip(path: &Path) {
    let mut bytes = fs::read(path).unwrap();
    bytes[0] ^= 1;
    fs::write(path, &bytes).unwrap();
}

/// A body served whole is verified before its first byte leaves (STORAGE
/// §7): `pimdir item export` of a corrupt body prints nothing.
#[test]
fn an_export_serves_no_byte_of_a_corrupt_body() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap();
    store.ensure_collection("contacts", "text/vcard").unwrap();
    let card = long_card();
    let (_, seq) = store.put_item("contacts", &card).unwrap();
    let path = store.blobs().path(&store.hash(&card)).unwrap();
    drop(store);
    flip(&path);

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_pimdir"))
        .arg("--store")
        .arg(dir.path())
        .args(["item", "export", &seq.to_string()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    // NOTE: the CLI prints its error on stdout; no byte of the body.
    let printed = String::from_utf8_lossy(&output.stdout);
    assert!(!printed.contains("xxxx"), "{} bytes printed", printed.len());
}
