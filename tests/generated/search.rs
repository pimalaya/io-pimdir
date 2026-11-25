//! # Search against the oracle on generated stores
//!
//! io-pimdir's query client against the generator's reading of SEARCH §8
//! (pimdir-generator's `query` module, written from SEARCH.md alone): per
//! seed, the structure profile's store taken through index.rs's run of
//! owner writes and the model through the same writes (the generator's
//! `upkeep`), the index refreshed after each, and one query per form of
//! the grammar, its ids, addresses, words and dates drawn from the model,
//! asked of both. The hits compare as a set, as a list in every order but
//! `relevance`, page by page at limits 1 to 3, by count, and with the
//! coverage the answer states.
//!
//! The oracle answers three-valued: an item it cannot place (a fixture's
//! body, which it never parses) is no answer, so a query it does not decide
//! whole is held to what it does decide, and an order or a count it cannot
//! settle is not compared. A write the model does not follow is reported
//! as such before any query is asked of its store.

use std::{collections::BTreeSet, thread};

use io_pimdir::{
    client::{
        PimdirStore,
        search::{PimdirIndex, PimdirIndexRefreshOptions, PimdirSearchClock, PimdirSearchOptions},
    },
    search::{coverage::PimdirSearchTemporal, plan::PimdirSearchSort, query::PimdirQuery},
};
use pimdir_generator::{
    model::{Item, Model, State, Table},
    profile::Profile,
    query::{Answer, Clock, Corpus, Hit, Matched, Order, Temporal, date::Zone, grammar},
    spec::Spec,
    time::Instant,
};
use rusqlite::types::Value;

use crate::{
    common::{Generated, SEEDS},
    index::{BATCH, NOW, Write, tamed, writes},
    reader,
};

/// The query handles each step's queries are spread over.
const QUERY_THREADS: usize = 4;

/// The orders asked, each query in the first and one other in turn.
const ORDERS: [(Order, PimdirSearchSort); 5] = [
    (Order::NewestFirst, PimdirSearchSort::NewestFirst),
    (Order::OldestFirst, PimdirSearchSort::OldestFirst),
    (Order::Title, PimdirSearchSort::Title),
    (Order::Sender, PimdirSearchSort::Sender),
    (Order::Relevance, PimdirSearchSort::Relevance),
];

#[test]
fn every_grammar_form_answers_what_the_oracle_decides_on_generated_stores() {
    let differences = seeded(false);
    assert!(differences.is_empty(), "{differences:#?}");
}

#[test]
#[ignore = "every order and every page of every query, for a release build"]
fn every_grammar_form_in_every_order_answers_what_the_oracle_decides() {
    let differences = seeded(true);
    assert!(differences.is_empty(), "{differences:#?}");
}

/// The differences of every seed, run side by side, each named by its
/// seed.
fn seeded(every: bool) -> Vec<String> {
    thread::scope(|scope| {
        let runs: Vec<_> = SEEDS
            .iter()
            .map(|seed| scope.spawn(move || (*seed, differences(*seed, every))))
            .collect();
        runs.into_iter()
            .flat_map(|run| {
                let (seed, differences) = run.join().unwrap();
                differences
                    .into_iter()
                    .map(move |difference| format!("seed {seed}: {difference}"))
            })
            .collect()
    })
}

/// Every answer io-pimdir gives at `seed` otherwise than the oracle
/// decides, after each write of the run. `every` asks each query in every
/// order and walks each.
fn differences(seed: u64, every: bool) -> Vec<String> {
    let spec = Spec::load_default().unwrap();
    let model = Profile::Structure.generate(&spec, seed).unwrap();
    let generated = Generated::write(spec, tamed(model));
    let root = generated.root();
    let mut drained = generated.model.clone();
    drained.drain_all();

    let options = PimdirIndexRefreshOptions {
        batch: BATCH,
        now: Some(NOW),
        ..Default::default()
    };
    let mut index = PimdirIndex::open(&root).unwrap();
    let mut owner = PimdirStore::open(&root)
        .unwrap()
        .for_source("local")
        .unwrap();
    let mut model = generated.model.clone();

    let mut differences = Vec::new();
    for (step, write) in writes(&drained).into_iter().enumerate() {
        let label = write.label();
        assert!(write.apply(&mut owner).unwrap(), "{label}");
        follow(&mut model, &write);
        index.refresh(&options).unwrap();

        let (_, rows) = reader::compare(&generated.spec, &model, &root, |_, _, _, _| None);
        let rows: Vec<String> = rows.into_iter().map(|(_, row)| row).collect();
        if !rows.is_empty() {
            differences.push(format!(
                "step {step} ({label}): the model does not follow the write: {rows:#?}"
            ));
            continue;
        }

        let corpus = Corpus {
            model: model.clone(),
            properties: Vec::new(),
            missing: BTreeSet::new(),
            opaque: BTreeSet::new(),
            horizon: horizon(NOW),
            floating: Zone::Utc,
        };
        let queries: Vec<(usize, String)> = queries(&model).into_iter().enumerate().collect();
        let share = queries.len().div_ceil(QUERY_THREADS);
        let found: Vec<String> = thread::scope(|scope| {
            let runs: Vec<_> = queries
                .chunks(share)
                .map(|chunk| {
                    let (root, corpus) = (&root, &corpus);
                    scope.spawn(move || {
                        let index = PimdirIndex::open_query(root).unwrap();
                        let mut found = Vec::new();
                        for (at, query) in chunk {
                            let asked: Vec<usize> = match every {
                                true => (0..ORDERS.len()).collect(),
                                false => vec![0, 1 + at % (ORDERS.len() - 1)],
                            };
                            for order in asked {
                                let walks: Vec<usize> = match every {
                                    true => vec![1, 2, 3],
                                    false => vec![1 + (at + order) % 3],
                                };
                                for difference in ask(&index, corpus, query, ORDERS[order], &walks)
                                {
                                    found.push(format!("{query:?}: {difference}"));
                                }
                            }
                        }
                        found
                    })
                })
                .collect();
            runs.into_iter()
                .flat_map(|run| run.join().unwrap())
                .collect()
        });
        differences.extend(
            found
                .into_iter()
                .map(|difference| format!("step {step} ({label}): {difference}")),
        );
    }
    differences
}

/// The model through the owner's write, as the generator's upkeep reads
/// STORAGE §11 and §14.
fn follow(model: &mut Model, write: &Write) {
    match write {
        Write::Nothing => {}
        Write::Drain => model.drain_all(),
        Write::Purge { collection, seq } => {
            model.purge(collection, *seq);
        }
        Write::SortKey {
            collection,
            link_id,
        } => model.set_sort_key(collection, link_id, "2001-01-01T00:00:00Z"),
        Write::Name { collection } => model.set_collection_name(collection, "Renamed"),
        Write::Rename { collection } => {
            model.rename_collection(collection, &format!("{collection} renamed"))
        }
        Write::Release { collection } => {
            model.release_before(&[collection], None);
        }
        Write::Evict { collection, before } => {
            model.evict_before(collection, before);
        }
        Write::Erase { collection, seq } => {
            model.erase(collection, *seq);
        }
        Write::Delete { collection } => {
            model.delete_collection(collection);
        }
        Write::Collect => model.collect(),
    }
}

/// The horizon SEARCH §7 draws around `now` by default: a year back and
/// two forward, on UTC's calendar.
fn horizon(now: i64) -> (i64, i64) {
    let (year, month, day, hour, minute, second) = Instant(now).parts();
    let at = |year: i64| Instant::at(year, month, day, hour, minute, second).0;
    (at(year - 1), at(year + 2))
}

/// What io-pimdir answers `query` in `order` otherwise than the oracle
/// decides: the hits, their order, the pages walked at each of `walks`,
/// the count and the coverage.
fn ask(
    index: &PimdirIndex,
    corpus: &Corpus,
    query: &str,
    (order, sort): (Order, PimdirSearchSort),
    walks: &[usize],
) -> Vec<String> {
    let clock = Clock {
        now: NOW,
        zone: Zone::Utc,
    };
    let answer = match (corpus.ask(query, &clock, order), PimdirQuery::parse(query)) {
        (Ok(answer), Ok(parsed)) => (answer, parsed),
        (Err(refusal), Err(_)) => {
            return vec![format!(
                "both refuse it (the oracle {}): a template is malformed",
                refusal.code
            )];
        }
        (Ok(_), Err(err)) => return vec![format!("io-pimdir refuses it: {err}")],
        (Err(refusal), Ok(_)) => return vec![format!("the oracle refuses it ({})", refusal.code)],
    };
    let (answer, parsed) = answer;

    let options = PimdirSearchOptions {
        sort,
        limit: 10_000,
        after: None,
        clock: PimdirSearchClock {
            now: Some(NOW),
            zone: None,
        },
    };
    let whole = match index.search(&parsed, &options) {
        Ok(whole) => whole,
        Err(err) => return vec![format!("{order:?}: io-pimdir fails: {err}")],
    };
    let got: Vec<Hit> = whole
        .hits
        .iter()
        .map(|hit| Hit {
            account: hit.account.clone(),
            seq: hit.seq,
        })
        .collect();

    let mut differences = Vec::new();
    hits(&answer, &got, order, &mut differences);

    for hit in &whole.hits {
        let at = Hit {
            account: hit.account.clone(),
            seq: hit.seq,
        };
        let Some(oracle) = answer.matched.get(&at) else {
            continue;
        };
        let held = hit.matched.map(|matched| matched.as_str());
        let oracle = oracle.map(|matched| match matched {
            Matched::Body => "body",
            Matched::Summary => "summary",
        });
        if held != oracle {
            differences.push(format!("{at:?} matched {held:?}, the oracle {oracle:?}"));
        }
    }

    for limit in walks {
        let mut walked = Vec::new();
        let mut page = PimdirSearchOptions {
            limit: *limit,
            ..options.clone()
        };
        for _ in 0..=got.len() {
            let answer = index.search(&parsed, &page).unwrap();
            walked.extend(answer.hits.iter().map(|hit| Hit {
                account: hit.account.clone(),
                seq: hit.seq,
            }));
            match answer.next {
                Some(next) => page.after = Some(next),
                None => break,
            }
        }
        if walked != got {
            differences.push(format!(
                "{order:?}: walked {limit} by {limit}: {walked:?}, whole {got:?}"
            ));
        }
    }

    let count = index.count(&parsed, &options.clock).unwrap();
    let (yes, unknown) = answer.split();
    let counted = count.count as usize;
    if counted < yes.len() || counted > yes.len() + unknown.len() || counted != got.len() {
        differences.push(format!(
            "count {counted}, {} hits listed, the oracle {} to {}",
            got.len(),
            yes.len(),
            yes.len() + unknown.len()
        ));
    }
    if count.coverage != whole.coverage {
        differences.push(String::from("a count's coverage is not the search's"));
    }

    coverage(&answer, &whole.coverage, &mut differences);
    differences
}

fn hits(answer: &Answer, got: &[Hit], order: Order, differences: &mut Vec<String>) {
    let held: BTreeSet<&Hit> = got.iter().collect();
    let (yes, unknown) = answer.split();
    let lost: Vec<&Hit> = yes.iter().filter(|hit| !held.contains(hit)).collect();
    let added: Vec<&&Hit> = held
        .iter()
        .filter(|hit| !yes.contains(hit) && !unknown.contains(hit))
        .collect();
    if !lost.is_empty() || !added.is_empty() {
        differences.push(format!("hits: io-pimdir lacks {lost:?} and adds {added:?}"));
        return;
    }
    if let Some(ordered) = &answer.order
        && ordered != got
    {
        differences.push(format!("{order:?}: order {got:?}, the oracle {ordered:?}"));
    }
}

fn coverage(
    answer: &Answer,
    got: &io_pimdir::search::coverage::PimdirSearchCoverage,
    differences: &mut Vec<String>,
) {
    let oracle = &answer.coverage;
    if got.local.unindexed != 0 || got.local.behind != 0 {
        differences.push(format!("a lag after a refresh: {:?}", got.local));
    }
    if let Some(kinds) = &oracle.kinds {
        let held: Vec<(String, [u64; 4])> = got
            .local
            .kinds
            .iter()
            .filter(|(_, counts)| !counts.is_empty())
            .map(|(kind, counts)| {
                let counts = [
                    counts.bodiless,
                    counts.missing,
                    counts.unparseable,
                    counts.encrypted,
                ];
                (kind.clone(), counts)
            })
            .collect();
        let want: Vec<(String, [u64; 4])> = kinds
            .iter()
            .map(|(kind, counts)| {
                let counts = [
                    counts.bodiless,
                    counts.missing,
                    counts.unparseable,
                    counts.encrypted,
                ];
                (kind.clone(), counts)
            })
            .collect();
        if held != want {
            differences.push(format!("coverage {held:?}, the oracle {want:?}"));
        }
    }
    if let Some(temporal) = &oracle.temporal {
        let held = match &got.temporal {
            PimdirSearchTemporal::Full => Temporal::Full,
            PimdirSearchTemporal::Bounded {
                since,
                until,
                filling,
            } => Temporal::Bounded {
                since: since.clone(),
                until: until.clone(),
                filling: *filling,
            },
        };
        if held != *temporal {
            differences.push(format!("temporal {held:?}, the oracle {temporal:?}"));
        }
    }
    let mut part = |name: &str, held: String, oracle: Option<String>| {
        if let Some(oracle) = oracle
            && oracle != held
        {
            differences.push(format!("{name} {held}, the oracle {oracle}"));
        }
    };
    part(
        "horizon_left",
        got.horizon_left.to_string(),
        oracle.horizon_left.map(|left| left.to_string()),
    );
    part(
        "cut",
        got.cut.to_string(),
        oracle.cut.map(|cut| cut.to_string()),
    );
    part(
        "zone_differs",
        got.zone_differs.to_string(),
        oracle.zone_differs.map(|differs| differs.to_string()),
    );
    part(
        "complete",
        got.is_complete().to_string(),
        oracle.complete.map(|complete| complete.to_string()),
    );
}

/// One query per form of SEARCH §8.3's grammar, its values drawn from what
/// `model` holds live, each text form asked over the whole store and over
/// each generated account alone, where the fixtures of the lab account
/// leave no answer the oracle cannot place.
fn queries(model: &Model) -> Vec<String> {
    let live: Vec<&Item> = model
        .items
        .iter()
        .filter(|item| item.state == State::Live)
        .collect();
    let rendered = |item: &&&Item| {
        item.object
            .as_ref()
            .is_some_and(|object| model.words.contains_key(object))
    };
    let of = |table: Table| -> Vec<&Item> {
        live.iter()
            .filter(rendered)
            .filter(|item| item.summary.table == table)
            .copied()
            .collect()
    };
    let mails = of(Table::Mail);
    let cards = of(Table::Contact);
    let events = of(Table::Event);
    let tasks = of(Table::Task);

    let text = |value: Value| match value {
        Value::Text(text) => text,
        _ => String::new(),
    };
    let words = |item: &Item, field: &str| -> Vec<String> {
        item.object
            .as_ref()
            .and_then(|object| model.words.get(object))
            .and_then(|words| words.fields.get(field))
            .map(|values| {
                values
                    .iter()
                    .flat_map(|value| value.split_whitespace().map(str::to_lowercase))
                    .collect()
            })
            .unwrap_or_default()
    };

    let mut forms: Vec<String> = Vec::new();
    let mut texts: Vec<String> = Vec::new();

    if let Some(mail) = mails.get(mails.len() / 2) {
        let body = words(mail, "body");
        let subject = words(mail, "title");
        let from = mail.addresses.iter().find(|address| address.role == "from");
        let to = mail.addresses.iter().find(|address| address.role == "to");
        if let [first, second, ..] = body.as_slice() {
            texts.push(first.clone());
            texts.push(format!("\"{first} {second}\""));
            texts.push(format!("{first} or {second}"));
            texts.push(format!("{first} not {second}"));
            texts.push(format!("body:{second}"));
            texts.push(format!("-\"{first} {second}\""));
            texts.push(format!("\"{first}\"{second}"));
            if first.len() >= 3 {
                texts.push(format!("{}*", &first[..3]));
            }
        }
        if let Some(word) = subject.first() {
            if word.chars().count() >= 3 {
                let prefix: String = word.chars().take(3).collect();
                texts.push(format!("subject:{prefix}*"));
            }
            texts.push(format!("subject:{word}"));
            texts.push(format!("title:{word}"));
            texts.push(format!("summary:\"{}\"", subject.join(" ")));
        }
        if let Some(from) = from {
            forms.push(format!("from:{}", from.address));
            let domain = from
                .address
                .rsplit_once('@')
                .map(|(_, domain)| domain)
                .unwrap_or_default();
            forms.push(format!("from:@{domain}"));
            forms.push(format!("with:{}", from.address));
            forms.push(format!("with:@{domain}"));
            if let Some(name) = &from.name {
                let first = name
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_lowercase();
                texts.push(format!("from:{first}"));
                texts.push(format!("with:{first}"));
                texts.push(format!("\"{}\"", name.to_lowercase()));
            }
        }
        if let Some(to) = to {
            forms.push(format!("to:{}", to.address));
            forms.push(format!("cc:{}", to.address));
            forms.push(format!("bcc:{}", to.address));
            if let Some(name) = &to.name {
                let last = name
                    .split_whitespace()
                    .last()
                    .unwrap_or_default()
                    .to_lowercase();
                texts.push(format!("to:{last}"));
                texts.push(format!("cc:{last}"));
            }
        }
        forms.push(format!("mid:{}", text(mail.summary.get("message_id"))));
        forms.push(format!("mid:<{}>", text(mail.summary.get("message_id"))));
        forms.push(format!("id:{}", mail.link_id));
        forms.push(format!("id:{}", mail.seq));
        if mail.sort_key.len() >= 10 {
            let day = &mail.sort_key[..10];
            forms.push(format!("date:{day}"));
            forms.push(format!("when:{}", &mail.sort_key[..7]));
            forms.push(format!(
                "kind:mail date:{}..{}",
                &mail.sort_key[..7],
                &mail.sort_key[..7]
            ));
            forms.push(format!("date:{day}T{}", &mail.sort_key[11..16]));
            forms.push(format!("date:{day}T{}Z", &mail.sort_key[11..19]));
            forms.push(format!("date:{day}T{}+02:00", &mail.sort_key[11..16]));
            forms.push(format!("date:{day}T{}-05:30", &mail.sort_key[11..16]));
            forms.push(format!("kind:mail date:..{day}"));
            forms.push(format!("kind:mail date:{day}.."));
        }
    }
    texts.push(String::from("attachment:report"));
    texts.push(String::from("mimetype:octet"));
    texts.push(String::from("location:room"));
    texts.push(String::from("\"\" or budget"));

    if let Some(card) = cards.first() {
        let name = text(card.summary.get("fn"));
        if let Some(first) = name.split_whitespace().next() {
            texts.push(format!("person:{}", first.to_lowercase()));
            texts.push(format!("kind:contact {}", first.to_lowercase()));
        }
        if let Some(email) = card.addresses.first() {
            forms.push(format!("email:{}", email.address));
            forms.push(format!("with:{}", email.address));
        }
        forms.push(format!("linked:{}", card.seq));
    }
    if let Some(event) = events.iter().find(|event| !event.addresses.is_empty()) {
        let attendee = event
            .addresses
            .iter()
            .find(|address| address.role == "attendee");
        if let Some(attendee) = attendee {
            forms.push(format!("attendee:{}", attendee.address));
            if let Some(name) = &attendee.name {
                texts.push(format!(
                    "attendee:{}",
                    name.split_whitespace()
                        .next()
                        .unwrap_or_default()
                        .to_lowercase()
                ));
            }
        }
        if let Some(organizer) = event
            .addresses
            .iter()
            .find(|address| address.role == "organizer")
        {
            forms.push(format!("organizer:{}", organizer.address));
        }
    }
    for event in &events {
        if event.sort_key.len() >= 10 {
            forms.push(format!("kind:event date:{}", &event.sort_key[..10]));
        }
    }
    if let Some(task) = tasks.first() {
        if let Some(category) = task.summary.categories.last() {
            forms.push(format!("category:{category}"));
            forms.push(format!("category:\"{}\"", category.to_uppercase()));
        }
        if task.sort_key.len() >= 10 {
            forms.push(format!("kind:task date:{}", &task.sort_key[..10]));
        }
    }

    let replied = live
        .iter()
        .find_map(|item| item.summary.in_reply_to.first().cloned());
    if let Some(parent) = replied {
        forms.push(format!("thread:{parent}"));
        forms.push(format!("thread:{parent} account:work"));
    }
    forms.push(String::from(
        "thread:only-from-sent@pimdir.test account:work",
    ));
    if let Some(mail) = mails.first() {
        forms.push(format!("thread:{} account:home", mail.link_id));
    }
    for reference in model.references.iter().take(3) {
        forms.push(format!("linked:{}", reference.from_link_id));
        forms.push(format!(
            "linked:{}:{}",
            reference.role, reference.to_link_id
        ));
    }

    for flag in ["\\Seen", "\\Flagged", "$Important", "\\seen", "$label1"] {
        forms.push(format!("tag:{flag}"));
    }
    forms.push(String::from("flag:\\Flagged kind:mail"));
    for value in [
        "unread",
        "read",
        "flagged",
        "answered",
        "draft",
        "encrypted",
    ] {
        forms.push(format!("is:{value}"));
    }
    for value in [
        "body",
        "attachment",
        "phone",
        "address",
        "birthday",
        "photo",
    ] {
        forms.push(format!("has:{value}"));
    }
    for value in ["mail", "contact", "event", "task", "journal", "file"] {
        forms.push(format!("kind:{value}"));
    }
    for value in ["needs-action", "in-process", "completed", "cancelled"] {
        forms.push(format!("status:{value}"));
    }
    for value in ["high", "medium", "low", "none"] {
        forms.push(format!("priority:{value}"));
    }
    forms.push(String::from("category:inbox kind:task"));
    for (id, collection) in &model.collections {
        if collection.kind.is_some() && !id.contains('\u{1}') {
            forms.push(format!("collection:\"{id}\""));
        }
    }
    let accounts: BTreeSet<&String> = model
        .collections
        .values()
        .filter_map(|collection| collection.account.as_ref())
        .collect();
    for account in &accounts {
        forms.push(format!("account:{account}"));
    }
    forms.push(String::from("folder:home/INBOX"));
    forms.push(String::from(
        "collection:work/INBOX collection:\"work/Archive\"",
    ));
    forms.push(String::from("label:receipts"));
    forms.push(String::from("property:pimdir/label=Receipts"));
    forms.push(String::from("changed:0.."));
    forms.push(String::from("changed:..0"));
    forms.push(String::from("changed:1000000000.."));
    forms.push(String::from("changed:5"));
    forms.push(String::from("changed:3..40"));
    forms.push(String::from("date:90min"));
    forms.push(String::from("date:12h"));
    forms.push(String::from("kind:mail date:2w"));
    forms.push(String::from("date:3mo..1mo"));
    forms.push(String::from("date:1y"));
    forms.push(String::from("date:2026"));
    forms.push(String::from("date:2026-03..2026-05"));
    forms.push(String::from("kind:event date:2026-06"));
    forms.push(String::from("kind:journal date:2026"));
    forms.push(String::from("date:..2026-06"));
    forms.push(String::from("date:30d"));
    forms.push(String::from("date:today"));
    forms.push(String::from("date:yesterday..today"));
    forms.push(String::from("kind:contact date:2026"));
    forms.push(String::from("kind:mail or kind:contact"));
    forms.push(String::from("not kind:mail"));
    forms.push(String::from("-(kind:mail or kind:file) account:work"));
    forms.push(String::from("(account:work) is:unread"));
    forms.push(String::from("((account:home)) (is:read or is:flagged)"));
    forms.push(String::from("kind:mail and is:flagged"));
    forms.push(String::from("--kind:mail"));
    forms.push(String::from("kind:mail -is:read OR kind:task"));
    forms.push(String::from("NOT kind:mail"));
    forms.push(String::from("kind:mail AND is:flagged"));
    forms.push(String::from("(kind:mail)is:unread"));
    forms.push(String::from("kind:mail\tis:unread"));
    forms.push(String::from("kind:contact\r\nor kind:task"));

    for text in &texts {
        forms.push(text.clone());
        for account in ["work", "home"] {
            forms.push(format!("account:{account} ({text})"));
        }
    }

    for form in &forms {
        assert!(
            grammar::parse(form).is_ok(),
            "{form:?}: a template the grammar refuses"
        );
    }
    forms
}
