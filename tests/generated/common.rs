//! # Generated stores
//!
//! A seed drawn into the generator's model, the model written into a store
//! by the canonical statements alone, and the cells both sides are compared
//! in: io-pimdir's answers projected onto the columns of the statement the
//! oracle states, the values no model predicts written as its markers.

use std::path::PathBuf;

use io_pimdir::{
    client::reader::PimdirItem,
    object::PimdirHash,
    placement::{PimdirFlags, PimdirLevel},
    summary::{PimdirSummary, calendar::PimdirTime},
};
use pimdir_generator::{
    model::Model,
    oracle::{INSTANT, INSTANTS, STAMP, STORE_ID},
    profile::Profile,
    spec::Spec,
    store::Store,
    writer,
};
use rusqlite::types::Value;
use tempfile::TempDir;

/// The marker a park's reason is compared as, its text being free.
pub const PARKED: &str = "parked";

/// The seeds every generated test runs, fixed so a failure replays.
pub const SEEDS: [u64; 4] = [7, 42, 1337, 20_261_120];

/// A model and the store the generator wrote from it.
pub struct Generated {
    /// The canonical text the store was written through.
    pub spec: Spec,
    /// The truth the store holds.
    pub model: Model,
    dir: TempDir,
}

impl Generated {
    /// The structure profile's store for `seed`.
    pub fn structure(seed: u64) -> Self {
        let spec = Spec::load_default().expect("the vendored spec");
        let model = Profile::Structure
            .generate(&spec, seed)
            .expect("a structure model");
        Self::write(spec, model)
    }

    /// The store the generator writes from `model`, its lock released.
    pub fn write(spec: Spec, model: Model) -> Self {
        let dir = tempfile::tempdir().expect("a temp dir");
        let store = Store::create(&spec, &dir.path().join("store")).expect("a fresh store");
        writer::materialise(&model, &store).expect("the model written");
        drop(store);
        Self { spec, model, dir }
    }

    /// The store's directory.
    pub fn root(&self) -> PathBuf {
        self.dir.path().join("store")
    }
}

/// A flag set as the `flags` column spells it, `NULL` when unknown.
pub fn flags(flags: &PimdirFlags) -> Value {
    match flags {
        PimdirFlags::Unknown => Value::Null,
        PimdirFlags::Known(set) => Value::Text(set.iter().cloned().collect::<Vec<_>>().join(" ")),
    }
}

/// A level as the `level` column holds it.
pub fn level(level: PimdirLevel) -> Value {
    match level {
        PimdirLevel::Meta => Value::Integer(1),
        PimdirLevel::Full => Value::Integer(2),
    }
}

/// A body's hash, `NULL` when none.
pub fn hash(hash: Option<&PimdirHash>) -> Value {
    text(hash.map(PimdirHash::as_str))
}

/// A text cell, `NULL` when none.
pub fn text(text: Option<&str>) -> Value {
    text.map_or(Value::Null, |text| Value::Text(String::from(text)))
}

/// Writes a flag set in one order, a value no model predicts (an instant
/// SQLite stamps, a change stamp, the store's id) as the marker the oracle
/// states for it once its shape is checked, and a park's reason, free
/// text (STORAGE §15.2), as [`PARKED`].
pub fn normalise(columns: &[&str], row: &mut [Value]) {
    for (column, value) in columns.iter().zip(row.iter_mut()) {
        let marker = match (*column, &*value) {
            ("flags" | "base_flags", Value::Text(text)) => {
                let mut set: Vec<&str> = text.split(' ').filter(|flag| !flag.is_empty()).collect();
                set.sort_unstable();
                Some(set.join(" "))
            }
            (column, Value::Text(text)) if INSTANTS.contains(&column) && is_instant(text) => {
                Some(String::from(INSTANT))
            }
            ("changed", Value::Integer(stamp)) if *stamp > 0 => Some(String::from(STAMP)),
            ("error", Value::Text(text)) if !text.starts_with("unconfirmed: ") => {
                Some(String::from(PARKED))
            }
            ("store_id", Value::Text(text))
                if text.len() == 32
                    && text
                        .bytes()
                        .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')) =>
            {
                Some(String::from(STORE_ID))
            }
            _ => None,
        };
        if let Some(marker) = marker {
            *value = Value::Text(marker);
        }
    }
}

/// The leading columns of every item read.
pub const ITEM: [&str; 6] = [
    "seq",
    "link_id",
    "flags",
    "object_hash",
    "sort_key",
    "level",
];

/// The columns of `mail_summary` a read joins.
pub const MAIL: [&str; 7] = [
    "message_id",
    "subject",
    "sender",
    "sender_name",
    "date",
    "size",
    "attachment",
];

/// The columns of `contact_summary` a read joins.
pub const CONTACT: [&str; 4] = ["uid", "fn", "kind", "org"];

/// The columns of `event_summary` a read joins.
pub const EVENT: [&str; 9] = [
    "uid",
    "summary",
    "location",
    "dtstart",
    "dtstart_tzid",
    "dtstart_value",
    "dtend",
    "recurring",
    "until",
];

/// The columns of `task_summary` a read joins.
pub const TASK: [&str; 17] = [
    "uid",
    "summary",
    "dtstart",
    "dtstart_tzid",
    "dtstart_value",
    "due",
    "due_tzid",
    "due_value",
    "status",
    "completed",
    "percent",
    "recurring",
    "until",
    "priority",
    "parent",
    "mid",
    "created",
];

/// The columns of `journal_summary` a read joins.
pub const JOURNAL: [&str; 5] = ["uid", "summary", "dtstart", "dtstart_tzid", "dtstart_value"];

/// The columns of `file_summary` a read joins.
pub const FILE: [&str; 4] = ["name", "media_type", "size", "part"];

/// The six columns every item read leads with.
pub fn item_cells(item: &PimdirItem) -> Vec<Value> {
    vec![
        Value::Integer(item.seq),
        Value::Text(item.link_id.0.clone()),
        flags(&item.flags),
        hash(item.object.as_ref()),
        Value::Text(item.sort_key.clone()),
        level(item.level),
    ]
}

/// The summary columns of `table` and what `summary` holds in them.
pub fn table_cells(
    table: &str,
    summary: Option<&PimdirSummary>,
) -> (&'static [&'static str], Vec<Value>) {
    let int = |value: Option<i64>| value.map_or(Value::Null, Value::Integer);
    let flag = |value: Option<bool>| int(value.map(i64::from));
    let size = |value: Option<u64>| int(value.map(|size| size as i64));
    let time = |time: Option<&PimdirTime>| {
        [
            text(time.map(|time| time.value.as_str())),
            text(time.and_then(|time| time.tzid.as_deref())),
            text(time.map(|time| time.value_kind())),
        ]
    };

    let (columns, cells): (&'static [&'static str], Option<Vec<Value>>) = match table {
        "mail" => (
            &MAIL,
            match summary {
                Some(PimdirSummary::Mail(mail)) => Some(vec![
                    text(mail.message_id.as_deref()),
                    Value::Text(mail.subject.clone()),
                    text(mail.sender.as_deref()),
                    text(mail.sender_name.as_deref()),
                    text(mail.date.as_deref()),
                    size(mail.size),
                    flag(mail.attachment),
                ]),
                _ => None,
            },
        ),
        "contact" | "contacts" => (
            &CONTACT,
            match summary {
                Some(PimdirSummary::Contact(card)) => Some(vec![
                    text(card.uid.as_deref()),
                    Value::Text(card.full_name.clone()),
                    text(card.kind.as_deref()),
                    text(card.org.as_deref()),
                ]),
                _ => None,
            },
        ),
        "event" | "events" => (
            &EVENT,
            match summary {
                Some(PimdirSummary::Event(event)) => {
                    let mut cells = vec![
                        text(event.uid.as_deref()),
                        Value::Text(event.summary.clone()),
                        text(event.location.as_deref()),
                    ];
                    cells.extend(time(event.dtstart.as_ref()));
                    cells.extend([
                        text(event.dtend.as_deref()),
                        flag(event.recurring),
                        text(event.until.as_deref()),
                    ]);
                    Some(cells)
                }
                _ => None,
            },
        ),
        "task" | "tasks" => (
            &TASK,
            match summary {
                Some(PimdirSummary::Task(task)) => {
                    let mut cells =
                        vec![text(task.uid.as_deref()), Value::Text(task.summary.clone())];
                    cells.extend(time(task.dtstart.as_ref()));
                    cells.extend(time(task.due.as_ref()));
                    cells.extend([
                        text(task.status.as_deref()),
                        text(task.completed.as_deref()),
                        int(task.percent),
                        flag(task.recurring),
                        text(task.until.as_deref()),
                        int(task.priority),
                        text(task.parent.as_deref()),
                        text(task.mid.as_deref()),
                        text(task.created.as_deref()),
                    ]);
                    Some(cells)
                }
                _ => None,
            },
        ),
        "journal" | "journals" => (
            &JOURNAL,
            match summary {
                Some(PimdirSummary::Journal(journal)) => {
                    let mut cells = vec![
                        text(journal.uid.as_deref()),
                        Value::Text(journal.summary.clone()),
                    ];
                    cells.extend(time(journal.dtstart.as_ref()));
                    Some(cells)
                }
                _ => None,
            },
        ),
        "file" | "files" => (
            &FILE,
            match summary {
                Some(PimdirSummary::File(file)) => Some(vec![
                    Value::Text(file.name.clone()),
                    text(file.media_type.as_deref()),
                    size(file.size),
                    text(file.part.as_deref()),
                ]),
                _ => None,
            },
        ),
        table => panic!("no summary table {table}"),
    };

    let cells = cells.unwrap_or_else(|| vec![Value::Null; columns.len()]);
    (columns, cells)
}

fn is_instant(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 24
        && bytes.iter().enumerate().all(|(i, byte)| match i {
            4 | 7 => *byte == b'-',
            10 => *byte == b'T',
            13 | 16 => *byte == b':',
            19 => *byte == b'.',
            23 => *byte == b'Z',
            _ => byte.is_ascii_digit(),
        })
}
