//! # Campaigns
//!
//! The freeze plan's campaigns (pimdir cairn/changes/the-freeze-plan,
//! section 7): every differential suite of this directory run over the
//! `(generator revision, seed, profile)` triples tests/generated/
//! campaigns.json records, in a release build, every difference written
//! with the triple, the suite, the history and the step it was found at.
//!
//! ```sh
//! PIMDIR_CAMPAIGN=campaign-1 PIMDIR_CAMPAIGN_OUT=/tmp/campaign-1 \
//!   nix develop --command cargo test --release --all-features \
//!   --test generated campaign -- --ignored --nocapture
//! ```
//!
//! The output directory, outside the tree, receives jobs.jsonl (one line
//! a job: its triple, suite, history, seconds, differences, and whether it
//! panicked), differences.jsonl (one line a difference) and summary.txt
//! (the jobs and differences by suite, and each failure's signature with
//! the first job showing it, the triple tests/generated/shrink.rs takes).
//! `PIMDIR_CAMPAIGN_PROFILES` (a comma list) and `PIMDIR_CAMPAIGN_SEEDS`
//! (seeds per profile at most) cut a campaign down for a sizing run,
//! `PIMDIR_CAMPAIGN_THREADS` sets the workers (every core by default).

use std::{
    collections::BTreeMap,
    env,
    fs::{self, File},
    io::Write,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Instant,
};

use pimdir_generator::{
    REVISION,
    history::History,
    model::Model,
    profile::{self, Profile},
    quirks,
    shrink::signature,
    spec::Spec,
};
use serde_json::{Value, json};

use crate::{crash, drain, index, overlay, reader, replay, scale, search};

/// The file recording the campaigns.
const CAMPAIGNS: &str = "tests/generated/campaigns.json";

/// The items of a count store when the campaign states none.
const ITEMS: usize = 2_000;

/// What every size of a bytes store is divided by when the campaign
/// states nothing.
const SCALE: usize = 64;

#[test]
#[ignore = "a campaign, for a release build: PIMDIR_CAMPAIGN and PIMDIR_CAMPAIGN_OUT"]
fn a_campaign_runs_every_suite_over_its_seeds_and_profiles() {
    let name = env::var("PIMDIR_CAMPAIGN").expect("PIMDIR_CAMPAIGN names a campaign");
    let out = PathBuf::from(env::var("PIMDIR_CAMPAIGN_OUT").expect("PIMDIR_CAMPAIGN_OUT"));
    let tree = Path::new(env!("CARGO_MANIFEST_DIR"));
    fs::create_dir_all(&out).unwrap();
    let out = out.canonicalize().unwrap();
    assert!(
        !out.starts_with(tree.canonicalize().unwrap()),
        "the campaign's output stays outside the tree"
    );

    let campaign = Campaign::load(&name);
    let profiles: Option<Vec<String>> = env::var("PIMDIR_CAMPAIGN_PROFILES")
        .ok()
        .map(|list| list.split(',').map(String::from).collect());
    let most: Option<usize> = env::var("PIMDIR_CAMPAIGN_SEEDS")
        .ok()
        .map(|most| most.parse().unwrap());
    let mut jobs = campaign.jobs(profiles.as_deref(), most);
    jobs.sort_by_key(|job| std::cmp::Reverse(job.weight()));
    let threads = env::var("PIMDIR_CAMPAIGN_THREADS")
        .ok()
        .map(|threads| threads.parse().unwrap())
        .unwrap_or_else(|| thread::available_parallelism().map_or(1, usize::from));

    let started = Instant::now();
    let next = AtomicUsize::new(0);
    let jobs_log = Mutex::new(File::create(out.join("jobs.jsonl")).unwrap());
    let found_log = Mutex::new(File::create(out.join("differences.jsonl")).unwrap());
    let results: Mutex<Vec<Ran>> = Mutex::new(Vec::new());
    println!(
        "campaign {name}, revision {REVISION}: {} jobs on {threads} workers",
        jobs.len()
    );
    thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let at = next.fetch_add(1, Ordering::SeqCst);
                    let Some(job) = jobs.get(at) else {
                        break;
                    };
                    let clock = Instant::now();
                    let (found, panicked) =
                        match panic::catch_unwind(AssertUnwindSafe(|| job.run())) {
                            Ok(found) => (found, false),
                            Err(payload) => (vec![Found::panicked(payload)], true),
                        };
                    let seconds = clock.elapsed().as_secs_f64();
                    {
                        let mut log = found_log.lock().unwrap();
                        for found in &found {
                            let line = job.record(&name, found);
                            writeln!(log, "{line}").unwrap();
                        }
                    }
                    let line = json!({
                        "seed": job.seed,
                        "profile": job.profile,
                        "suite": job.suite,
                        "history": job.history,
                        "seconds": seconds,
                        "differences": found.len(),
                        "panicked": panicked,
                    });
                    writeln!(jobs_log.lock().unwrap(), "{line}").unwrap();
                    println!(
                        "[{:>5}/{}] {:>7.1}s {} {} {} {}: {}",
                        at + 1,
                        jobs.len(),
                        started.elapsed().as_secs_f64(),
                        job.profile,
                        job.seed,
                        job.suite,
                        job.history.unwrap_or(""),
                        found.len()
                    );
                    results.lock().unwrap().push(Ran {
                        job: job.clone(),
                        seconds,
                        found,
                        panicked,
                    });
                }
            });
        }
    });

    let results = results.into_inner().unwrap();
    let summary = summary(&name, &results, started.elapsed().as_secs_f64(), threads);
    fs::write(out.join("summary.txt"), &summary).unwrap();
    println!("{summary}");
}

/// One campaign as campaigns.json records it.
struct Campaign {
    cases: Vec<Case>,
}

/// One profile of a campaign and the seeds it runs.
struct Case {
    profile: String,
    seeds: Vec<u64>,
    size: Option<usize>,
}

impl Campaign {
    /// The campaign `name`, refused when it was recorded under another
    /// revision of the generator, whose seeds give other histories.
    fn load(name: &str) -> Self {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(CAMPAIGNS);
        let file: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        let campaign = file["campaigns"]
            .as_array()
            .unwrap()
            .iter()
            .find(|campaign| campaign["name"] == name)
            .unwrap_or_else(|| panic!("no campaign {name} in {CAMPAIGNS}"));
        assert_eq!(
            campaign["revision"].as_u64(),
            Some(u64::from(REVISION)),
            "{name} was recorded under another generator revision"
        );
        let cases = campaign["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|case| Case {
                profile: String::from(case["profile"].as_str().unwrap()),
                seeds: case["seeds"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|seed| seed.as_u64().unwrap())
                    .collect(),
                size: case["size"].as_u64().map(|size| size as usize),
            })
            .collect();
        Self { cases }
    }

    fn jobs(&self, profiles: Option<&[String]>, most: Option<usize>) -> Vec<Job> {
        let mut jobs = Vec::new();
        for case in &self.cases {
            if profiles.is_some_and(|profiles| !profiles.contains(&case.profile)) {
                continue;
            }
            let names: Vec<&'static str> = match case.profile.as_str() {
                "histories" => replay::histories(0).iter().map(|(name, _)| *name).collect(),
                "quirks" => quirks::histories(0).iter().map(|(name, _)| *name).collect(),
                _ => Vec::new(),
            };
            for seed in case.seeds.iter().take(most.unwrap_or(usize::MAX)) {
                for suite in suites(&case.profile) {
                    let job = |history| Job {
                        seed: *seed,
                        profile: case.profile.clone(),
                        suite,
                        history,
                        size: case.size,
                    };
                    match names.is_empty() {
                        true => jobs.push(job(None)),
                        false => jobs.extend(names.iter().map(|name| job(Some(*name)))),
                    }
                }
            }
        }
        jobs
    }
}

/// The suites a profile runs.
pub fn suites(profile: &str) -> &'static [&'static str] {
    match profile {
        "structure" => &[
            "reader",
            "overlay",
            "drain",
            "index",
            "search",
            "crash-owner",
            "crash-index",
        ],
        "histories" | "quirks" => &["replay", "crash-engine"],
        "count" | "bytes" => &["scale"],
        profile => panic!("no profile {profile}"),
    }
}

/// One suite over one seed of one profile, one history of it where the
/// profile holds several.
#[derive(Clone, Debug)]
pub struct Job {
    /// The seed.
    pub seed: u64,
    /// The profile.
    pub profile: String,
    /// The suite.
    pub suite: &'static str,
    /// The history's name, where the profile holds several.
    pub history: Option<&'static str>,
    /// The size a count or bytes store is drawn at.
    pub size: Option<usize>,
}

/// One difference a job found.
pub struct Found {
    /// The history step it was found after, where the suite replays one.
    pub step: Option<usize>,
    /// The known finding it stands under, if any.
    pub known: Option<&'static str>,
    /// What differs.
    pub text: String,
}

impl Found {
    fn of(text: String) -> Self {
        Self {
            step: None,
            known: None,
            text,
        }
    }

    fn panicked(payload: Box<dyn std::any::Any + Send>) -> Self {
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| {
                payload
                    .downcast_ref::<&str>()
                    .map(|text| String::from(*text))
            })
            .unwrap_or_default();
        Self::of(format!("the suite panicked: {message}"))
    }
}

impl Job {
    /// The histories the job's profile holds at its seed.
    pub fn histories(&self) -> Vec<(&'static str, History)> {
        match self.profile.as_str() {
            "histories" => replay::histories(self.seed).into_iter().collect(),
            _ => quirks::histories(self.seed),
        }
    }

    /// The model the job's profile draws at its seed, tamed where the
    /// suite indexes it (index.rs, G8).
    pub fn model(&self) -> Model {
        let spec = Spec::load_default().unwrap();
        let model = match self.profile.as_str() {
            "structure" => Profile::Structure.generate(&spec, self.seed).unwrap(),
            "count" => profile::count(self.seed, self.size.unwrap_or(ITEMS)),
            "bytes" => profile::bytes(self.seed, self.size.unwrap_or(SCALE)),
            profile => panic!("{profile} draws histories"),
        };
        match self.suite {
            "index" | "search" | "crash-index" => index::tamed(model),
            _ => model,
        }
    }

    /// What the suite answers over `model`, a model [`Job::model`] draws.
    pub fn differences(&self, model: Model) -> Vec<Found> {
        let known = |found: Vec<(Option<&'static str>, String)>| {
            found
                .into_iter()
                .map(|(known, text)| {
                    let (step, text) = unstepped(text);
                    Found { step, known, text }
                })
                .collect()
        };
        let plain = |found: Vec<String>| {
            found
                .into_iter()
                .map(|text| {
                    let (step, text) = unstepped(text);
                    Found {
                        step,
                        known: None,
                        text,
                    }
                })
                .collect()
        };
        match self.suite {
            "reader" => known(reader::disagreements_of(model).1),
            "overlay" => known(overlay::differences_of(model)),
            "drain" => known(drain::differences_of(model)),
            "index" => known(index::differences_of(model)),
            "search" => plain(search::differences_of(model, false)),
            "crash-owner" => plain(crash::owner_cuts(model)),
            "crash-index" => plain(crash::index_cuts(model)),
            "scale" => plain(scale::differences(model)),
            suite => panic!("{suite} replays histories"),
        }
    }

    /// What the suite answers over `history`.
    pub fn replayed(&self, history: &History) -> Vec<Found> {
        let found = match self.suite {
            "replay" => replay::replay(history)
                .into_iter()
                .map(|(step, text)| (step, unstepped(text)))
                .collect(),
            _ => crash::engine_cuts(history)
                .into_iter()
                .map(|(step, text)| (step, (None, text)))
                .collect::<Vec<_>>(),
        };
        found
            .into_iter()
            .map(|(step, (_, text))| Found {
                step: Some(step),
                known: None,
                text,
            })
            .collect()
    }

    fn run(&self) -> Vec<Found> {
        match self.history {
            Some(name) => {
                let (_, history) = self
                    .histories()
                    .into_iter()
                    .find(|(held, _)| *held == name)
                    .expect("a history of the profile");
                self.replayed(&history)
            }
            None => self.differences(self.model()),
        }
    }

    /// What a job's time grows with, the longest started first.
    fn weight(&self) -> usize {
        match self.suite {
            "scale" => 100,
            "crash-engine" => 50,
            "search" | "crash-index" | "crash-owner" => 20,
            _ => 10,
        }
    }

    fn record(&self, campaign: &str, found: &Found) -> Value {
        json!({
            "campaign": campaign,
            "revision": REVISION,
            "seed": self.seed,
            "profile": self.profile,
            "suite": self.suite,
            "history": self.history,
            "step": found.step,
            "known": found.known,
            "signature": signature(&found.text),
            "text": found.text,
        })
    }

    /// The triple and suite tests/generated/shrink.rs takes.
    fn shrink_key(&self, step: Option<usize>) -> String {
        let mut key = format!("{}:{}:{}", self.profile, self.seed, self.suite);
        if let Some(name) = self.history {
            key.push_str(&format!(":{name}"));
        }
        if let Some(step) = step {
            key.push_str(&format!(":{step}"));
        }
        key
    }
}

/// A difference's text without the `step N (label)` it leads with, and
/// that step.
pub fn unstepped(text: String) -> (Option<usize>, String) {
    let Some(rest) = text.strip_prefix("step ") else {
        return (None, text);
    };
    let Some((number, rest)) = rest.split_once(" (") else {
        return (None, text);
    };
    let Ok(step) = number.parse() else {
        return (None, text);
    };
    let end = [rest.find("): "), rest.find("), ")]
        .into_iter()
        .flatten()
        .min();
    match end {
        Some(end) => (Some(step), String::from(&rest[end + 3..])),
        None => (None, text),
    }
}

/// A job run: what it found, in how long, and whether it panicked.
struct Ran {
    job: Job,
    seconds: f64,
    found: Vec<Found>,
    panicked: bool,
}

/// The campaign's results by suite and by failure signature.
fn summary(name: &str, results: &[Ran], wall: f64, threads: usize) -> String {
    let mut by_suite: BTreeMap<&str, (usize, usize, usize, usize, f64)> = BTreeMap::new();
    let mut by_signature: BTreeMap<(String, String), (usize, String)> = BTreeMap::new();
    for Ran {
        job,
        seconds,
        found,
        panicked,
    } in results
    {
        let entry = by_suite.entry(job.suite).or_default();
        entry.0 += 1;
        entry.1 += usize::from(found.iter().any(|found| found.known.is_none()));
        entry.2 += found.iter().filter(|found| found.known.is_none()).count();
        entry.3 += usize::from(*panicked);
        entry.4 += seconds;
        for found in found.iter().filter(|found| found.known.is_none()) {
            let key = (String::from(job.suite), signature(&found.text));
            let first = by_signature
                .entry(key)
                .or_insert_with(|| (0, job.shrink_key(found.step)));
            first.0 += 1;
        }
    }

    let mut text = format!(
        "campaign {name}, generator revision {REVISION}: {} jobs in {wall:.0} s on {threads} workers\n\n",
        results.len()
    );
    text.push_str("suite          jobs  failing  differences  panicked  cpu-seconds\n");
    for (suite, (jobs, failing, differences, panicked, seconds)) in &by_suite {
        text.push_str(&format!(
            "{suite:<13} {jobs:>5}  {failing:>7}  {differences:>11}  {panicked:>8}  {seconds:>11.0}\n"
        ));
    }
    text.push_str("\nfailures by signature (count, first PIMDIR_SHRINK key):\n");
    if by_signature.is_empty() {
        text.push_str("none\n");
    }
    for ((suite, signature), (count, key)) in &by_signature {
        text.push_str(&format!(
            "{count:>6}  {suite}: {signature}\n        {key}\n"
        ));
    }
    text
}
