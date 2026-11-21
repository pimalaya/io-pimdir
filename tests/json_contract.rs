//! The JSON contract pinned: the schema of every payload an integrator reads
//! (cairn/spec/cli.md, "Integrators read the JSON output"), as `pimdir
//! json-schema` prints it, against the copy committed under
//! tests/json_contract/, so a change of shape fails until the copy is updated
//! on purpose.
//!
//! The `$id` and the prose (`description`) are left out of the copies: they
//! name the release and explain the keys, neither is the shape. Run with
//! `PIMDIR_BLESS_CONTRACT=1` to write the copies from the binary, then read
//! the diff: a removed or renamed key, a changed type or a closed vocabulary
//! moves io-pimdir's major, which `formatVersion` follows.

#![cfg(feature = "cli")]

use std::{env, fs, path::Path, process::Command};

use serde_json::Value;

/// The payloads of the contract, by the key `pimdir json-schema` names them.
const CONTRACT: &[&str] = &[
    "pimdir-collection-list",
    "pimdir-count",
    "pimdir-error",
    "pimdir-item-list",
    "pimdir-item-show",
    "pimdir-queue-list",
    "pimdir-queue-status",
    "pimdir-search",
    "pimdir-show",
    "pimdir-store-info",
    "pimdir-tag",
    "pimdir-thread",
    "pimdir-version",
];

/// Drops every `description` holding prose, so a reworded doc comment is no
/// change of shape; a property named `description` holds a schema and stays.
fn shape(schema: &mut Value) {
    match schema {
        Value::Object(map) => {
            if map.get("description").is_some_and(Value::is_string) {
                map.remove("description");
            }
            map.values_mut().for_each(shape);
        }
        Value::Array(values) => values.iter_mut().for_each(shape),
        _ => (),
    }
}

/// The schemas the binary prints, by key, their `$id` and their prose left
/// out.
fn printed() -> Vec<(&'static str, Value)> {
    let dir = tempfile::tempdir().unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_pimdir"))
        .arg("json-schema")
        .arg("--dir")
        .arg(dir.path())
        .args(CONTRACT)
        .output()
        .unwrap();
    assert!(status.status.success(), "{status:?}");

    CONTRACT
        .iter()
        .map(|key| {
            let path = dir.path().join(format!("{key}.json"));
            let mut schema: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            schema.as_object_mut().unwrap().remove("$id");
            shape(&mut schema);
            (*key, schema)
        })
        .collect()
}

/// Every contract payload prints the schema its committed copy holds, and
/// the copies hold no other payload.
#[test]
fn every_contract_payload_matches_its_committed_schema() {
    let copies = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/json_contract");
    let bless = env::var_os("PIMDIR_BLESS_CONTRACT").is_some();
    let mut problems = Vec::new();

    for (key, schema) in printed() {
        let path = copies.join(format!("{key}.schema.json"));

        if bless {
            let json = serde_json::to_string_pretty(&schema).unwrap();
            fs::write(&path, json + "\n").unwrap();
            continue;
        }

        let Ok(bytes) = fs::read(&path) else {
            problems.push(format!("{key}: no committed copy at {}", path.display()));
            continue;
        };
        let committed: Value = serde_json::from_slice(&bytes).unwrap();

        if committed != schema {
            problems.push(format!("{key}: the schema differs from {}", path.display()));
        }
    }

    for entry in fs::read_dir(&copies).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        let key = name.strip_suffix(".schema.json").unwrap_or(&name);

        if !CONTRACT.contains(&key) {
            problems.push(format!("{name}: a copy naming no contract payload"));
        }
    }

    assert!(
        problems.is_empty(),
        "{}\n\nA minor only adds keys, and values to vocabularies declared open; \
         anything else moves io-pimdir's major. Rewrite the copies with \
         PIMDIR_BLESS_CONTRACT=1 once the change is meant.",
        problems.join("\n")
    );
}

/// `version` states the contract's major, the crate's own.
#[test]
fn the_version_probe_states_the_crate_major() {
    let output = Command::new(env!("CARGO_BIN_EXE_pimdir"))
        .args(["--json", "version"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let major: u64 = env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap();
    assert_eq!(json["formatVersion"], major);
    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
}
