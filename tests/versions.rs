//! The order of format versions against the format's own vectors (pimdir
//! STORAGE §4.2, vectors/versions.json): every profile refuses a store of
//! another version, and says which of the two is the earlier.
//!
//! The vectors are the vendored copy under spec/vectors/, so the suite
//! always runs.

#![cfg(feature = "client")]

use std::{fs, path::PathBuf};

use io_pimdir::version::{PimdirFormatOrder, PimdirFormatRefusal};
use serde_json::Value;

/// Every pair orders as the vector says.
#[test]
fn every_version_pair_orders_as_its_vector_says() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("spec/vectors/versions.json");
    let vectors: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(vectors["v"], 1, "a format this suite reads");

    let cases = vectors["cases"].as_array().unwrap();
    assert!(!cases.is_empty());

    for case in cases {
        let refusal = PimdirFormatRefusal {
            found: case["found"].as_str(),
            own: case["own"].as_str().unwrap(),
            program: "this program",
        };
        let order = match case["order"].as_str().unwrap() {
            "earlier" => PimdirFormatOrder::Earlier,
            "same" => PimdirFormatOrder::Same,
            "later" => PimdirFormatOrder::Later,
            "unknown" => PimdirFormatOrder::Unknown,
            other => panic!("an order this suite does not know: {other}"),
        };
        assert_eq!(refusal.order(), order, "{case}");
    }
}
