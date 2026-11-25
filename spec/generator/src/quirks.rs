//! # The quirks profile
//!
//! Histories over remotes behaving as the servers the family met do
//! (README.md, "Profiles"): handles renumbered, listings unsorted, repeating
//! a member or dropping one, a `Message-ID` reused across mailboxes or
//! absent, revisions that stay put over an edit, capped and narrowed
//! listings with and without `enumerated`, received dates skewed from the
//! `Date`, relocations stating the handle they made in `to` or not, bodies
//! rewritten on the server, and collections empty or large. The seed draws
//! the sizes, the handles and which variant each quirk takes, so a campaign
//! over many seeds meets each in many shapes.
//!
//! Each quirk is a scripted remote behaviour the engine model already runs,
//! or one SYNC states as equal to a behaviour it runs: a page's members
//! unsorted and one of them listed twice, its second entry stating other
//! flags, is read as the page sorted with the first entry kept (SYNC §4),
//! so the model runs the clean page and the run serves the jumbled one; a
//! relocation stating the handle its member has in `to` lands the create on
//! it in the write recording the outcome (SYNC §4), which [`land`] applies
//! over the run the model made stating none.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    body::{Card, Mail},
    engine::{Answer, Options, Push, Run},
    history::{History, Step},
    model::{Collection, Kind, Model, Presentation, Scope, State, synced_by},
    profile::Draw,
    queue::Action,
    remote::{Page, Remote},
    rng::Rng,
    time::Instant,
};

/// The PCG32 stream the quirks draw their shapes from, apart from the
/// surface [`Draw`] renders.
const STREAM: u64 = 0x7175_6972_6b73;

/// Every history of the quirks profile at `seed`, by name.
pub fn histories(seed: u64) -> Vec<(&'static str, History)> {
    vec![
        ("renumbered", renumbered(seed)),
        ("listings", listings(seed)),
        ("message-ids", message_ids(seed)),
        ("revisions", revisions(seed)),
        ("narrowed", narrowed(seed)),
        ("relocations", relocations(seed)),
        ("rewrites", rewrites(seed)),
        ("extremes", extremes(seed)),
    ]
}

/// The steps of a quirk history as they are recorded, each run's pages
/// jumbled as the source serves them.
struct Recorder {
    steps: Vec<Step>,
    rng: Rng,
}

impl Recorder {
    fn new(label: &str, model: &Model, rng: Rng) -> Self {
        let step = Step {
            label: String::from(label),
            run: None,
            model: model.clone(),
        };
        Self {
            steps: vec![step],
            rng,
        }
    }

    fn record(&mut self, label: String, run: Option<Run>, model: &Model) {
        self.steps.push(Step {
            label,
            run,
            model: model.clone(),
        });
    }

    /// Records a run whose pages the source serves unsorted, one member
    /// listed again after the page's others with other flags, which the
    /// engine reads as the page sorted and the first entry kept (SYNC §4).
    fn run(&mut self, label: String, mut run: Run, model: &Model) {
        for page in &mut run.pages {
            jumble(page, &mut self.rng);
        }
        self.record(label, Some(run), model);
    }

    fn history(self) -> History {
        History { steps: self.steps }
    }
}

/// A page's members in reverse handle order, and one of them listed twice
/// when the seed says so, its second entry stating a flag the first does
/// not.
fn jumble(page: &mut Page, rng: &mut Rng) {
    page.items.reverse();
    if page.items.is_empty() || rng.below(3) == 0 {
        return;
    }
    let at = rng.below(page.items.len() as u32) as usize;
    let mut again = page.items[at].clone();
    again.flags.insert(String::from("$Repeated"));
    page.items.push(again);
}

fn set(flags: &[&str]) -> BTreeSet<String> {
    flags.iter().map(|flag| String::from(*flag)).collect()
}

/// A collection of the work account syncing through `source`.
fn declare(model: &mut Model, id: &str, kind: Kind, source: &str) {
    model.declare(Collection {
        id: String::from(id),
        account: Some(String::from("work")),
        kind: Some(kind),
        name: String::from(id.trim_start_matches("work/")),
        role: None,
        sources: synced_by(&[source]),
        conflict: String::from("manual"),
        generation: 1,
        parent: None,
        presentation: Presentation::default(),
    });
}

/// The `seq`, flags and body of the item `handle` binds in `collection`.
fn bound(model: &Model, collection: &str, handle: &str) -> Option<(i64, BTreeSet<String>)> {
    model
        .items
        .iter()
        .find(|item| {
            item.collection == collection
                && item.state == State::Live
                && item.bindings.iter().any(|binding| binding.handle == handle)
        })
        .map(|item| (item.seq, item.flags.clone().unwrap_or_default()))
}

/// The handles a remote holds, in byte order.
fn handles(remote: &Remote) -> Vec<String> {
    remote.members.keys().cloned().collect()
}

/// A random flag set of the three an inbox shows.
fn flags(rng: &mut Rng) -> BTreeSet<String> {
    match rng.below(3) {
        0 => BTreeSet::new(),
        1 => set(&["\\Seen"]),
        _ => set(&["\\Seen", "$label1"]),
    }
}

/// A scope from the first of `month` of 2026 on.
fn from(month: u32) -> Options {
    Options {
        scope: Scope {
            since: Some(Instant::at(2026, month, 1, 0, 0, 0)),
            until: None,
        },
        accept_absences: false,
    }
}

fn accepting() -> Options {
    Options {
        accept_absences: true,
        ..Options::default()
    }
}

/// Handles renumbered under a collection: a mailbox's `UIDVALIDITY` bumped
/// into a range overlapping its old handles or not, one member lost on the
/// way or not, and an address book renumbered over a card the server
/// edited, each rekeyed (SYNC §8) and synced after.
fn renumbered(seed: u64) -> History {
    let mut draw = Draw::new(seed);
    let mut rng = Rng::new(seed, STREAM);
    let mut model = Model::new();
    declare(&mut model, "work/INBOX", Kind::Mail, "work-imap");
    declare(&mut model, "work/contacts", Kind::Contact, "work-carddav");
    let mut inbox = Remote::new("work-imap", "work/INBOX");
    let mut book = Remote::dav("work-carddav", "work/contacts");
    for _ in 0..6 + rng.below(15) {
        inbox.deliver(draw.message(false).file(), flags(&mut rng));
    }
    let mut cards = BTreeMap::new();
    for _ in 0..3 + rng.below(5) {
        let card = draw.card();
        let (handle, _) = book.deliver(card.file(), BTreeSet::new());
        cards.insert(handle, card);
    }
    let page = 2 + rng.below(4) as usize;
    let mut steps = Recorder::new("a mailbox and an address book", &model, rng);

    let run = model.sync(&mut inbox, page);
    steps.run(String::from("the mailbox's first round"), run, &model);
    let fetched: Vec<String> = handles(&inbox).into_iter().take(3).collect();
    let run = model.upgrade(&inbox, &fetched);
    steps.record(String::from("three bodies fetched"), Some(run), &model);

    if steps.rng.below(2) == 0 {
        let lost = handles(&inbox)[0].clone();
        inbox.forget(&lost);
    }
    let start = match steps.rng.below(3) {
        0 => 1,
        1 => inbox.next,
        _ => 1_000 + u64::from(steps.rng.below(9_000)),
    };
    inbox.renumber(start);
    let mut run = model.rekey(&inbox, &Options::default());
    let parked = run.parked.is_some();
    for page in &mut run.pages {
        jumble(page, &mut steps.rng);
    }
    steps.record(
        format!("the mailbox renumbered from {start}"),
        Some(run),
        &model,
    );
    if parked {
        let run = model.rekey(&inbox, &accepting());
        steps.record(String::from("the rekey accepted"), Some(run), &model);
    }
    let listed = handles(&inbox);
    if let Some(handle) = listed.get(steps.rng.below(listed.len() as u32) as usize) {
        let mut flagged = inbox.members[handle].flags.clone();
        flagged.insert(String::from("\\Flagged"));
        inbox.set_flags(handle, flagged);
    }
    let run = model.sync(&mut inbox, page);
    steps.run(String::from("a delta after the rekey"), run, &model);

    let run = model.sync(&mut book, page);
    steps.run(String::from("the address book's first round"), run, &model);
    let edited = handles(&book)[0].clone();
    let renamed = Card {
        name: String::from("Edited Before Renumbering"),
        ..cards[&edited].clone()
    };
    book.edit(&edited, renamed.file());
    book.renumber(10 + u64::from(steps.rng.below(90)));
    let run = model.rekey(&book, &Options::default());
    let parked = run.parked.is_some();
    steps.record(
        String::from("the address book renumbered"),
        Some(run),
        &model,
    );
    if parked {
        let run = model.rekey(&book, &accepting());
        steps.record(String::from("its rekey accepted"), Some(run), &model);
    }
    let run = model.sync(&mut book, page);
    steps.run(String::from("a delta over the address book"), run, &model);

    steps.history()
}

/// Listings dropping members: a delta that never lists them, a round
/// finding them absent, and one of them listed again under its handle.
fn listings(seed: u64) -> History {
    let mut draw = Draw::new(seed);
    let mut rng = Rng::new(seed, STREAM + 1);
    let mut model = Model::new();
    declare(&mut model, "work/INBOX", Kind::Mail, "work-imap");
    let mut inbox = Remote::new("work-imap", "work/INBOX");
    let count = 8 + rng.below(12);
    for _ in 0..count {
        inbox.deliver(draw.message(false).file(), flags(&mut rng));
    }
    let page = 2 + rng.below(5) as usize;
    let mut steps = Recorder::new("a mailbox", &model, rng);

    let run = model.sync(&mut inbox, page);
    steps.run(String::from("a first round"), run, &model);
    let fetched: Vec<String> = handles(&inbox)
        .into_iter()
        .filter(|_| steps.rng.below(2) == 0)
        .collect();
    let run = model.upgrade(&inbox, &fetched);
    steps.record(String::from("some bodies fetched"), Some(run), &model);

    let mut dropped = Vec::new();
    for _ in 0..1 + steps.rng.below(count / 3) {
        let held = handles(&inbox);
        let handle = held[steps.rng.below(held.len() as u32) as usize].clone();
        let member = inbox.members[&handle].clone();
        inbox.forget(&handle);
        dropped.push((handle, member));
    }
    let run = model.sync_with(&mut inbox, page, &from(12));
    steps.run(
        format!("a delta listing none of {} dropped", dropped.len()),
        run,
        &model,
    );
    let run = model.sync_with(&mut inbox, page, &Options::default());
    steps.run(String::from("a round finding them absent"), run, &model);

    let (handle, mut member) = dropped.swap_remove(0);
    inbox.modseq += 1;
    member.changed = inbox.modseq;
    inbox.members.insert(handle.clone(), member);
    let run = model.sync(&mut inbox, page);
    steps.run(format!("member {handle} listed again"), run, &model);
    let run = model.sync(&mut inbox, page);
    steps.run(String::from("a delta with nothing to do"), run, &model);

    steps.history()
}

/// `Message-ID`s reused and absent: a message filed in two mailboxes,
/// another message reusing its `Message-ID` in the second, messages with
/// none, two of them alike in subject, date and sender.
fn message_ids(seed: u64) -> History {
    let mut draw = Draw::new(seed);
    let mut rng = Rng::new(seed, STREAM + 2);
    let mut model = Model::new();
    declare(&mut model, "work/INBOX", Kind::Mail, "work-imap");
    declare(&mut model, "work/Archive", Kind::Mail, "work-imap");
    let mut remotes = BTreeMap::from([
        (
            String::from("work/INBOX"),
            Remote::new("work-imap", "work/INBOX"),
        ),
        (
            String::from("work/Archive"),
            Remote::new("work-imap", "work/Archive"),
        ),
    ]);

    let shared = draw.message(false);
    let reusing = Mail {
        message_id: shared.message_id.clone(),
        ..draw.message(false)
    };
    let bare = Mail {
        message_id: None,
        ..draw.message(false)
    };
    let twin = Mail {
        text: String::from("another text under the same subject, date and sender"),
        ..bare.clone()
    };
    let inbox = remotes.get_mut("work/INBOX").expect("an inbox");
    inbox.deliver(shared.file(), flags(&mut rng));
    inbox.deliver(bare.file(), flags(&mut rng));
    if rng.below(2) == 0 {
        inbox.deliver(twin.file(), flags(&mut rng));
    }
    for _ in 0..rng.below(4) {
        let message = match rng.below(2) {
            0 => draw.message(false),
            _ => Mail {
                message_id: None,
                ..draw.message(false)
            },
        };
        inbox.deliver(message.file(), flags(&mut rng));
    }
    let archive = remotes.get_mut("work/Archive").expect("an archive");
    archive.deliver(shared.file(), set(&["\\Seen"]));
    archive.deliver(reusing.file(), BTreeSet::new());
    if rng.below(2) == 0 {
        archive.deliver(twin.file(), BTreeSet::new());
    }
    let page = 2 + rng.below(3) as usize;
    let mut steps = Recorder::new("two mailboxes", &model, rng);

    for id in ["work/INBOX", "work/Archive"] {
        let run = model.sync_in(&mut remotes, id, page, &Options::default());
        steps.run(format!("{id}: a first round"), run, &model);
    }
    for id in ["work/INBOX", "work/Archive"] {
        let fetched = handles(&remotes[id]);
        let run = model.upgrade(&remotes[id], &fetched);
        steps.record(format!("{id}: every body fetched"), Some(run), &model);
    }
    let archive = remotes.get_mut("work/Archive").expect("an archive");
    archive.expunge("1");
    let run = model.sync_in(&mut remotes, "work/Archive", page, &Options::default());
    steps.run(String::from("the shared copy expunged"), run, &model);
    for id in ["work/INBOX", "work/Archive"] {
        let run = model.sync_in(&mut remotes, id, page, &Options::default());
        steps.run(format!("{id}: nothing to do"), run, &model);
    }

    steps.history()
}

/// Revisions that do not move: a server edit keeping its ETag, beside one
/// moving it, and a pushed edit the server answers at the ETag it held.
fn revisions(seed: u64) -> History {
    let mut draw = Draw::new(seed);
    let mut rng = Rng::new(seed, STREAM + 3);
    let mut model = Model::new();
    let id = "work/contacts";
    declare(&mut model, id, Kind::Contact, "work-carddav");
    let mut book = Remote::dav("work-carddav", id);
    let mut cards = BTreeMap::new();
    for _ in 0..3 + rng.below(6) {
        let card = draw.card();
        let (handle, _) = book.deliver(card.file(), BTreeSet::new());
        cards.insert(handle, card);
    }
    let page = 2 + rng.below(3) as usize;
    let mut steps = Recorder::new("an address book", &model, rng);
    let renamed = |handle: &str, name: &str| Card {
        name: String::from(name),
        ..cards[handle].clone()
    };

    let run = model.sync(&mut book, page);
    steps.run(String::from("a first round"), run, &model);

    let held = handles(&book);
    let stuck = held[0].clone();
    let revision = book.members[&stuck].revision.clone();
    book.edit(&stuck, renamed(&stuck, "Edited Under One Etag").file());
    if let Some(member) = book.members.get_mut(&stuck) {
        member.revision = revision;
    }
    let moved = held[1].clone();
    book.edit(&moved, renamed(&moved, "Edited Under Another").file());
    let run = model.sync(&mut book, page);
    steps.run(
        String::from("two server edits, one keeping its etag"),
        run,
        &model,
    );

    let pushed = held[2].clone();
    let index = model
        .items
        .iter()
        .position(|item| item.collection == id && item.bindings.iter().any(|b| b.handle == pushed))
        .expect("a bound card");
    let (seq, object) = (
        model.items[index].seq,
        model.items[index].object.clone().expect("a held card"),
    );
    let row = model.enqueue(
        "cardamum",
        id,
        Action::Update {
            seq,
            filed: renamed(&pushed, "Edited Here").file(),
            if_match: Some(object),
            resolves: None,
        },
    );
    model.drain(row);
    steps.record(String::from("a card edited here"), None, &model);

    let held_revision = book.members[&pushed].revision.clone();
    let mut run = model.sync(&mut book, page);
    let still = steps.rng.below(2) == 0;
    if still {
        for outcome in &mut run.outcomes {
            if outcome.handle == pushed && outcome.answer == Answer::Accepted {
                outcome.revision = held_revision.clone();
            }
        }
        if let Some(member) = book.members.get_mut(&pushed) {
            member.revision = held_revision.clone();
        }
        let binding = model.items[index]
            .bindings
            .iter_mut()
            .find(|binding| binding.handle == pushed)
            .expect("the pushed binding");
        binding.base_revision = held_revision;
    }
    let label = match still {
        true => "the edit pushed, answered at the etag held",
        false => "the edit pushed",
    };
    steps.run(String::from(label), run, &model);
    let run = model.sync(&mut book, page);
    steps.run(String::from("a delta with nothing to do"), run, &model);

    steps.history()
}

/// Listings a date filter narrows, received dates skewed from the `Date`
/// they carry, an enumeration handed or not, and a capped mailbox.
fn narrowed(seed: u64) -> History {
    let mut draw = Draw::new(seed);
    let mut rng = Rng::new(seed, STREAM + 4);
    let mut model = Model::new();
    declare(&mut model, "work/INBOX", Kind::Mail, "work-graph");
    declare(&mut model, "work/Sent", Kind::Mail, "sent-imap");
    let mut inbox = Remote::new("work-graph", "work/INBOX");
    inbox.narrows = true;
    inbox.enumerates = rng.below(2) == 0;
    let day = 86_400;
    for _ in 0..6 + rng.below(10) {
        let message = draw.message(false);
        let skew = match rng.below(5) {
            0 => -90 * day,
            1 => -3 * day,
            2 => 3 * day,
            3 => 120 * day,
            _ => 0,
        };
        let filed = match (rng.below(6), message.date) {
            (0, _) | (_, None) => Mail {
                date: None,
                ..message
            }
            .file(),
            (_, Some(date)) => {
                let filed = message.file();
                inbox.deliver_at(filed, flags(&mut rng), Instant(date.0 + skew));
                continue;
            }
        };
        inbox.clock = Instant(Instant::at(2026, 9, 1, 0, 0, 0).0 + skew);
        inbox.deliver(filed, flags(&mut rng));
    }
    let mut sent = Remote::new("sent-imap", "work/Sent");
    let count = 4 + rng.below(8);
    for _ in 0..count {
        sent.deliver(draw.message(false).file(), set(&["\\Seen"]));
    }
    let page = 2 + rng.below(4) as usize;
    let mut steps = Recorder::new("an inbox narrowing and a capped mailbox", &model, rng);

    let run = model.sync(&mut inbox, page);
    steps.run(String::from("the inbox's first round"), run, &model);
    let month = 7 + steps.rng.below(5);
    let run = model.sync_with(&mut inbox, page, &from(month));
    steps.run(format!("a delta narrowing to month {month}"), run, &model);
    for _ in 0..1 + steps.rng.below(3) {
        let held = handles(&inbox);
        if held.len() > 1 {
            let handle = held[steps.rng.below(held.len() as u32) as usize].clone();
            inbox.forget(&handle);
        }
    }
    let run = model.sync_with(&mut inbox, page, &from(month - 2));
    let enumerating = match inbox.enumerates {
        true => "enumerating",
        false => "handing no enumeration",
    };
    steps.run(format!("a narrowed round {enumerating}"), run, &model);
    inbox.enumerates = !inbox.enumerates;
    let run = model.sync_with(&mut inbox, page, &from(month - 4));
    steps.run(String::from("a narrowed round again"), run, &model);

    let run = model.sync(&mut sent, page);
    steps.run(String::from("the sent mailbox's first round"), run, &model);
    let run = model.sync_with(&mut sent, page, &from(12));
    steps.run(String::from("a delta narrowing it"), run, &model);
    sent.cap = Some(1 + steps.rng.below(count - 1) as usize);
    for handle in handles(&sent).into_iter().take(2) {
        if steps.rng.below(2) == 0 {
            sent.forget(&handle);
        }
    }
    let run = model.sync(&mut sent, page);
    steps.run(String::from("a capped round"), run, &model);

    steps.history()
}

/// Relocations: moves out of an inbox whose server states the handle each
/// relocated member has in the archive, or does not, and a copy.
fn relocations(seed: u64) -> History {
    let mut draw = Draw::new(seed);
    let mut rng = Rng::new(seed, STREAM + 5);
    let mut model = Model::new();
    declare(&mut model, "work/INBOX", Kind::Mail, "work-imap");
    declare(&mut model, "work/Archive", Kind::Mail, "work-imap");
    let mut remotes = BTreeMap::from([
        (
            String::from("work/INBOX"),
            Remote::new("work-imap", "work/INBOX"),
        ),
        (
            String::from("work/Archive"),
            Remote::new("work-imap", "work/Archive"),
        ),
    ]);
    let count = 4 + rng.below(5);
    for _ in 0..count {
        let inbox = remotes.get_mut("work/INBOX").expect("an inbox");
        inbox.deliver(draw.message(false).file(), flags(&mut rng));
    }
    let archive = remotes.get_mut("work/Archive").expect("an archive");
    archive.deliver(draw.message(false).file(), set(&["\\Seen"]));
    let page = 2 + rng.below(3) as usize;
    let mut steps = Recorder::new("two mailboxes", &model, rng);

    for id in ["work/INBOX", "work/Archive"] {
        let run = model.sync_in(&mut remotes, id, page, &Options::default());
        steps.run(format!("{id}: a first round"), run, &model);
    }
    let fetched = handles(&remotes["work/INBOX"]);
    let run = model.upgrade(&remotes["work/INBOX"], &fetched);
    steps.record(
        String::from("the inbox's bodies fetched"),
        Some(run),
        &model,
    );

    let moved: Vec<String> = handles(&remotes["work/INBOX"])
        .into_iter()
        .filter(|_| steps.rng.below(2) == 0)
        .collect();
    for (at, handle) in moved.iter().enumerate() {
        let (seq, _) = bound(&model, "work/INBOX", handle).expect("a bound message");
        let action = match at == 0 && moved.len() > 1 {
            true => Action::Copy {
                seq,
                to: String::from("work/Archive"),
            },
            false => Action::Move {
                seq,
                to: String::from("work/Archive"),
            },
        };
        let row = model.enqueue("himalaya", "work/INBOX", action);
        model.drain(row);
    }
    steps.record(
        format!("{} moved or copied, drained", moved.len()),
        None,
        &model,
    );

    let mut run = model.sync_in(&mut remotes, "work/INBOX", page, &Options::default());
    let stating = steps.rng.below(2) == 0;
    if stating {
        land(&mut model, &remotes, &mut run);
    }
    let label = match stating {
        true => "the inbox relocates, its server stating each handle",
        false => "the inbox relocates, its server stating none",
    };
    steps.run(String::from(label), run, &model);
    for id in ["work/Archive", "work/INBOX", "work/Archive"] {
        let run = model.sync_in(&mut remotes, id, page, &Options::default());
        steps.run(format!("{id}: a delta"), run, &model);
    }

    steps.history()
}

/// Lands each create a relocation of `run` delivered on the handle its
/// member has in `to`, as the write recording an outcome stating that
/// handle does (SYNC §4): the binding moved to it, its base the flags the
/// member carries, the staged body and no revision, nothing reported.
pub fn land(model: &mut Model, remotes: &BTreeMap<String, Remote>, run: &mut Run) {
    let source = run.source.clone();
    for (push, _) in &run.pushes {
        let Push::Remove {
            handle,
            to: Some(to),
            link_id,
            ..
        } = push
        else {
            continue;
        };
        let Some(outcome) = run
            .outcomes
            .iter_mut()
            .find(|outcome| &outcome.handle == handle && outcome.relocated == Some(true))
        else {
            continue;
        };
        let provisional = format!("\u{1}{link_id}");
        let Some(item) = model.items.iter_mut().find(|item| {
            &item.collection == to
                && item
                    .bindings
                    .iter()
                    .any(|binding| binding.source == source && binding.handle == provisional)
        }) else {
            continue;
        };
        let Some(body) = item.object.clone() else {
            continue;
        };
        let Some((assigned, member)) = remotes[to]
            .members
            .iter()
            .filter(|(_, member)| member.filed.hint.as_ref() == Some(link_id))
            .max_by_key(|(_, member)| member.arrival)
        else {
            continue;
        };
        let binding = item
            .bindings
            .iter_mut()
            .find(|binding| binding.source == source)
            .expect("the delivered create");
        binding.handle = assigned.clone();
        binding.base_present = true;
        binding.base_flags = Some(member.flags.clone());
        binding.base_object = Some(body);
        binding.base_revision = None;
        outcome.assigned = Some(assigned.clone());
    }
}

/// Bodies the server rewrites: a message appended here whose `Message-ID`
/// the server replaces, its append answered with a handle or none, and a
/// message received whose bytes gain a header before its body is read.
fn rewrites(seed: u64) -> History {
    let mut draw = Draw::new(seed);
    let mut rng = Rng::new(seed, STREAM + 6);
    let mut model = Model::new();
    let id = "work/INBOX";
    declare(&mut model, id, Kind::Mail, "work-imap");
    let mut inbox = Remote::new("work-imap", id);
    inbox.uidplus = rng.below(2) == 0;
    let mut received = BTreeMap::new();
    for _ in 0..3 + rng.below(5) {
        let message = draw.message(false);
        let (handle, _) = inbox.deliver(message.file(), flags(&mut rng));
        received.insert(handle, message);
    }
    let page = 2 + rng.below(3) as usize;
    let appended = draw.message(false);
    let mut steps = Recorder::new("a mailbox", &model, rng);

    let run = model.sync(&mut inbox, page);
    steps.run(String::from("a first round"), run, &model);
    let row = model.enqueue(
        "himalaya",
        id,
        Action::Add {
            link_id: appended.message_id.clone().unwrap_or_default(),
            flags: set(&["\\Seen"]),
            filed: appended.file(),
        },
    );
    model.drain(row);
    steps.record(String::from("a message filed here"), None, &model);
    let run = model.sync(&mut inbox, page);
    let answered = match inbox.uidplus {
        true => "its append answered with a handle",
        false => "its append answered with none",
    };
    steps.run(format!("a delta pushing it, {answered}"), run, &model);

    let newest = inbox
        .members
        .iter()
        .max_by_key(|(_, member)| member.arrival)
        .map(|(handle, _)| handle.clone())
        .expect("the appended member");
    let rewritten = Mail {
        message_id: Some(format!("rewritten.{newest}@mail.example.net")),
        ..appended.clone()
    };
    inbox.modseq += 1;
    let modseq = inbox.modseq;
    if let Some(member) = inbox.members.get_mut(&newest) {
        member.filed = rewritten.file();
        member.changed = modseq;
    }
    let (first, message) = received.iter().next().expect("a received message");
    let mut stamped = message.file();
    let mut bytes = b"X-Spam-Score: 0.1\r\n".to_vec();
    bytes.extend_from_slice(&stamped.bytes);
    stamped
        .summary
        .set("size", rusqlite::types::Value::Integer(bytes.len() as i64));
    stamped.bytes = bytes;
    inbox.modseq += 1;
    let modseq = inbox.modseq;
    if let Some(member) = inbox.members.get_mut(first) {
        member.filed = stamped;
        member.changed = modseq;
    }
    let run = model.sync(&mut inbox, page);
    steps.run(String::from("a delta listing both rewritten"), run, &model);
    let run = model.upgrade(&inbox, std::slice::from_ref(first));
    steps.record(
        String::from("the stamped message's body fetched"),
        Some(run),
        &model,
    );
    let run = model.sync(&mut inbox, page);
    steps.run(String::from("a delta with nothing to do"), run, &model);

    steps.history()
}

/// Collections empty and large: an empty mailbox synced, a large one in
/// pages losing more than half its members, parked and then accepted, and
/// one emptied whole, parked and then accepted.
fn extremes(seed: u64) -> History {
    let mut draw = Draw::new(seed);
    let mut rng = Rng::new(seed, STREAM + 7);
    let mut model = Model::new();
    declare(&mut model, "work/Empty", Kind::Mail, "empty-imap");
    declare(&mut model, "work/Large", Kind::Mail, "large-imap");
    declare(&mut model, "work/Emptied", Kind::Mail, "emptied-imap");
    let mut empty = Remote::new("empty-imap", "work/Empty");
    let mut large = Remote::new("large-imap", "work/Large");
    let mut emptied = Remote::new("emptied-imap", "work/Emptied");
    let count = 300 + rng.below(700);
    for _ in 0..count {
        large.deliver(draw.message(false).file(), flags(&mut rng));
    }
    for _ in 0..2 + rng.below(5) {
        emptied.deliver(draw.message(false).file(), BTreeSet::new());
    }
    let page = 50 + rng.below(150) as usize;
    let mut steps = Recorder::new("three mailboxes", &model, rng);

    let run = model.sync(&mut empty, 3);
    steps.run(String::from("an empty mailbox's first round"), run, &model);
    let run = model.sync(&mut empty, 3);
    steps.run(String::from("an empty mailbox's delta"), run, &model);

    let run = model.sync(&mut large, page);
    steps.run(format!("{count} members in pages of {page}"), run, &model);
    for handle in handles(&large) {
        if steps.rng.below(10) == 0 {
            large.set_flags(&handle, set(&["\\Seen", "\\Flagged"]));
        }
    }
    let run = model.sync(&mut large, page);
    steps.run(String::from("a delta over flag changes"), run, &model);
    let lost = count / 2 + 1 + steps.rng.below(count / 4);
    for handle in handles(&large).into_iter().take(lost as usize) {
        large.forget(&handle);
    }
    let run = model.sync_with(&mut large, page, &from(12));
    steps.run(String::from("a delta narrowing it"), run, &model);
    let run = model.sync(&mut large, page);
    steps.run(format!("a round missing {lost}, parked"), run, &model);
    let run = model.sync_with(&mut large, page, &accepting());
    steps.run(String::from("the absences accepted"), run, &model);

    let run = model.sync(&mut emptied, 3);
    steps.run(String::from("a mailbox's first round"), run, &model);
    for handle in handles(&emptied) {
        emptied.forget(&handle);
    }
    let run = model.sync_with(&mut emptied, 3, &from(12));
    steps.run(String::from("a delta narrowing it"), run, &model);
    let run = model.sync(&mut emptied, 3);
    steps.run(String::from("an empty listing, parked"), run, &model);
    let run = model.sync_with(&mut emptied, 3, &accepting());
    steps.run(String::from("the emptied mailbox accepted"), run, &model);

    steps.history()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_quirk_history_holds_its_steps_and_runs() {
        for seed in [7, 42, 1337] {
            for (name, history) in histories(seed) {
                assert!(history.steps.len() > 2, "{name} at {seed}");
                assert!(
                    history.steps.iter().any(|step| step.run.is_some()),
                    "{name} at {seed}"
                );
            }
        }
    }

    #[test]
    fn a_jumbled_page_lists_its_first_entries_as_the_sorted_page() {
        let mut draw = Draw::new(7);
        let mut remote = Remote::new("work-imap", "work/INBOX");
        for _ in 0..5 {
            remote.deliver(draw.message(false).file(), set(&["\\Seen"]));
        }
        let clean = remote.round(10, &Scope::default()).remove(0);
        for seed in 0..20 {
            let mut page = clean.clone();
            jumble(&mut page, &mut Rng::new(seed, STREAM));
            let mut seen = BTreeSet::new();
            let first: Vec<(String, BTreeSet<String>)> = page
                .items
                .iter()
                .filter(|item| seen.insert(item.handle.clone()))
                .map(|item| (item.handle.clone(), item.flags.clone()))
                .collect();
            let mut sorted = first.clone();
            sorted.sort();
            let want: Vec<(String, BTreeSet<String>)> = clean
                .items
                .iter()
                .map(|item| (item.handle.clone(), item.flags.clone()))
                .collect();
            assert_eq!(sorted, want, "seed {seed}");
            assert!(page.items.len() <= clean.items.len() + 1);
        }
    }
}
