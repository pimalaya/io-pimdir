//! # Shrinking a campaign's failure
//!
//! One failure a campaign found (campaign.rs), named by the key its
//! summary prints, `<profile>:<seed>:<suite>[:<history>[:<step>]]`, made
//! smaller while the same failure stands (pimdir-generator's `shrink`) and
//! written where it lands:
//!
//! - an engine step of the replay as a pimdir sync vector (format 11, the
//!   next free number of spec/vectors/sync/, a label and a note to restate
//!   from SYNC before it lands) and the fixtures it names;
//! - a failure of another suite as an io-pimdir test: the seed and what the
//!   shrunk model kept of the profile's, asserted to answer as the oracle;
//! - a crash cut, which a sync vector cannot state (format 11 holds one
//!   run), as its key and step, to be replayed by the suite.
//!
//! ```sh
//! PIMDIR_SHRINK=histories:42:replay:mail:5 PIMDIR_SHRINK_OUT=/tmp/shrunk \
//!   nix develop --command cargo test --release --all-features \
//!   --test generated shrink -- --ignored --nocapture
//! ```
//!
//! `PIMDIR_SHRINK_SIZE` gives a count or bytes store's size as the
//! campaign states it. The step is the first failing one when the key names
//! none; the failure kept is the first difference found there.

use std::{
    collections::BTreeSet,
    env, fs,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
};

use io_pimdir::client::PimdirStore;
use pimdir_generator::{
    REVISION,
    model::Model,
    shrink::{self, Case, Kept, signature},
    vector,
};
use rusqlite::Connection;

use crate::{campaign::Job, replay};

#[test]
#[ignore = "shrinks one campaign failure: PIMDIR_SHRINK and PIMDIR_SHRINK_OUT"]
fn a_failure_shrinks_into_a_vector_or_a_test() {
    let key = env::var("PIMDIR_SHRINK").expect("PIMDIR_SHRINK names a failure");
    let out = PathBuf::from(env::var("PIMDIR_SHRINK_OUT").expect("PIMDIR_SHRINK_OUT"));
    fs::create_dir_all(&out).unwrap();
    let out = out.canonicalize().unwrap();
    let tree = Path::new(env!("CARGO_MANIFEST_DIR"))
        .canonicalize()
        .unwrap();
    assert!(
        !out.starts_with(&tree),
        "the shrunk case stays outside the tree"
    );

    let parts: Vec<&str> = key.split(':').collect();
    let profile = parts[0];
    let suite = campaign_suite(parts[2]);
    let job = Job {
        seed: parts[1].parse().expect("a seed"),
        profile: String::from(profile),
        suite,
        history: None,
        size: env::var("PIMDIR_SHRINK_SIZE")
            .ok()
            .map(|size| size.parse().unwrap()),
    };

    let report = match (profile, suite) {
        ("histories" | "quirks", "replay") => engine(&job, parts[3], parts.get(4), &out),
        ("histories" | "quirks", _) => format!(
            "{key}: a run cut after a commit and run again spans two runs, which a sync \
             vector does not state (format 11); replay it with the crash suite"
        ),
        _ => model(&job, &key, &out),
    };
    fs::write(out.join("shrink.txt"), &report).unwrap();
    println!("{report}");
}

/// The suite of campaign.rs a key names.
fn campaign_suite(name: &str) -> &'static str {
    ["structure", "histories", "count"]
        .iter()
        .flat_map(|profile| crate::campaign::suites(profile).iter())
        .find(|suite| **suite == name)
        .copied()
        .unwrap_or_else(|| panic!("no suite {name}"))
}

/// Shrinks the replay's failure at `step` of the history `name` into a
/// sync vector.
fn engine(job: &Job, name: &str, step: Option<&&str>, out: &Path) -> String {
    let (_, history) = job
        .histories()
        .into_iter()
        .find(|(held, _)| *held == name)
        .unwrap_or_else(|| panic!("no history {name}"));
    let history = replay::declared(&history);
    let found = replay::replay(&history);
    let step = match step {
        Some(step) => step.parse().expect("a step"),
        None => found.first().expect("a failing step").0,
    };
    let at: Vec<String> = found
        .iter()
        .filter(|(index, _)| *index == step)
        .map(|(_, text)| crate::campaign::unstepped(text.clone()).1)
        .collect();
    let target = signature(at.first().expect("a difference at the step"));
    let originals: BTreeSet<String> = at.iter().map(|text| signature(text)).collect();

    let case = Case::of(&history, step).expect("a run at the step");
    let fails = |case: &Case| {
        let found = panic::catch_unwind(AssertUnwindSafe(|| replay::replay_case(case)));
        let Ok(found) = found else {
            return false;
        };
        let signatures: BTreeSet<String> = found.iter().map(|text| signature(text)).collect();
        signatures.contains(&target) && signatures.is_subset(&originals)
    };
    if !fails(&case) {
        return format!(
            "{name} step {step} at seed {}: the step does not fail alone, its \
             failure needs the steps before it (`{target}`)",
            job.seed
        );
    }
    let (small, dropped) = shrink::shrink(case, Case::candidates, fails);

    let pair = replay::declared(&small.history());
    let run = pair.steps[1].run.as_ref().expect("the case's run");
    let asked = replay::asked(&pair.steps[0], &pair.steps[1], run);
    let number = next_vector();
    let label = format!(
        "a generated {} history ({name}, step {step}) shrunk: {target}",
        job.profile
    );
    let note = format!(
        "shrunk by io-pimdir tests/generated/shrink.rs from generator revision {REVISION}, \
         seed {}, profile {}, history {name}, step {step}, dropping {}; the expectation is \
         pimdir-generator's model, a second reading of SYNC: restate the label and the note \
         from SYNC before landing",
        job.seed,
        job.profile,
        match dropped.is_empty() {
            true => String::from("nothing"),
            false => dropped.join(", "),
        },
    );
    let written = match vector::write(&small, &asked, &label, &note, &summary_columns()) {
        Ok(written) => written,
        Err(reason) => {
            return format!(
                "{name} step {step} at seed {}: shrunk ({} dropped), but {reason}: \
                 reported with its history",
                job.seed,
                dropped.len()
            );
        }
    };
    let file = format!("{number}-a-generated-case.json");
    fs::create_dir_all(out.join("vectors/sync")).unwrap();
    fs::create_dir_all(out.join("vectors/fixtures")).unwrap();
    fs::write(
        out.join("vectors/sync").join(&file),
        serde_json::to_string_pretty(&written.json).unwrap() + "\n",
    )
    .unwrap();
    for (fixture, bytes) in &written.fixtures {
        fs::write(out.join("vectors/fixtures").join(fixture), bytes).unwrap();
    }
    format!(
        "{name} step {step} at seed {}: `{target}`\nshrunk from {} to {} items ({} steps: {})\n\
         wrote vectors/sync/{file} and {} fixtures under {}",
        job.seed,
        history.steps[step - 1].model.items.len(),
        small.before.items.len(),
        dropped.len(),
        dropped.join(", "),
        written.fixtures.len(),
        out.display(),
    )
}

/// The next free number of spec/vectors/sync/.
fn next_vector() -> usize {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("spec/vectors/sync");
    fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            name.split('-').next()?.parse::<usize>().ok()
        })
        .max()
        .unwrap_or(0)
        + 1
}

/// The columns of a fresh store's `summary` table, those a case's
/// summaries may state.
fn summary_columns() -> BTreeSet<String> {
    let dir = tempfile::tempdir().unwrap();
    drop(PimdirStore::open(dir.path()).unwrap());
    let conn = Connection::open(dir.path().join("pimdir.db")).unwrap();
    let mut statement = conn
        .prepare("SELECT name FROM pragma_table_info('summary')")
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Shrinks a failure of a suite over a model into an io-pimdir test.
fn model(job: &Job, key: &str, out: &Path) -> String {
    let start = job.model();
    let found = job.differences(start.clone());
    let unknown: Vec<String> = found
        .into_iter()
        .filter(|found| found.known.is_none())
        .map(|found| found.text)
        .collect();
    let Some(first) = unknown.first() else {
        return format!("{key}: no difference to shrink");
    };
    let target = signature(first);
    let originals: BTreeSet<String> = unknown.iter().map(|text| signature(text)).collect();
    let fails = |model: &Model| {
        let found = panic::catch_unwind(AssertUnwindSafe(|| job.differences(shrink::dense(model))));
        let Ok(found) = found else {
            return false;
        };
        let signatures: BTreeSet<String> = found
            .iter()
            .filter(|found| found.known.is_none())
            .map(|found| signature(&found.text))
            .collect();
        signatures.contains(&target) && signatures.is_subset(&originals)
    };
    let (small, dropped) = shrink::shrink(start.clone(), shrink::candidates, fails);
    let kept = Kept::of(&small);

    let model = match job.profile.as_str() {
        "structure" => format!(
            "Profile::Structure.generate(&Spec::load_default().unwrap(), {}).unwrap()",
            job.seed
        ),
        "count" => format!(
            "profile::count({}, {})",
            job.seed,
            job.size.unwrap_or(2_000)
        ),
        _ => format!("profile::bytes({}, {})", job.seed, job.size.unwrap_or(64)),
    };
    let tame = match job.suite {
        "index" | "search" | "crash-index" => format!("index::tamed({model})"),
        _ => model,
    };
    let call = match job.suite {
        "reader" => "reader::disagreements_of(model).1",
        "overlay" => "overlay::differences_of(model)",
        "drain" => "drain::differences_of(model)",
        "index" => "index::differences_of(model)",
        "search" => "search::differences_of(model, false, Readings::default(), None)",
        "crash-owner" => "crash::owner_cuts(model)",
        "crash-index" => "crash::index_cuts(model)",
        _ => "scale::differences(model)",
    };
    let items: Vec<String> = kept
        .items
        .iter()
        .map(|(collection, link_id)| {
            format!("            ({collection:?}.into(), {link_id:?}.into()),")
        })
        .collect();
    let test = format!(
        "/// Shrunk from {key} (generator revision {REVISION}): `{target}`.\n\
         #[test]\n\
         fn {}_{}_{}_answers_as_the_oracle() {{\n    \
             let kept = Kept {{\n        \
                 collections: vec!{:?}.into_iter().map(String::from).collect(),\n        \
                 items: vec![\n{}\n        ],\n        \
                 queue: vec!{:?},\n        \
                 capabilities: {},\n        \
                 references: {},\n    \
             }};\n    \
             let model = kept.apply(&{tame});\n    \
             let differences = {call};\n    \
             assert!(differences.is_empty(), \"{{differences:#?}}\");\n\
         }}\n",
        job.suite.replace('-', "_"),
        job.profile,
        job.seed,
        kept.collections,
        items.join("\n"),
        kept.queue,
        kept.capabilities,
        kept.references,
    );
    let file = out.join(format!(
        "{}_{}_{}.rs",
        job.suite.replace('-', "_"),
        job.profile,
        job.seed
    ));
    fs::write(&file, &test).unwrap();
    format!(
        "{key}: `{target}`\nshrunk from {} to {} items ({} steps)\nwrote {}",
        start.items.len(),
        small.items.len(),
        dropped.len(),
        file.display()
    )
}
