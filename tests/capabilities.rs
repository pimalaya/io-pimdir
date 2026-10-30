//! Capabilities (STORAGE §15.6): an undeclared source gates nothing, a
//! declared one refuses at the enqueue what it lacks, an intent needs one
//! performer and the user's choice among several, the owner parks what
//! slipped past the gate, and a store from 0.5 gains the tables on open.

use std::path::Path;

use io_pimdir::{
    capability::{self, PimdirCapability, PimdirSupport},
    change::PimdirWriteOp,
    client::{PimdirError, PimdirSourceStore, PimdirStore, producer::PimdirProducer},
    codec::{PimdirAction, PimdirActionError},
    collection::PimdirCollectionId,
    intent::{self, PimdirCollectionCreate, PimdirIntentItem, PimdirInvitation, PimdirPartstat},
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
};
use rusqlite::Connection;

/// A one-message INBOX bound by source `graph`, plus an Archive.
fn seeded(dir: &Path) -> (PimdirSourceStore, i64) {
    let mut store = PimdirStore::open(dir).unwrap().for_source("graph");
    store.ensure_collection("INBOX", "message/rfc822").unwrap();
    store
        .ensure_collection("Archive", "message/rfc822")
        .unwrap();
    let flags = PimdirFlags::from_iter(["\\Seen"]);
    store
        .write(vec![
            PimdirWriteOp::StoreObject {
                object: PimdirObject {
                    hash: PimdirHash("cafebabe".into()),
                    size: 3,
                },
                body: Some(b"abc".to_vec()),
            },
            PimdirWriteOp::UpsertPlacement(PimdirPlacement {
                sort_key: Default::default(),
                collection: PimdirCollectionId("INBOX".into()),
                handle: PimdirHandle("1".into()),
                link_id: Some(PimdirLinkId("mid:a".into())),
                object: Some(PimdirHash("cafebabe".into())),
                level: PimdirLevel::Full,
                summary: None,
                flags: flags.clone(),
                status: PimdirStatus::Clean,
                conflict_revision: None,
                conflict_object: None,
                base: Some(PimdirBase {
                    flags,
                    revision: None,
                    object: Some(PimdirHash("cafebabe".into())),
                }),
                origin: None,
            }),
        ])
        .unwrap();
    let seq = store.list_items("INBOX", None, 10).unwrap()[0].seq;
    (store, seq)
}

/// A mail declaration: every capability `full`, those named `none`.
fn mail(none: &[&str]) -> Vec<PimdirCapability> {
    declaration(capability::MAIL, none)
}

/// A calendar declaration: every capability `full`, those named `none`.
fn calendar(none: &[&str]) -> Vec<PimdirCapability> {
    declaration(capability::CALENDAR, none)
}

fn declaration(names: &[&str], none: &[&str]) -> Vec<PimdirCapability> {
    names
        .iter()
        .map(|name| PimdirCapability {
            collection: None,
            name: name.to_string(),
            support: match none.contains(name) {
                true => PimdirSupport::None,
                false => PimdirSupport::Full,
            },
            detail: none.contains(name).then(|| "pull-only".to_string()),
        })
        .collect()
}

fn archive(seq: i64) -> PimdirAction {
    PimdirAction::Move {
        seq,
        to: PimdirCollectionId("Archive".into()),
    }
}

fn submit(source: Option<&str>) -> PimdirAction {
    let source = source
        .map(|s| format!(",\"source\":\"{s}\""))
        .unwrap_or_default();
    PimdirAction::Unknown {
        kind: "submit".into(),
        payload: format!("{{\"v\":1,\"from\":\"a@b\",\"rcpts\":[\"c@d\"]{source}}}"),
        object_hash: None,
    }
}

#[test]
fn an_undeclared_source_gates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (_store, seq) = seeded(dir.path());

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer.enqueue("INBOX", &archive(seq), None).unwrap();
}

#[test]
fn a_declared_source_refuses_a_move_it_cannot_push() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());
    store
        .declare("graph", &mail(&[capability::MAIL_MESSAGE_MOVE]))
        .unwrap();

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let Err(PimdirError::Unsupported(refusal)) = producer.enqueue("INBOX", &archive(seq), None)
    else {
        panic!("the move should be refused");
    };
    assert_eq!(refusal.capability, capability::MAIL_MESSAGE_MOVE);
    assert_eq!(refusal.source, "graph");
    assert_eq!(refusal.detail.as_deref(), Some("pull-only"));
    assert!(producer.pending_actions("INBOX").unwrap().is_empty());

    let flag = PimdirAction::SetFlags {
        seq,
        flags: PimdirFlags::from_iter(["\\Seen", "\\Flagged"]),
    };
    producer.enqueue("INBOX", &flag, None).unwrap();
}

#[test]
fn the_owner_parks_a_move_that_slipped_past_the_gate() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, seq) = seeded(dir.path());

    // enqueued while the source was undeclared, then declared without move
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer.enqueue("INBOX", &archive(seq), None).unwrap();
    drop(producer);
    store
        .declare("graph", &mail(&[capability::MAIL_MESSAGE_MOVE]))
        .unwrap();

    let report = store.drain().unwrap();
    assert_eq!(report.parked, 1);
    let parked = store.parked_actions().unwrap();
    assert!(parked[0].error.contains(capability::MAIL_MESSAGE_MOVE));
}

#[test]
fn an_intent_needs_the_user_to_choose_among_several_performers() {
    let dir = tempfile::tempdir().unwrap();
    // NOTE: `smtp` syncs nothing yet, and is a candidate all the same.
    let (mut store, _) = seeded(dir.path());
    store.declare("graph", &mail(&[])).unwrap();
    store.declare("smtp", &mail(&[])).unwrap();

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let Err(PimdirError::Ambiguous { candidates, .. }) =
        producer.enqueue("INBOX", &submit(None), None)
    else {
        panic!("two senders should be ambiguous");
    };
    assert_eq!(candidates, ["graph", "smtp"]);
    producer
        .enqueue("INBOX", &submit(Some("graph")), None)
        .unwrap();
    let Err(PimdirError::Unsupported(_)) = producer.enqueue("INBOX", &submit(Some("nope")), None)
    else {
        panic!("a source declaring nothing cannot perform it");
    };

    let choose = PimdirAction::SetPerformer {
        capability: capability::MAIL_SUBMIT.into(),
        source: Some("smtp".into()),
    };
    producer.enqueue("INBOX", &choose, None).unwrap();

    // the choice holds while still queued, before the owner applies it
    assert_eq!(
        producer
            .performer("INBOX", capability::MAIL_SUBMIT, None)
            .unwrap(),
        "smtp"
    );
    producer.enqueue("INBOX", &submit(None), None).unwrap();
    drop(producer);
    assert_eq!(store.drain().unwrap().applied, 1);

    let producer = PimdirProducer::open(dir.path(), "test").unwrap();
    assert_eq!(
        producer
            .performer("INBOX", capability::MAIL_SUBMIT, None)
            .unwrap(),
        "smtp"
    );
}

#[test]
fn a_store_from_an_older_owner_gains_the_tables_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let (store, seq) = seeded(dir.path());
    drop(store);
    Connection::open(dir.path().join("pimdir.db"))
        .unwrap()
        .execute_batch("DROP TABLE capabilities; DROP TABLE performers;")
        .unwrap();

    // a producer reads the missing tables as nothing declared
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    producer.enqueue("INBOX", &archive(seq), None).unwrap();
    drop(producer);

    let mut store = PimdirStore::open(dir.path()).unwrap();
    store
        .declare("graph", &mail(&[capability::MAIL_MESSAGE_MOVE]))
        .unwrap();
}

const MEETING: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:m1\r\nDTSTART:20261010T090000Z\r\nORGANIZER:mailto:a@b.org\r\nATTENDEE:mailto:c@d.org\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

/// One event in `collection`, bound by `source`, returning its seq.
fn seeded_event(dir: &Path, source: &str, collection: &str, uid: &str) -> i64 {
    let mut store = PimdirStore::open(dir).unwrap().for_source(source);
    store
        .ensure_collection(collection, "text/calendar")
        .unwrap();
    let body = MEETING.replace("UID:m1", &format!("UID:{uid}"));
    let hash = io_pimdir::hash::PimdirHashAlgo::default().hash(body.as_bytes());
    store
        .write(vec![
            PimdirWriteOp::StoreObject {
                object: PimdirObject {
                    hash: hash.clone(),
                    size: body.len(),
                },
                body: Some(body.into_bytes()),
            },
            PimdirWriteOp::UpsertPlacement(PimdirPlacement {
                sort_key: Default::default(),
                collection: PimdirCollectionId(collection.into()),
                handle: PimdirHandle(uid.into()),
                link_id: Some(PimdirLinkId(uid.into())),
                object: Some(hash.clone()),
                level: PimdirLevel::Full,
                summary: None,
                flags: PimdirFlags::default(),
                status: PimdirStatus::Clean,
                conflict_revision: None,
                conflict_object: None,
                base: Some(PimdirBase {
                    flags: PimdirFlags::default(),
                    revision: None,
                    object: Some(hash),
                }),
                origin: None,
            }),
        ])
        .unwrap();
    store.list_items(collection, None, 10).unwrap()[0].seq
}

/// Writes `body` into the blob store the way a producer does before its
/// enqueue, and the `add` carrying it.
fn add(producer: &PimdirProducer, body: &str) -> (PimdirAction, PimdirObject) {
    let object = producer.stage_body(body.as_bytes()).unwrap();
    let action = PimdirAction::Add {
        link_id: None,
        flags: PimdirFlags::default(),
        object: Some(object.hash.clone()),
        supersedes: None,
    };
    (action, object)
}

#[test]
fn a_scheduled_event_needs_a_source_that_notifies() {
    let dir = tempfile::tempdir().unwrap();
    let seq = seeded_event(dir.path(), "dav", "work", "m0");
    let mut store = PimdirStore::open(dir.path()).unwrap();
    store
        .declare("dav", &calendar(&[capability::CALENDAR_SCHEDULING]))
        .unwrap();
    drop(store);

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let (action, object) = add(&producer, MEETING);
    let Err(PimdirError::Unsupported(refusal)) = producer.enqueue("work", &action, Some(&object))
    else {
        panic!("a scheduled event should be refused");
    };
    assert_eq!(refusal.capability, capability::CALENDAR_SCHEDULING);

    // the user accepted that nobody is notified
    let unscheduled = MEETING
        .replace("ORGANIZER:", "ORGANIZER;SCHEDULE-AGENT=NONE:")
        .replace("ATTENDEE:", "ATTENDEE;SCHEDULE-AGENT=NONE:");
    let (action, object) = add(&producer, &unscheduled);
    producer.enqueue("work", &action, Some(&object)).unwrap();

    let Err(PimdirError::Unsupported(_)) =
        producer.enqueue("work", &PimdirAction::Remove { seq }, None)
    else {
        panic!("removing a scheduled event should be refused");
    };
}

#[test]
fn an_online_meeting_is_asked_in_the_resource() {
    let dir = tempfile::tempdir().unwrap();
    seeded_event(dir.path(), "dav", "work", "m0");
    let mut store = PimdirStore::open(dir.path()).unwrap();
    store
        .declare("dav", &calendar(&[capability::CALENDAR_ONLINE_MEETING]))
        .unwrap();
    drop(store);

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let online = MEETING.replace("END:VEVENT", "X-PIMDIR-ONLINE-MEETING:TRUE\r\nEND:VEVENT");
    let (action, object) = add(&producer, &online);
    let Err(PimdirError::Unsupported(refusal)) = producer.enqueue("work", &action, Some(&object))
    else {
        panic!("an online meeting should be refused");
    };
    assert_eq!(refusal.capability, capability::CALENDAR_ONLINE_MEETING);
}

#[test]
fn a_native_reply_where_it_holds_and_an_imip_one_anywhere_are_both_candidates() {
    let dir = tempfile::tempdir().unwrap();
    let work = seeded_event(dir.path(), "dav", "work", "m0");
    let home = seeded_event(dir.path(), "google", "home", "m1");
    let mut store = PimdirStore::open(dir.path()).unwrap();
    store
        .declare("dav", &calendar(&[capability::CALENDAR_REPLY]))
        .unwrap();
    // the provider's own verb reaches only the calendar it holds
    let mut google = calendar(&[capability::CALENDAR_REPLY]);
    google.push(PimdirCapability {
        collection: Some("home".into()),
        name: capability::CALENDAR_REPLY.into(),
        support: PimdirSupport::Full,
        detail: None,
    });
    store.declare("google", &google).unwrap();
    let imip = PimdirCapability {
        collection: None,
        name: capability::CALENDAR_REPLY.into(),
        support: PimdirSupport::Partial,
        detail: Some("by iMIP".into()),
    };
    store.declare("smtp", &[imip]).unwrap();
    drop(store);

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    assert_eq!(
        producer
            .performer("work", capability::CALENDAR_REPLY, None)
            .unwrap(),
        "smtp"
    );
    let Err(PimdirError::Ambiguous { candidates, .. }) =
        producer.performer("home", capability::CALENDAR_REPLY, None)
    else {
        panic!("a native and an iMIP reply should be the user's choice");
    };
    assert_eq!(candidates, ["google", "smtp"]);

    let reply = |source: &str, seq: i64| PimdirAction::Unknown {
        kind: "calendar-reply".into(),
        payload: format!(
            "{{\"v\":1,\"source\":\"{source}\",\"seq\":{seq},\"partstat\":\"ACCEPTED\"}}"
        ),
        object_hash: None,
    };
    let Err(PimdirError::Unsupported(_)) = producer.enqueue("work", &reply("google", work), None)
    else {
        panic!("the native reply does not reach a calendar it does not hold");
    };
    producer
        .enqueue("home", &reply("google", home), None)
        .unwrap();
}

#[test]
fn a_copy_is_asked_of_the_sender_and_filed_once_sent() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, _) = seeded(dir.path());
    store
        .declare("graph", &mail(&[capability::MAIL_SUBMIT_COPY]))
        .unwrap();
    drop(store);

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let raw = "From: a@b\r\nTo: c@d\r\nSubject: hi\r\n\r\nhi\r\n";
    let (_, object) = add(&producer, raw);
    let submit = |copy: Option<&str>| PimdirAction::Unknown {
        kind: "submit".into(),
        payload: match copy {
            Some(copy) => format!(
                "{{\"v\":1,\"source\":\"graph\",\"from\":\"a@b\",\"rcpts\":[\"c@d\"],\"copy\":\"{copy}\"}}"
            ),
            None => "{\"v\":1,\"source\":\"graph\",\"from\":\"a@b\",\"rcpts\":[\"c@d\"]}".into(),
        },
        object_hash: Some(object.hash.clone()),
    };
    let Err(PimdirError::Unsupported(refusal)) =
        producer.enqueue("INBOX", &submit(Some("Archive")), Some(&object))
    else {
        panic!("a sender that cannot file the copy should be refused");
    };
    assert_eq!(refusal.capability, capability::MAIL_SUBMIT_COPY);
    let id = producer
        .enqueue("INBOX", &submit(None), Some(&object))
        .unwrap();
    drop(producer);

    // the send performed, the intent gives way to the copy it asked for
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("graph");
    let copy = PimdirAction::Add {
        link_id: None,
        flags: PimdirFlags::from_iter(["\\Seen"]),
        object: Some(object.hash.clone()),
        supersedes: None,
    };
    assert!(store.replace_action(id, "owner", "Archive", &copy).unwrap());
    assert!(!store.replace_action(id, "owner", "Archive", &copy).unwrap());
    let queued = store.list_pending_actions().unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].action.kind(), "add");

    assert_eq!(store.drain().unwrap().applied, 1);
    assert_eq!(store.list_items("Archive", None, 10).unwrap().len(), 1);
}

#[test]
fn a_collection_is_created_by_a_source_that_declares_it_under_its_anchor() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, _) = seeded(dir.path());
    // the provider reserves children under its Archive
    let mut imap = mail(&[]);
    imap.push(PimdirCapability {
        collection: Some("Archive".into()),
        name: capability::COLLECTION_CREATE.into(),
        support: PimdirSupport::None,
        detail: Some("\\Noinferiors".into()),
    });
    store.declare("graph", &imap).unwrap();
    drop(store);

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let id = producer
        .enqueue_collection_create("INBOX", "Projects", None, None)
        .unwrap();
    let pending = producer.pending_actions("INBOX").unwrap();
    let PimdirAction::Unknown { kind, payload, .. } = &pending[0].action else {
        panic!("an intent travels as an unknown kind");
    };
    assert_eq!(pending[0].id, id);
    assert_eq!(kind, intent::COLLECTION_CREATE);
    assert_eq!(
        PimdirCollectionCreate::from_payload(payload).unwrap(),
        PimdirCollectionCreate {
            source: Some("graph".into()),
            name: "Projects".into(),
            parent: None,
        }
    );

    // anchored on its parent, whose own row refuses children there
    let Err(PimdirError::NoPerformer { capability }) =
        producer.enqueue_collection_create("INBOX", "2026", Some("Archive"), None)
    else {
        panic!("a parent refusing children has no performer");
    };
    assert_eq!(capability, capability::COLLECTION_CREATE);

    // a payload anchored elsewhere than its parent, or nameless, is refused
    let create = |payload: &str| PimdirAction::Unknown {
        kind: intent::COLLECTION_CREATE.into(),
        payload: payload.into(),
        object_hash: None,
    };
    let Err(PimdirError::Action(PimdirActionError::Invalid {
        field: "parent", ..
    })) = producer.enqueue(
        "INBOX",
        &create(r#"{"v":1,"source":"graph","name":"a","parent":"Archive"}"#),
        None,
    )
    else {
        panic!("a parent is the anchor");
    };
    let Err(PimdirError::Action(PimdirActionError::MissingField("name"))) =
        producer.enqueue("INBOX", &create(r#"{"v":1,"source":"graph"}"#), None)
    else {
        panic!("a collection needs a name");
    };
    let Err(PimdirError::Action(PimdirActionError::Invalid { .. })) = producer.enqueue(
        "Nowhere",
        &create(r#"{"v":1,"source":"graph","name":"a"}"#),
        None,
    ) else {
        panic!("the anchor is a collection of a declared kind");
    };
}

#[test]
fn a_collection_create_needs_a_declared_source() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = seeded(dir.path());
    drop(store);

    // an owner declaring nothing predates the intent and would skip it
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let Err(PimdirError::NoPerformer { .. }) =
        producer.enqueue_collection_create("INBOX", "Projects", None, None)
    else {
        panic!("nobody performs a collection-create");
    };

    let mut store = PimdirStore::open(dir.path()).unwrap();
    store
        .declare("graph", &mail(&[capability::COLLECTION_CREATE]))
        .unwrap();
    drop(store);
    let Err(PimdirError::NoPerformer { .. }) =
        producer.enqueue_collection_create("INBOX", "Projects", None, None)
    else {
        panic!("a source declaring none performs nothing");
    };
}

#[test]
fn an_occurrence_is_answered_only_by_a_performer_declaring_it() {
    let dir = tempfile::tempdir().unwrap();
    let seq = seeded_event(dir.path(), "dav", "work", "m0");
    let invitation = |kind: &str, recurrence_id: Option<&str>| {
        PimdirInvitation {
            source: Some("dav".into()),
            item: PimdirIntentItem::Seq(seq),
            partstat: (kind == intent::CALENDAR_REPLY).then_some(PimdirPartstat::Accepted),
            comment: None,
            recurrence_id: recurrence_id.map(String::from),
        }
        .to_action()
        .unwrap()
    };

    // an owner declaring nothing would widen the occurrence to the series
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let Err(PimdirError::Unsupported(refusal)) = producer.enqueue(
        "work",
        &invitation(intent::CALENDAR_CANCEL, Some("20261010T090000Z")),
        None,
    ) else {
        panic!("an undeclared owner gets no occurrence");
    };
    assert_eq!(refusal.capability, capability::CALENDAR_CANCEL_OCCURRENCE);
    producer
        .enqueue("work", &invitation(intent::CALENDAR_CANCEL, None), None)
        .unwrap();

    let mut store = PimdirStore::open(dir.path()).unwrap();
    store
        .declare("dav", &calendar(&[capability::CALENDAR_REPLY_OCCURRENCE]))
        .unwrap();
    drop(store);

    let Err(PimdirError::Unsupported(refusal)) = producer.enqueue(
        "work",
        &invitation(intent::CALENDAR_REPLY, Some("20261010T090000Z")),
        None,
    ) else {
        panic!("a performer not declaring the occurrence is refused it");
    };
    assert_eq!(refusal.capability, capability::CALENDAR_REPLY_OCCURRENCE);
    assert_eq!(refusal.source, "dav");
    producer
        .enqueue("work", &invitation(intent::CALENDAR_REPLY, None), None)
        .unwrap();
    producer
        .enqueue(
            "work",
            &invitation(intent::CALENDAR_CANCEL, Some("20261010T090000Z")),
            None,
        )
        .unwrap();

    let malformed = PimdirAction::Unknown {
        kind: intent::CALENDAR_CANCEL.into(),
        payload: format!(r#"{{"v":1,"source":"dav","seq":{seq},"recurrence_id":"tomorrow"}}"#),
        object_hash: None,
    };
    let Err(PimdirError::Action(PimdirActionError::Invalid {
        field: "recurrence_id",
        ..
    })) = producer.enqueue("work", &malformed, None)
    else {
        panic!("a malformed occurrence is refused");
    };
}

const TODO: &str = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t1\r\nDTSTAMP:20261028T090000Z\r\nSUMMARY:Call Ann\r\nCATEGORIES:inbox\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
const EVENT: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:e9\r\nDTSTART:20261010T090000Z\r\nSUMMARY:Lunch\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

/// Source caldav syncing Tasks, holding task t0, and Events, declared with
/// every calendar capability and `x-vtodo` `none` on Events alone.
fn events_only(dir: &Path) -> i64 {
    seeded_event(dir, "caldav", "Events", "e0");
    let mut store = PimdirStore::open(dir).unwrap().for_source("caldav");
    store.ensure_collection("Tasks", "text/calendar").unwrap();
    let body = TODO.replace("UID:t1", "UID:t0");
    let hash = io_pimdir::hash::PimdirHashAlgo::default().hash(body.as_bytes());
    store
        .write(vec![
            PimdirWriteOp::StoreObject {
                object: PimdirObject {
                    hash: hash.clone(),
                    size: body.len(),
                },
                body: Some(body.into_bytes()),
            },
            PimdirWriteOp::UpsertPlacement(PimdirPlacement {
                sort_key: Default::default(),
                collection: PimdirCollectionId("Tasks".into()),
                handle: PimdirHandle("t0".into()),
                link_id: Some(PimdirLinkId("t0".into())),
                object: Some(hash.clone()),
                level: PimdirLevel::Full,
                summary: None,
                flags: PimdirFlags::default(),
                status: PimdirStatus::Clean,
                conflict_revision: None,
                conflict_object: None,
                base: Some(PimdirBase {
                    flags: PimdirFlags::default(),
                    revision: None,
                    object: Some(hash),
                }),
                origin: None,
            }),
        ])
        .unwrap();
    store.list_items("Tasks", None, 10).unwrap()[0].seq
}

fn refuse_tasks(store: &mut PimdirStore) {
    let mut rows = calendar(&[]);
    rows.push(PimdirCapability {
        collection: Some("Events".into()),
        name: capability::X_VTODO.into(),
        support: PimdirSupport::None,
        detail: Some("events only".into()),
    });
    store.declare("caldav", &rows).unwrap();
}

#[test]
fn a_calendar_refusing_tasks_refuses_a_task_and_takes_an_event() {
    let dir = tempfile::tempdir().unwrap();
    let task = events_only(dir.path());
    let mut store = PimdirStore::open(dir.path()).unwrap();
    refuse_tasks(&mut store);
    drop(store);

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let (action, object) = add(&producer, TODO);
    let Err(PimdirError::Unsupported(refusal)) = producer.enqueue("Events", &action, Some(&object))
    else {
        panic!("a task into an events-only calendar should be refused");
    };
    assert_eq!(refusal.capability, capability::X_VTODO);
    assert_eq!(refusal.source, "caldav");
    assert_eq!(refusal.detail.as_deref(), Some("events only"));

    let (action, object) = add(&producer, EVENT);
    producer.enqueue("Events", &action, Some(&object)).unwrap();
    let (action, object) = add(&producer, TODO);
    producer.enqueue("Tasks", &action, Some(&object)).unwrap();

    let into_events = PimdirAction::Move {
        seq: task,
        to: PimdirCollectionId("Events".into()),
    };
    let Err(PimdirError::Unsupported(refusal)) = producer.enqueue("Tasks", &into_events, None)
    else {
        panic!("moving a task into an events-only calendar should be refused");
    };
    assert_eq!(refusal.capability, capability::X_VTODO);

    let reader = io_pimdir::client::reader::PimdirReader::open(dir.path()).unwrap();
    assert!(!reader.accepts_vtodo("Events").unwrap());
    assert!(reader.accepts_vtodo("Tasks").unwrap());
}

#[test]
fn the_owner_parks_a_task_into_a_calendar_refusing_tasks() {
    let dir = tempfile::tempdir().unwrap();
    events_only(dir.path());

    // NOTE: enqueued while the source was undeclared, then declared.
    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let (action, object) = add(&producer, TODO);
    producer.enqueue("Events", &action, Some(&object)).unwrap();
    drop(producer);
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("caldav");
    refuse_tasks(&mut store);

    let report = store.drain().unwrap();
    assert_eq!(report.parked, 1);
    assert!(
        store.parked_actions().unwrap()[0]
            .error
            .contains(capability::X_VTODO)
    );
}

#[test]
fn a_source_stating_no_x_vtodo_row_refuses_no_task() {
    let dir = tempfile::tempdir().unwrap();
    events_only(dir.path());
    let mut store = PimdirStore::open(dir.path()).unwrap();
    store.declare("caldav", &calendar(&[])).unwrap();
    drop(store);

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let (action, object) = add(&producer, TODO);
    producer.enqueue("Events", &action, Some(&object)).unwrap();
}

#[test]
fn a_superseding_add_needs_the_remove_of_what_it_supersedes() {
    let dir = tempfile::tempdir().unwrap();
    let task = events_only(dir.path());
    let mut store = PimdirStore::open(dir.path()).unwrap();
    store
        .declare("caldav", &calendar(&[capability::CALENDAR_ITEM_REMOVE]))
        .unwrap();
    drop(store);

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let (action, object) = add(&producer, TODO);
    producer.enqueue("Tasks", &action, Some(&object)).unwrap();
    let PimdirAction::Add {
        link_id,
        flags,
        object: hash,
        ..
    } = action
    else {
        unreachable!();
    };
    let superseding = PimdirAction::Add {
        link_id,
        flags,
        object: hash,
        supersedes: Some(io_pimdir::codec::PimdirSupersedes {
            collection: PimdirCollectionId("Tasks".into()),
            seq: task,
        }),
    };
    let Err(PimdirError::Unsupported(refusal)) =
        producer.enqueue("Tasks", &superseding, Some(&object))
    else {
        panic!("superseding an item its source cannot remove should be refused");
    };
    assert_eq!(refusal.capability, capability::CALENDAR_ITEM_REMOVE);
}

/// A write the gate refuses stages nothing when its body is handed in
/// (§15.1, §15.6): the gate reads the bytes in hand, where staging first
/// left the refused task's body unpinned in the tree.
#[test]
fn a_refused_create_stages_nothing() {
    let dir = tempfile::tempdir().unwrap();
    events_only(dir.path());
    let mut store = PimdirStore::open(dir.path()).unwrap();
    refuse_tasks(&mut store);
    drop(store);

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let path = producer.blobs().path(&producer.hash(TODO.as_bytes()));
    let refused = producer
        .enqueue_with_body("Events", TODO.as_bytes(), |hash| PimdirAction::Add {
            link_id: None,
            flags: PimdirFlags::default(),
            object: Some(hash),
            supersedes: None,
        })
        .unwrap_err();
    assert!(
        matches!(refused, PimdirError::Unsupported(_)),
        "{refused:?}"
    );
    assert!(!path.exists(), "the refused body never reached the tree");

    producer
        .enqueue_with_body("Tasks", TODO.as_bytes(), |hash| PimdirAction::Add {
            link_id: None,
            flags: PimdirFlags::default(),
            object: Some(hash),
            supersedes: None,
        })
        .unwrap();
    assert!(path.exists());
}

/// The one-step enqueue answers the partial supports its action passed
/// with, read with the body in hand, so a frontend warns without running
/// the gate a second time.
#[test]
fn a_checked_enqueue_answers_the_partial_supports_it_passed_with() {
    let dir = tempfile::tempdir().unwrap();
    seeded_event(dir.path(), "dav", "work", "m0");
    let mut declared = calendar(&[]);
    for capability in &mut declared {
        if capability.name == capability::CALENDAR_SCHEDULING {
            capability.support = PimdirSupport::Partial;
            capability.detail = Some("no counter-proposals".into());
        }
    }
    PimdirStore::open(dir.path())
        .unwrap()
        .declare("dav", &declared)
        .unwrap();

    let mut producer = PimdirProducer::open(dir.path(), "test").unwrap();
    let meeting = MEETING.replace("UID:m1", "UID:m2");
    let enqueued = producer
        .enqueue_with_body_checked("work", meeting.as_bytes(), |hash| PimdirAction::Add {
            link_id: None,
            flags: PimdirFlags::default(),
            object: Some(hash),
            supersedes: None,
        })
        .unwrap();
    assert_eq!(enqueued.partial.len(), 1, "{:?}", enqueued.partial);
    assert_eq!(
        enqueued.partial[0].capability,
        capability::CALENDAR_SCHEDULING
    );
    assert_eq!(enqueued.partial[0].source, "dav");
    assert_eq!(
        producer.pending_actions("work").unwrap()[0].id,
        enqueued.id,
        "the row is queued"
    );
}
