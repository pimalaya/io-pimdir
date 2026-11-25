//! # The reader against the oracle
//!
//! Every expectation the generator's oracle states over a generated store
//! (STORAGE §14.1, §15.4, §4.5), answered through the [`PimdirReader`]
//! call wrapping its statement at the same parameter point, and compared
//! on the columns that call exposes. A statement no reader call wraps is
//! named in [`UNWRAPPED`], so one the oracle adds fails until it is
//! mapped or listed.

use core::cmp::Ordering;

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use io_pimdir::{
    client::{
        PimdirError,
        producer::PimdirActionStatus,
        reader::{
            PimdirAccountAttachmentCursor, PimdirAddressCursor, PimdirItem, PimdirMailCursor,
            PimdirMailEntry, PimdirMailFilter, PimdirReader,
        },
    },
    placement::PimdirLinkId,
    reference::PimdirEndpoint,
    summary::{PimdirAddress, PimdirAddressRole, PimdirSummary},
};
use pimdir_generator::{
    model::Model,
    oracle::{self, Expectation},
    spec::Spec,
};
use rusqlite::types::Value;

use crate::common::{
    FILE, Generated, ITEM, SEEDS, flags, hash, item_cells, level, normalise, table_cells, text,
};

/// The statements the oracle states that no reader call wraps, each the
/// owner's or the engine's to run.
const UNWRAPPED: [&str; 3] = [
    "storage/read/handle_for_link",
    "storage/owner/load_conflict",
    "storage/owner/load_round",
];

#[test]
fn every_read_of_a_generated_store_answers_what_the_oracle_states() {
    for seed in SEEDS {
        let (total, disagreements) = disagreements(seed);
        let unknown: Vec<&String> = disagreements
            .iter()
            .filter(|(known, _)| known.is_none())
            .map(|(_, disagreement)| disagreement)
            .collect();
        assert!(
            unknown.is_empty(),
            "seed {seed}: {} of {total} points disagree:\n{unknown:#?}",
            unknown.len(),
        );
    }
}

/// The same, every known finding included: G1 (a malformed pending row
/// failing its collection's pending listing, where STORAGE §15.4 lists a
/// row a reader cannot show) failed it until the row was listed opaque.
#[test]
fn every_read_of_a_generated_store_answers_what_the_oracle_states_known_included() {
    let disagreements: Vec<(u64, Option<&str>, String)> = SEEDS
        .into_iter()
        .flat_map(|seed| {
            disagreements(seed)
                .1
                .into_iter()
                .map(move |(known, disagreement)| (seed, known, disagreement))
        })
        .collect();
    assert!(disagreements.is_empty(), "{disagreements:#?}");
}

/// Every point of the oracle at `seed` io-pimdir's reader disagrees on,
/// each with the known finding it stands under, beside how many points
/// were stated.
fn disagreements(seed: u64) -> (usize, Vec<(Option<&'static str>, String)>) {
    disagreements_of(Generated::structure(seed).model)
}

/// Every point of the oracle over the store of `model` io-pimdir's reader
/// disagrees on, beside how many points were stated.
pub fn disagreements_of(model: Model) -> (usize, Vec<(Option<&'static str>, String)>) {
    let generated = Generated::write(Spec::load_default().unwrap(), model);
    compare(
        &generated.spec,
        &generated.model,
        &generated.root(),
        |_, _, _, _| None,
    )
}

/// Every point of the oracle over `model` the reader of the store at
/// `root` disagrees on, each with the known finding it stands under,
/// beside how many points were stated. `differ` names the finding a
/// point no known one covers stands under, from the columns compared and
/// the rows io-pimdir and the oracle answer there.
pub fn compare(
    spec: &Spec,
    model: &Model,
    root: &Path,
    differ: impl Fn(&Expectation, &[&str], &[Vec<Value>], &[Vec<Value>]) -> Option<&'static str>,
) -> (usize, Vec<(Option<&'static str>, String)>) {
    let reader = PimdirReader::open(root).unwrap();
    let expectations = oracle::expectations(model, spec).unwrap();

    let mut disagreements = Vec::new();
    let mut answered: BTreeSet<&str> = BTreeSet::new();
    let mut skipped: BTreeSet<&str> = BTreeSet::new();
    for expectation in &expectations {
        let statement = expectation.statement.as_str();
        let known = known(model, expectation);
        let answer = match answer(&reader, expectation) {
            Ok(Some(answer)) => answer,
            Ok(None) => {
                skipped.insert(statement);
                continue;
            }
            Err(err) => {
                let disagreement = format!(
                    "{statement} at {:?}: the reader fails: {err}",
                    expectation.params
                );
                disagreements.push((known, disagreement));
                continue;
            }
        };
        answered.insert(statement);

        let columns = &spec.statement(statement).unwrap().columns;
        let mut want: Vec<Vec<Value>> = expectation
            .rows
            .iter()
            .map(|row| {
                answer
                    .columns
                    .iter()
                    .map(|name| {
                        let at = columns
                            .iter()
                            .position(|column| column == name)
                            .unwrap_or_else(|| panic!("{statement} answers no {name}"));
                        row[at].clone()
                    })
                    .collect()
            })
            .collect();
        let mut got = answer.rows;
        for row in got.iter_mut().chain(want.iter_mut()) {
            normalise(&answer.columns, row);
        }
        if !expectation.ordered {
            got.sort_by_key(|row| format!("{row:?}"));
            want.sort_by_key(|row| format!("{row:?}"));
        }
        if got != want {
            let disagreement = format!(
                "{statement} at {:?} {:?} on {:?}: io-pimdir answers {got:?}, the oracle {want:?}",
                expectation.params, expectation.slot, answer.columns
            );
            let known = known.or_else(|| differ(expectation, &answer.columns, &got, &want));
            disagreements.push((known, disagreement));
        }
    }

    let unmapped: Vec<&str> = skipped
        .difference(&answered)
        .copied()
        .filter(|statement| !UNWRAPPED.contains(statement))
        .collect();
    assert!(unmapped.is_empty(), "no reader call answers {unmapped:?}");

    (expectations.len(), disagreements)
}

/// The known finding a point stands under, if any: none since G1 (a
/// malformed pending row failing its collection's listing) was fixed.
fn known(_: &Model, _: &Expectation) -> Option<&'static str> {
    None
}

/// What a reader call answers at an expectation's point, projected onto
/// the statement's columns it exposes.
struct Answer {
    columns: Vec<&'static str>,
    rows: Vec<Vec<Value>>,
}

impl Answer {
    fn new(columns: &[&'static str], rows: Vec<Vec<Value>>) -> Self {
        Self {
            columns: columns.to_vec(),
            rows,
        }
    }
}

/// The reader call wrapping the expectation's statement, run at its point;
/// `None` for a point no call reaches.
fn answer(reader: &PimdirReader, expectation: &Expectation) -> Result<Option<Answer>, PimdirError> {
    let name = expectation
        .statement
        .rsplit_once('/')
        .map_or(expectation.statement.as_str(), |(_, name)| name);
    let at = Point(expectation);

    let answer = match name {
        "list_collections" | "list_collections_by_account" => {
            let collections = match name {
                "list_collections" => reader.list_collections()?,
                _ => reader.list_collections_by_account(at.text("account").as_deref())?,
            };
            let rows = collections
                .into_iter()
                .map(|collection| {
                    let coverage = collection.coverage.as_ref();
                    vec![
                        Value::Text(collection.id),
                        text(collection.account.as_deref()),
                        Value::Text(collection.kind),
                        Value::Text(collection.name),
                        text(collection.parent.as_deref()),
                        text(collection.color.as_deref()),
                        text(collection.description.as_deref()),
                        collection.sort_order.map_or(Value::Null, Value::Integer),
                        Value::Integer(collection.generation),
                        text(collection.role.as_deref()),
                        text(coverage.and_then(|coverage| coverage.scope.since.as_deref())),
                        text(coverage.and_then(|coverage| coverage.scope.until.as_deref())),
                        text(coverage.map(|coverage| coverage.at.as_str())),
                    ]
                })
                .collect();
            Answer::new(
                &[
                    "id",
                    "account",
                    "kind",
                    "name",
                    "parent",
                    "color",
                    "description",
                    "sort_order",
                    "generation",
                    "role",
                    "covered_since",
                    "covered_until",
                    "covered_at",
                ],
                rows,
            )
        }
        "list_accounts" => Answer::new(
            &["account"],
            reader
                .list_accounts()?
                .into_iter()
                .map(|account| vec![Value::Text(account)])
                .collect(),
        ),
        "list_item_properties" => Answer::new(
            &["key", "value"],
            reader
                .list_item_properties(at.int("seq").unwrap())?
                .into_iter()
                .map(|property| vec![Value::Text(property.key), Value::Text(property.value)])
                .collect(),
        ),
        "list_property_values" => Answer::new(
            &["value"],
            reader
                .list_property_values(&at.string("key"))?
                .into_iter()
                .map(|value| vec![Value::Text(value)])
                .collect(),
        ),
        "list_property_holders" | "list_value_holders" => {
            let key = at.string("key");
            let holders = match name {
                "list_property_holders" => reader.list_property_holders(&key)?,
                _ => reader.list_value_holders(&key, &at.string("value"))?,
            };
            Answer::new(
                &["collection", "seq"],
                holders
                    .into_iter()
                    .map(|holder| vec![Value::Text(holder.collection), Value::Integer(holder.seq)])
                    .collect(),
            )
        }
        "list_sources" => Answer::new(
            &["source"],
            reader
                .distinct_sources()?
                .into_iter()
                .map(|source| vec![Value::Text(source)])
                .collect(),
        ),
        "load_kind" => Answer::new(
            &["kind"],
            reader
                .collection_kind(at.string("collection"))?
                .into_iter()
                .map(|kind| vec![Value::Text(kind)])
                .collect(),
        ),
        "load_account" => Answer::new(
            &["account"],
            reader
                .collection_account(at.string("collection"))?
                .into_iter()
                .map(|account| vec![text(account.as_deref())])
                .collect(),
        ),
        "load_generation" => Answer::new(
            &["generation"],
            reader
                .generation(at.string("collection"))?
                .into_iter()
                .map(|generation| vec![Value::Integer(generation)])
                .collect(),
        ),
        "list_coverage" => {
            let rows = reader
                .list_coverage(at.string("collection"))?
                .into_iter()
                .map(|source| {
                    let coverage = source.coverage.as_ref();
                    let round = source.round.as_ref();
                    vec![
                        Value::Text(source.source.clone()),
                        text(coverage.and_then(|coverage| coverage.scope.since.as_deref())),
                        text(coverage.and_then(|coverage| coverage.scope.until.as_deref())),
                        text(coverage.map(|coverage| coverage.at.as_str())),
                        text(round.map(|round| round.started_at.as_str())),
                        text(round.and_then(|round| round.scope.since.as_deref())),
                        text(round.and_then(|round| round.scope.until.as_deref())),
                    ]
                })
                .collect();
            Answer::new(
                &[
                    "source",
                    "covered_since",
                    "covered_until",
                    "covered_at",
                    "round_started_at",
                    "round_since",
                    "round_until",
                ],
                rows,
            )
        }
        "count_items" => Answer::new(
            &["count"],
            vec![vec![Value::Integer(
                reader.count_items(at.string("collection"))? as i64,
            )]],
        ),
        "count_retained" => Answer::new(
            &["count"],
            vec![vec![Value::Integer(
                reader.count_retained(at.string("collection"))?,
            )]],
        ),
        "list_items_page" => {
            let items = reader.list_items(
                at.string("collection"),
                at.text("after").as_deref(),
                at.limit(),
            )?;
            Answer::new(&ITEM, items.iter().map(item_cells).collect())
        }
        "list_items_page_asc" | "list_items_page_desc" => {
            let after = at.text("after_key").zip(at.int("after_seq"));
            let after = after.as_ref().map(|(key, seq)| (key.as_str(), *seq));
            let items = match name {
                "list_items_page_asc" => {
                    reader.list_items_page_asc(at.string("collection"), after, at.limit())?
                }
                _ => reader.list_items_page_desc(at.string("collection"), after, at.limit())?,
            };
            Answer::new(&ITEM, items.iter().map(item_cells).collect())
        }
        "list_mail_page_desc"
        | "list_contacts_page_asc"
        | "list_events_page_asc"
        | "list_tasks_page_asc"
        | "list_journals_page_asc"
        | "list_files_page_asc" => {
            let after = at.text("after_key").zip(at.int("after_seq"));
            let after = after.as_ref().map(|(key, seq)| (key.as_str(), *seq));
            let items = reader.list_summaries(at.string("collection"), after, at.limit())?;
            let table = name.trim_start_matches("list_").split('_').next().unwrap();
            let (columns, rows) = summarised(&[], table, items.iter().map(|item| (None, item)));
            Answer { columns, rows }
        }
        "list_retained_page" => {
            let items =
                reader.list_retained(at.string("collection"), at.int("after"), at.limit())?;
            let rows = items
                .iter()
                .map(|item| {
                    let retention = item.retention.as_ref();
                    let mut cells = item_cells(item);
                    cells.extend([
                        text(retention.and_then(|retention| retention.at.as_deref())),
                        text(retention.and_then(|retention| retention.by.as_deref())),
                        retention
                            .and_then(|retention| retention.size)
                            .map_or(Value::Null, |size| Value::Integer(size as i64)),
                    ]);
                    cells
                })
                .collect();
            let mut columns = ITEM.to_vec();
            columns.extend(["retained_at", "retained_by", "size"]);
            Answer { columns, rows }
        }
        "load_values_by_collection" | "load_values_by_link" => {
            let held = held(reader, &at.string("collection"))?;
            let rows = held
                .iter()
                .flat_map(|(link, summary)| {
                    values(summary.as_ref()).into_iter().map(move |row| {
                        let mut cells = vec![Value::Text(link.clone())];
                        cells.extend(row);
                        cells
                    })
                })
                .collect();
            Answer::new(&["link_id", "role", "position", "value", "name"], rows)
        }
        "load_values" => {
            let held = held(reader, &at.string("collection"))?;
            let summary = held.get(&at.string("link_id")).cloned().flatten();
            Answer::new(
                &["role", "position", "value", "name"],
                values(summary.as_ref()),
            )
        }
        "get_item" => Answer::new(
            &ITEM,
            reader
                .get_item(at.string("collection"), at.int("seq").unwrap())?
                .iter()
                .map(item_cells)
                .collect(),
        ),
        "get_mail" | "get_contact" | "get_event" | "get_task" | "get_journal" | "get_file" => {
            let item = reader.get_item(at.string("collection"), at.int("seq").unwrap())?;
            let table = name.trim_start_matches("get_");
            let (columns, rows) = summarised(&[], table, item.iter().map(|item| (None, item)));
            Answer { columns, rows }
        }
        "get_mail_row" => {
            let link = PimdirLinkId(at.string("link_id"));
            let entry = reader.get_mail_row(at.string("collection"), &link)?;
            entries(&["collection"], "mail", entry.iter())
        }
        "seq_by_link" => Answer::new(
            &["seq"],
            reader
                .seq_for_link(at.string("collection"), &at.string("link_id"))?
                .into_iter()
                .map(|seq| vec![Value::Integer(seq)])
                .collect(),
        ),
        "list_item_bindings" => {
            let link = at.string("link_id");
            let rows = reader
                .item_bindings(at.string("collection"), &link)?
                .into_iter()
                .map(|(source, binding)| {
                    let base = binding.base.as_ref();
                    vec![
                        Value::Text(link.clone()),
                        Value::Text(source.0),
                        Value::Text(binding.handle.0.clone()),
                        base.map_or(Value::Null, |base| flags(&base.flags)),
                        hash(base.and_then(|base| base.object.as_ref())),
                        text(base.and_then(|base| base.revision.as_deref())),
                        Value::Integer(i64::from(base.is_some())),
                        Value::Integer(i64::from(binding.conflicted)),
                        text(binding.conflict_revision.as_deref()),
                        hash(binding.conflict_object.as_ref()),
                        hash(binding.shared_object.as_ref()),
                    ]
                })
                .collect();
            Answer::new(
                &[
                    "link_id",
                    "source",
                    "handle",
                    "base_flags",
                    "base_object",
                    "base_revision",
                    "base_present",
                    "conflicted",
                    "conflict_revision",
                    "conflict_object",
                    "shared_object",
                ],
                rows,
            )
        }
        "list_link_placements" => Answer::new(
            &[
                "collection",
                "account",
                "seq",
                "object_hash",
                "flags",
                "level",
            ],
            reader
                .link_placements(&at.string("link_id"))?
                .into_iter()
                .map(|at| {
                    vec![
                        Value::Text(at.collection),
                        text(at.account.as_deref()),
                        Value::Integer(at.seq),
                        hash(at.object.as_ref()),
                        flags(&at.flags),
                        level(at.level),
                    ]
                })
                .collect(),
        ),
        "list_object_placements" => Answer::new(
            &["collection", "account", "seq", "link_id", "flags", "level"],
            reader
                .object_placements(&at.string("hash"))?
                .into_iter()
                .map(|at| {
                    vec![
                        Value::Text(at.collection),
                        text(at.account.as_deref()),
                        Value::Integer(at.seq),
                        Value::Text(at.link_id.0),
                        flags(&at.flags),
                        level(at.level),
                    ]
                })
                .collect(),
        ),
        "describe_endpoint" => Answer::new(
            &["collection", "seq", "title"],
            reader
                .describe_endpoint(&at.endpoint())?
                .into_iter()
                .map(|view| {
                    vec![
                        Value::Text(view.collection),
                        Value::Integer(view.seq),
                        text(view.title.as_deref()),
                    ]
                })
                .collect(),
        ),
        "retained_bytes" => Answer::new(
            &["bytes"],
            vec![vec![Value::Integer(reader.retained_bytes()? as i64)]],
        ),
        "count_mail" => Answer::new(
            &["count"],
            vec![vec![Value::Integer(reader.count_mail(
                &at.slot(),
                at.filter(),
                at.text("since").as_deref(),
            )? as i64)]],
        ),
        "count_unread" => Answer::new(
            &["collection", "count"],
            reader
                .count_unread(
                    &at.slot(),
                    at.bool("attachment"),
                    at.text("since").as_deref(),
                )?
                .into_iter()
                .map(|(collection, count)| {
                    vec![Value::Text(collection), Value::Integer(count as i64)]
                })
                .collect(),
        ),
        "count_mail_by_day" => Answer::new(
            &["day", "count"],
            reader
                .count_mail_by_day(
                    &at.slot(),
                    at.filter(),
                    at.text("since").as_deref(),
                    at.int("shift"),
                )?
                .into_iter()
                .map(|day| vec![text(day.day.as_deref()), Value::Integer(day.count as i64)])
                .collect(),
        ),
        "sum_mail" => {
            let sum = reader.sum_mail(
                &at.slot(),
                at.filter(),
                at.bool("held"),
                at.text("since").as_deref(),
                at.text("until").as_deref(),
            )?;
            Answer::new(
                &["messages", "bytes", "unsized"],
                vec![vec![
                    Value::Integer(sum.count as i64),
                    Value::Integer(sum.size as i64),
                    Value::Integer(sum.unknown as i64),
                ]],
            )
        }
        "list_mail_page_filtered" => {
            let page = reader.list_mail_page_filtered(
                &at.slot(),
                at.filter(),
                at.text("since").as_deref(),
                at.cursor().as_ref(),
                at.limit(),
            )?;
            entries(&["collection"], "mail", page.iter())
        }
        "search_mail" => {
            let page = reader.search_mail(
                &at.slot(),
                &at.string("pattern"),
                at.filter(),
                at.cursor().as_ref(),
                at.limit(),
            )?;
            entries(&["collection"], "mail", page.iter())
        }
        "search_contacts" | "search_files" => {
            let pattern = at.string("pattern");
            let (page, table) = match name {
                "search_contacts" => (
                    reader.search_contacts(
                        &at.slot(),
                        &pattern,
                        at.cursor().as_ref(),
                        at.limit(),
                    )?,
                    "contact",
                ),
                _ => (
                    reader.search_files(&at.slot(), &pattern, at.cursor().as_ref(), at.limit())?,
                    "file",
                ),
            };
            entries(&["collection"], table, page.iter())
        }
        "search_calendar" => {
            let page = reader.search_calendar(
                &at.slot(),
                &at.string("pattern"),
                at.cursor().as_ref(),
                at.limit(),
            )?;
            let rows = page
                .into_iter()
                .map(|hit| {
                    let mut cells = vec![Value::Text(hit.entry.collection.clone())];
                    cells.extend(item_cells(&hit.entry.item));
                    cells.extend([Value::Text(hit.component), Value::Text(hit.title)]);
                    cells
                })
                .collect();
            let mut columns = vec!["collection"];
            columns.extend(ITEM);
            columns.extend(["component", "title"]);
            Answer { columns, rows }
        }
        "list_replied" => {
            let ids: Vec<String> = at.slot();
            Answer::new(
                &["message_id"],
                reader
                    .replied(&ids)?
                    .into_iter()
                    .map(|id| vec![Value::Text(id)])
                    .collect(),
            )
        }
        "list_conflicted_items" => Answer::new(
            &[
                "collection",
                "link_id",
                "seq",
                "object_hash",
                "conflict_object",
            ],
            reader
                .list_item_conflicts(at.text("account").as_deref())?
                .into_iter()
                .map(|conflict| {
                    vec![
                        Value::Text(conflict.collection),
                        Value::Text(conflict.link_id.0),
                        Value::Integer(conflict.seq),
                        hash(conflict.object.as_ref()),
                        hash(conflict.conflict_object.as_ref()),
                    ]
                })
                .collect(),
        ),
        "list_conflicted_bindings" => Answer::new(
            &[
                "collection",
                "link_id",
                "source",
                "handle",
                "conflict_revision",
                "base_object",
                "object_hash",
                "conflict_object",
            ],
            reader
                .list_conflicts(at.text("account").as_deref())?
                .into_iter()
                .map(|conflict| {
                    vec![
                        Value::Text(conflict.collection),
                        Value::Text(conflict.link_id.0),
                        Value::Text(conflict.source.0),
                        Value::Text(conflict.handle.0),
                        text(conflict.conflict_revision.as_deref()),
                        hash(conflict.base_object.as_ref()),
                        hash(conflict.object.as_ref()),
                        hash(conflict.conflict_object.as_ref()),
                    ]
                })
                .collect(),
        ),
        "list_refusals" => Answer::new(
            &[
                "seq",
                "source",
                "handle",
                "refusal",
                "refused_key",
                "refused_at",
            ],
            reader
                .refusals(at.string("collection"))?
                .into_iter()
                .map(|refusal| {
                    vec![
                        Value::Integer(refusal.seq),
                        Value::Text(refusal.source),
                        Value::Text(refusal.handle),
                        Value::Text(refusal.reason),
                        Value::Text(refusal.key),
                        Value::Text(refusal.refused_at),
                    ]
                })
                .collect(),
        ),
        "load_pending_arrivals" => Answer::new(
            &[
                "id",
                "created_at",
                "producer",
                "collection",
                "action",
                "attempts",
            ],
            reader
                .pending_arrivals(at.string("collection"))?
                .into_iter()
                .map(|pending| {
                    vec![
                        Value::Integer(pending.id),
                        Value::Text(pending.created_at),
                        Value::Text(pending.producer),
                        Value::Text(pending.collection),
                        Value::Text(String::from(pending.action.kind())),
                        Value::Integer(pending.attempts),
                    ]
                })
                .collect(),
        ),
        "load_pending_actions" => Answer::new(
            &["id", "created_at", "producer", "action", "attempts"],
            reader
                .pending_actions(at.string("collection"))?
                .into_iter()
                .map(|pending| {
                    vec![
                        Value::Integer(pending.id),
                        Value::Text(pending.created_at),
                        Value::Text(pending.producer),
                        Value::Text(String::from(pending.action.kind())),
                        Value::Integer(pending.attempts),
                    ]
                })
                .collect(),
        ),
        "load_action" => {
            let id = at.int("id").unwrap();
            let row = match reader.action_status(id)? {
                PimdirActionStatus::Pending {
                    collection,
                    kind,
                    attempts,
                } => Some((collection, kind, attempts, Value::Null)),
                PimdirActionStatus::Parked {
                    collection,
                    kind,
                    attempts,
                    error,
                } => Some((collection, kind, attempts, Value::Text(error))),
                PimdirActionStatus::Unconfirmed {
                    collection,
                    kind,
                    attempts,
                    detail,
                } => Some((
                    collection,
                    kind,
                    attempts,
                    Value::Text(format!("unconfirmed: {detail}")),
                )),
                _ => None,
            };
            Answer::new(
                &["id", "collection", "action", "attempts", "error"],
                row.into_iter()
                    .map(|(collection, kind, attempts, error)| {
                        vec![
                            Value::Integer(id),
                            Value::Text(collection),
                            Value::Text(kind),
                            Value::Integer(attempts),
                            error,
                        ]
                    })
                    .collect(),
            )
        }
        "load_receipt" => {
            let rows = match reader.action_status(at.int("id").unwrap())? {
                PimdirActionStatus::Applied {
                    applied_at,
                    collection,
                    seq,
                    ..
                } => vec![vec![
                    Value::Text(applied_at),
                    text(collection.as_deref()),
                    seq.map_or(Value::Null, Value::Integer),
                ]],
                _ => Vec::new(),
            };
            Answer::new(&["applied_at", "collection", "seq"], rows)
        }
        "load_receipt_refusals" => Answer::new(
            &["collection", "source", "refusal", "refused_at"],
            reader
                .receipt_refusals(at.int("id").unwrap())?
                .into_iter()
                .map(|refusal| {
                    vec![
                        Value::Text(refusal.collection),
                        Value::Text(refusal.source),
                        Value::Text(refusal.reason),
                        Value::Text(refusal.refused_at),
                    ]
                })
                .collect(),
        ),
        "load_parked_actions" => Answer::new(
            &[
                "id",
                "created_at",
                "producer",
                "collection",
                "action",
                "payload",
                "attempts",
                "error",
            ],
            reader
                .parked_actions()?
                .into_iter()
                .map(|parked| {
                    vec![
                        Value::Integer(parked.id),
                        Value::Text(parked.created_at),
                        Value::Text(parked.producer),
                        Value::Text(parked.collection),
                        Value::Text(parked.action),
                        Value::Text(parked.payload),
                        Value::Integer(parked.attempts),
                        Value::Text(parked.error),
                    ]
                })
                .collect(),
        ),
        "list_address_placements" => {
            let after = at.text("after_key").map(|sort_key| PimdirAddressCursor {
                sort_key,
                seq: at.int("after_seq").unwrap(),
                collection: at.string("after_collection"),
                role: PimdirAddressRole::parse(&at.string("after_role")).unwrap(),
            });
            let role = at.text("role").map(|role| PimdirAddressRole::parse(&role));
            let Some(role) = role.map_or(Some(None), |role| role.map(Some)) else {
                return Ok(None);
            };
            let rows = reader
                .address_placements(&at.string("address"), role, after.as_ref(), at.limit())?
                .into_iter()
                .map(|placement| {
                    vec![
                        Value::Text(String::from(placement.role.as_str())),
                        Value::Text(placement.collection),
                        text(placement.account.as_deref()),
                        Value::Text(placement.kind),
                        Value::Integer(placement.seq),
                        Value::Text(placement.sort_key),
                    ]
                })
                .collect();
            Answer::new(
                &["role", "collection", "account", "kind", "seq", "sort_key"],
                rows,
            )
        }
        "list_domain_placements" => {
            let role = at.text("role").map(|role| PimdirAddressRole::parse(&role));
            let Some(role) = role.map_or(Some(None), |role| role.map(Some)) else {
                return Ok(None);
            };
            let rows = reader
                .domain_placements(&at.string("domain"), role)?
                .into_iter()
                .map(|placement| {
                    vec![
                        Value::Text(placement.address),
                        Value::Text(String::from(placement.role.as_str())),
                        Value::Text(placement.collection),
                        text(placement.account.as_deref()),
                        Value::Text(placement.kind),
                        Value::Integer(placement.seq),
                        Value::Text(placement.sort_key),
                    ]
                })
                .collect();
            Answer::new(
                &[
                    "address",
                    "role",
                    "collection",
                    "account",
                    "kind",
                    "seq",
                    "sort_key",
                ],
                rows,
            )
        }
        "load_capabilities" | "load_item_capabilities" => {
            let sources = match name {
                "load_capabilities" => reader.capabilities(at.string("collection"))?,
                _ => reader.item_capabilities(at.string("collection"), at.int("seq").unwrap())?,
            };
            let rows = sources
                .into_iter()
                .flat_map(|source| match source.declared {
                    None => vec![vec![
                        Value::Text(source.source),
                        Value::Null,
                        Value::Null,
                        Value::Null,
                    ]],
                    Some(declared) => declared
                        .into_iter()
                        .map(|(capability, (support, detail))| {
                            vec![
                                Value::Text(source.source.clone()),
                                Value::Text(capability),
                                Value::Text(String::from(support.as_str())),
                                text(detail.as_deref()),
                            ]
                        })
                        .collect(),
                })
                .collect();
            Answer::new(&["source", "capability", "support", "detail"], rows)
        }
        "list_capability_sources" if at.text("collection").is_none() => {
            let (sources, _) =
                reader.performers(at.text("account").as_deref(), &at.string("capability"))?;
            Answer::new(
                &["source"],
                sources
                    .into_iter()
                    .map(|source| vec![Value::Text(source)])
                    .collect(),
            )
        }
        "load_performer" => {
            let (_, chosen) =
                reader.performers(at.text("account").as_deref(), &at.string("capability"))?;
            Answer::new(
                &["source"],
                chosen
                    .into_iter()
                    .map(|source| vec![Value::Text(source)])
                    .collect(),
            )
        }
        "references_from" | "references_to" => {
            let endpoint = at.endpoint();
            let references = match name {
                "references_from" => reader.references_from(&endpoint)?,
                _ => reader.references_to(&endpoint)?,
            };
            let rows = references
                .into_iter()
                .map(|reference| {
                    vec![
                        Value::Text(reference.from.kind),
                        Value::Text(reference.from.link_id.0),
                        Value::Text(reference.to.kind),
                        Value::Text(reference.to.link_id.0),
                        Value::Text(String::from(reference.role.as_str())),
                        Value::Text(String::from(reference.origin.as_str())),
                        Value::Text(reference.created_at),
                    ]
                })
                .collect();
            Answer::new(
                &[
                    "from_kind",
                    "from_link_id",
                    "to_kind",
                    "to_link_id",
                    "role",
                    "origin",
                    "created_at",
                ],
                rows,
            )
        }
        "list_attachments" => {
            let message = PimdirLinkId(at.string("link_id"));
            let rows = reader
                .list_attachments(at.text("account").as_deref(), &message)?
                .into_iter()
                .map(|attachment| {
                    let mut cells = vec![
                        Value::Text(attachment.link_id.0),
                        Value::Text(attachment.collection),
                        Value::Integer(attachment.seq),
                    ];
                    let summary = attachment.summary.map(PimdirSummary::File);
                    cells.extend(table_cells("file", summary.as_ref()).1);
                    cells.push(hash(attachment.object.as_ref()));
                    cells
                })
                .collect();
            let mut columns = vec!["link_id", "collection", "seq"];
            columns.extend(FILE);
            columns.push("object_hash");
            Answer { columns, rows }
        }
        "list_attachments_by_account" => {
            let after =
                at.text("after_key")
                    .map(|message_sort_key| PimdirAccountAttachmentCursor {
                        message_sort_key,
                        seq: at.int("after_seq").unwrap(),
                    });
            let rows = reader
                .list_attachments_by_account(
                    at.text("account").as_deref(),
                    after.as_ref(),
                    at.limit(),
                )?
                .into_iter()
                .map(|attachment| {
                    let mut cells = vec![
                        Value::Text(attachment.collection),
                        Value::Integer(attachment.seq),
                        Value::Text(attachment.link_id.0),
                    ];
                    let summary = attachment.summary.map(PimdirSummary::File);
                    cells.extend(table_cells("file", summary.as_ref()).1);
                    cells.extend([
                        Value::Text(attachment.message.0),
                        Value::Text(attachment.message_collection),
                        Value::Integer(attachment.message_seq),
                        Value::Text(attachment.message_sort_key),
                        text(attachment.sender.as_deref()),
                        text(attachment.sender_name.as_deref()),
                        text(attachment.date.as_deref()),
                    ]);
                    cells
                })
                .collect();
            let mut columns = vec!["collection", "seq", "link_id"];
            columns.extend(FILE);
            columns.extend([
                "message_link_id",
                "message_collection",
                "message_seq",
                "message_sort_key",
                "sender",
                "sender_name",
                "date",
            ]);
            Answer { columns, rows }
        }
        "list_items_changed_since" => Answer::new(
            &[
                "collection",
                "link_id",
                "seq",
                "changed",
                "deleted",
                "retained_at",
            ],
            reader
                .items_changed_since(at.int("since").unwrap(), at.limit())?
                .into_iter()
                .map(|change| {
                    vec![
                        Value::Text(change.collection),
                        Value::Text(change.link_id.0),
                        Value::Integer(change.seq),
                        Value::Integer(change.changed),
                        Value::Integer(i64::from(change.deleted)),
                        text(change.retained_at.as_deref()),
                    ]
                })
                .collect(),
        ),
        "list_collections_changed_since" => Answer::new(
            &["id", "account", "kind", "name", "changed"],
            reader
                .collections_changed_since(at.int("since").unwrap(), at.limit())?
                .into_iter()
                .map(|change| {
                    vec![
                        Value::Text(change.id),
                        text(change.account.as_deref()),
                        Value::Text(change.kind),
                        Value::Text(change.name),
                        Value::Integer(change.changed),
                    ]
                })
                .collect(),
        ),
        "load_change_cursor" => {
            let cursor = reader.change_cursor()?;
            Answer::new(
                &["changed", "purges", "store_id", "incarnation"],
                vec![vec![
                    Value::Integer(cursor.changed),
                    Value::Integer(cursor.purges),
                    Value::Text(cursor.store_id),
                    Value::Integer(cursor.incarnation),
                ]],
            )
        }
        _ => return Ok(None),
    };

    Ok(Some(answer))
}

/// An expectation's parameter point, read by name.
struct Point<'a>(&'a Expectation);

impl Point<'_> {
    fn value(&self, name: &str) -> Option<&Value> {
        self.0
            .params
            .iter()
            .find(|(param, _)| param == name)
            .map(|(_, value)| value)
    }

    fn text(&self, name: &str) -> Option<String> {
        match self.value(name) {
            Some(Value::Text(text)) => Some(text.clone()),
            _ => None,
        }
    }

    fn string(&self, name: &str) -> String {
        self.text(name)
            .unwrap_or_else(|| panic!("{} states no {name}", self.0.statement))
    }

    fn int(&self, name: &str) -> Option<i64> {
        match self.value(name) {
            Some(Value::Integer(value)) => Some(*value),
            _ => None,
        }
    }

    fn bool(&self, name: &str) -> Option<bool> {
        self.int(name).map(|value| value != 0)
    }

    fn limit(&self) -> usize {
        self.int("limit").unwrap() as usize
    }

    fn filter(&self) -> PimdirMailFilter {
        PimdirMailFilter {
            seen: self.bool("seen"),
            attachment: self.bool("attachment"),
        }
    }

    fn cursor(&self) -> Option<PimdirMailCursor> {
        self.text("after_key").map(|sort_key| PimdirMailCursor {
            sort_key,
            seq: self.int("after_seq").unwrap(),
            collection: self.string("after_collection"),
        })
    }

    fn endpoint(&self) -> PimdirEndpoint {
        PimdirEndpoint {
            kind: self.string("kind"),
            link_id: PimdirLinkId(self.string("link_id")),
        }
    }

    fn slot(&self) -> Vec<String> {
        let (_, values) = self.0.slot.as_ref().expect("a slot bound");
        values
            .iter()
            .map(|value| match value {
                Value::Text(text) => text.clone(),
                value => panic!("a slot of text holds {value:?}"),
            })
            .collect()
    }
}

/// item's summary is of another, after the leading `extra` columns.
fn summarised<'a>(
    extra: &[&'static str],
    table: &str,
    items: impl Iterator<Item = (Option<&'a str>, &'a PimdirItem)>,
) -> (Vec<&'static str>, Vec<Vec<Value>>) {
    let (names, _) = table_cells(table, None);
    let mut columns = extra.to_vec();
    columns.extend(ITEM);
    columns.extend(names);
    let rows = items
        .map(|(collection, item)| {
            let mut cells: Vec<Value> = collection
                .map(|collection| Value::Text(String::from(collection)))
                .into_iter()
                .collect();
            cells.extend(item_cells(item));
            cells.extend(table_cells(table, item.summary.as_ref()).1);
            cells
        })
        .collect();
    (columns, rows)
}

fn entries<'a>(
    extra: &[&'static str],
    table: &str,
    entries: impl Iterator<Item = &'a PimdirMailEntry>,
) -> Answer {
    let (columns, rows) = summarised(
        extra,
        table,
        entries.map(|entry| (Some(entry.collection.as_str()), &entry.item)),
    );
    Answer { columns, rows }
}

/// Every row of `collection`, live and deleted, by key, with the summary,
/// addresses and lists a reader joins on it.
fn held(
    reader: &PimdirReader,
    collection: &str,
) -> Result<BTreeMap<String, Option<PimdirSummary>>, PimdirError> {
    let mut held = BTreeMap::new();

    let mut after: Option<String> = None;
    loop {
        let page = reader.list_items(collection, after.as_deref(), 64)?;
        let Some(last) = page.last() else {
            break;
        };
        after = Some(last.link_id.0.clone());
        for item in page {
            let read = reader.get_item(collection, item.seq)?;
            held.insert(item.link_id.0, read.and_then(|read| read.summary));
        }
    }

    let mut after = None;
    loop {
        let page = reader.list_retained(collection, after, 64)?;
        let Some(last) = page.last() else {
            break;
        };
        after = Some(last.seq);
        for item in page {
            held.insert(item.link_id.0, item.summary);
        }
    }

    Ok(held)
}

/// The `summary_value` rows a summary's people, `In-Reply-To` ids and
/// categories stand for (Annex A.1, A.4, A.6), by role bytewise, then
/// position.
fn values(summary: Option<&PimdirSummary>) -> Vec<Vec<Value>> {
    let lists = match summary {
        Some(PimdirSummary::Mail(mail)) => vec![("in-reply-to", &mail.in_reply_to)],
        Some(PimdirSummary::Task(task)) => vec![("category", &task.categories)],
        _ => Vec::new(),
    };
    let mut rows = addresses(summary);
    for (role, values) in lists {
        for (position, value) in values.iter().enumerate() {
            rows.push(vec![
                Value::Text(String::from(role)),
                Value::Integer(position as i64),
                Value::Text(value.clone()),
                Value::Null,
            ]);
        }
    }
    rows.sort_by(|a, b| match (&a[0], &a[1], &b[0], &b[1]) {
        (Value::Text(ra), Value::Integer(pa), Value::Text(rb), Value::Integer(pb)) => {
            (ra, pa).cmp(&(rb, pb))
        }
        _ => Ordering::Equal,
    });
    rows
}

/// The `summary_value` rows a summary's people stand for (Annex A.6).
fn addresses(summary: Option<&PimdirSummary>) -> Vec<Vec<Value>> {
    let mut roles: Vec<(&str, Vec<&PimdirAddress>)> = Vec::new();
    match summary {
        Some(PimdirSummary::Mail(mail)) => {
            roles.push(("from", mail.from.iter().collect()));
            roles.push(("to", mail.to.iter().collect()));
            roles.push(("cc", mail.cc.iter().collect()));
            roles.push(("bcc", mail.bcc.iter().collect()));
        }
        Some(PimdirSummary::Contact(card)) => roles.push(("email", card.emails.iter().collect())),
        Some(PimdirSummary::Event(event)) => {
            roles.push(("organizer", event.organizer.iter().collect()));
            roles.push(("attendee", event.attendees.iter().collect()));
        }
        Some(PimdirSummary::Task(task)) => {
            roles.push(("organizer", task.organizer.iter().collect()));
            roles.push(("attendee", task.attendees.iter().collect()));
            roles.push(("contact", task.contacts.iter().collect()));
        }
        Some(PimdirSummary::Journal(journal)) => {
            roles.push(("organizer", journal.organizer.iter().collect()));
            roles.push(("attendee", journal.attendees.iter().collect()));
        }
        Some(PimdirSummary::File(_)) | None => {}
    }

    roles
        .into_iter()
        .flat_map(|(role, people)| {
            people
                .into_iter()
                .enumerate()
                .map(move |(position, person)| {
                    vec![
                        Value::Text(String::from(role)),
                        Value::Integer(position as i64),
                        Value::Text(person.address.clone()),
                        text(person.name.as_deref()),
                    ]
                })
        })
        .collect()
}
