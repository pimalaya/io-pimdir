//! Build script: generates the canonical SQL constants from the
//! specification vendored under spec/, and bakes the CLI's build
//! metadata.
//!
//! spec/ is a byte-for-byte copy of the pimdir repository's
//! migrations/storage/ and queries/storage/; one file per statement,
//! named after it, sorted by the profile that runs it. Re-vendor by
//! copying the two directories over, and tests/spec_drift.rs proves the
//! copy against a sibling checkout.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let spec = root.join("spec");
    println!("cargo:rerun-if-changed=spec");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("canonical.rs");
    fs::write(out, canonical(&spec)).unwrap();

    #[cfg(feature = "cli")]
    cli();
}

/// The generated module body: the migrations, one constant per statement
/// with the file's leading comment as its documentation, and the index.
fn canonical(spec: &Path) -> String {
    let mut out = String::new();

    let migrations = files(&spec.join("migrations/storage"));
    out += "/// The schema version: the number of canonical migrations.\n";
    out += &format!("pub const VERSION: i64 = {};\n\n", migrations.len());
    for (index, path) in migrations.iter().enumerate() {
        out += &format!(
            "/// Schema migration {0:04}, migrations/storage/{1} verbatim.\n\
             pub const MIGRATION_{0:04}: &str = include_str!({2:?});\n\n",
            index + 1,
            path.file_name().unwrap().to_str().unwrap(),
            path.display(),
        );
    }
    out += "/// Every canonical migration in order, the runner of STORAGE §6 applying\n";
    out += "/// each one above `user_version`.\n";
    out += "pub const MIGRATIONS: &[&str] = &[\n";
    for index in 0..migrations.len() {
        out += &format!("    MIGRATION_{:04},\n", index + 1);
    }
    out += "];\n\n";

    let mut names = Vec::new();
    for profile in ["read", "queue", "owner"] {
        for path in files(&spec.join("queries/storage").join(profile)) {
            let name = path.file_stem().unwrap().to_str().unwrap().to_uppercase();
            let text = fs::read_to_string(&path).unwrap();
            let comment: Vec<&str> = text
                .lines()
                .take_while(|line| line.starts_with("--"))
                .collect();
            if comment.is_empty() {
                let stem = path.file_stem().unwrap().to_str().unwrap();
                out += &format!("#[doc = \"The canonical `{stem}` statement (STORAGE §4.4).\"]\n");
            }
            for line in comment {
                let doc = line
                    .trim_start_matches('-')
                    .trim_start()
                    .replace('[', "\\[");
                out += &format!("#[doc = {doc:?}]\n");
            }
            out += &format!(
                "pub const {name}: &str = include_str!({:?});\n\n",
                path.display()
            );
            names.push(name);
        }
    }

    out += "/// Every canonical statement, paired with its constant name.\n";
    out += "pub const CANONICAL: &[(&str, &str)] = &[\n";
    for name in &names {
        out += &format!("    ({name:?}, {name}),\n");
    }
    out += "];\n";

    out
}

/// The files of a directory, in name order.
fn files(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("{}: {err}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    paths
}

#[cfg(feature = "cli")]
fn cli() {
    use pimalaya_cli::build::{features_env, git_envs, target_envs};

    features_env(include_str!("./Cargo.toml"));
    target_envs();
    git_envs();
}
