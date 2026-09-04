//! A summary row as the operator sees it: its table, its columns and its
//! addresses, rendered from store data alone. The library types carry no
//! serde, so this is the one adapter every verb prints a summary through.

use io_pimdir::summary::{PimdirSummary, calendar::PimdirTime};
use serde_json::{Map, Value, json};

/// A summary as `{ table, row, addresses }`, the columns spelled as the
/// store's own (STORAGE Annex A).
pub fn json(summary: &PimdirSummary) -> Value {
    let (table, row) = columns(summary);
    let addresses: Vec<Value> = summary
        .addresses()
        .into_iter()
        .map(|(role, address)| {
            json!({ "role": role.as_str(), "address": address.address, "name": address.name })
        })
        .collect();

    json!({ "table": table, "row": row, "addresses": addresses })
}

/// The lines `item show` prints under a placement, off the value
/// [`json()`] built: one per column, then one per address.
pub fn lines(summary: &Value) -> Vec<String> {
    let mut lines = vec![format!(
        " - summary ({}):",
        summary["table"].as_str().unwrap_or("?")
    )];
    for (column, value) in summary["row"].as_object().into_iter().flatten() {
        let value = match value {
            Value::Null => String::from("-"),
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        lines.push(format!("    - {column}: {value}"));
    }
    for address in summary["addresses"].as_array().into_iter().flatten() {
        let role = address["role"].as_str().unwrap_or("?");
        let spec = address["address"].as_str().unwrap_or("?");
        match address["name"].as_str() {
            Some(name) => lines.push(format!("    - {role}: {name} <{spec}>")),
            None => lines.push(format!("    - {role}: {spec}")),
        }
    }

    lines
}

/// The summary's table and its columns, in the table's order.
fn columns(summary: &PimdirSummary) -> (&'static str, Map<String, Value>) {
    let mut row = Map::new();
    let mut put = |column: &str, value: Value| {
        row.insert(column.to_string(), value);
    };
    let time = |put: &mut dyn FnMut(&str, Value), name: &str, time: &Option<PimdirTime>| {
        put(name, json!(time.as_ref().map(|t| &t.value)));
        put(
            &format!("{name}_tzid"),
            json!(time.as_ref().and_then(|t| t.tzid.clone())),
        );
        put(
            &format!("{name}_value"),
            json!(time.as_ref().map(|t| t.value_kind())),
        );
    };

    let table = match summary {
        PimdirSummary::Mail(mail) => {
            put("message_id", json!(mail.message_id));
            put("in_reply_to", json!(mail.in_reply_to));
            put("subject", json!(mail.subject));
            put("sender", json!(mail.sender));
            put("sender_name", json!(mail.sender_name));
            put("date", json!(mail.date));
            put("size", json!(mail.size));
            put("attachment", json!(mail.attachment.map(i64::from)));
            "mail_summary"
        }
        PimdirSummary::Contact(contact) => {
            put("uid", json!(contact.uid));
            put("fn", json!(contact.full_name));
            put("kind", json!(contact.kind));
            put("org", json!(contact.org));
            "contact_summary"
        }
        PimdirSummary::Event(event) => {
            put("uid", json!(event.uid));
            put("summary", json!(event.summary));
            put("location", json!(event.location));
            time(&mut put, "dtstart", &event.dtstart);
            put("dtend", json!(event.dtend));
            put("recurring", json!(event.recurring.map(i64::from)));
            put("until", json!(event.until));
            "event_summary"
        }
        PimdirSummary::Task(task) => {
            put("uid", json!(task.uid));
            put("summary", json!(task.summary));
            time(&mut put, "dtstart", &task.dtstart);
            time(&mut put, "due", &task.due);
            put("status", json!(task.status));
            put("completed", json!(task.completed));
            put("percent", json!(task.percent));
            put("recurring", json!(task.recurring.map(i64::from)));
            put("until", json!(task.until));
            "task_summary"
        }
        PimdirSummary::Journal(journal) => {
            put("uid", json!(journal.uid));
            put("summary", json!(journal.summary));
            time(&mut put, "dtstart", &journal.dtstart);
            "journal_summary"
        }
    };

    (table, row)
}
