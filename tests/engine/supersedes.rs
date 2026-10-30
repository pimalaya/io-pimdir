//! # An add superseding an item
//!
//! A task filed into another collection is one `add` there carrying
//! `supersedes` (STORAGE §15.3): the owner applies it with the source's
//! tombstone in one transaction, and the source's delete is held until the
//! add is pushed (SYNC §5). Neither a parked add nor a refused push may
//! cost the item it replaces, which the pair of an `add` and a `remove`
//! queued together did.

use io_pimdir::{
    client::{producer::PimdirProducer, reader::PimdirReader},
    codec::{PimdirAction, PimdirSupersedes},
    collection::PimdirCollectionId,
    placement::{PimdirFlags, PimdirHandle, PimdirLinkId},
    sync::PimdirSyncOptions,
};

use crate::common::{Client, MemRemote};

const MAIL: &str = "message/rfc822";

/// The rewritten item the add files into `archive`.
const FILED: &[u8] = b"Message-ID: <filed-a>\r\nSubject: The lease, clarified\r\n\r\nDone.\r\n";

/// A client syncing `inbox`, holding `msg-a` on its server, and an empty
/// `archive`, both synced once; answers the item's seq.
fn synced() -> (Client, i64) {
    let mut remote = MemRemote::default();
    remote.seed("inbox", "1", "msg-a", &[], b"the lease");
    let mut client = Client::new(remote);
    client.store.ensure_collection("inbox", MAIL).unwrap();
    client.store.ensure_collection("archive", MAIL).unwrap();
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    client
        .sync("archive", PimdirSyncOptions::default())
        .unwrap();
    let seq = client.store.list_items("inbox", None, 10).unwrap()[0].seq;
    (client, seq)
}

/// Writes [`FILED`] the way a producer does and queues its add into
/// `archive`, superseding the item `seq` of `inbox`.
fn file(client: &Client, seq: i64) {
    let mut producer = PimdirProducer::open(client.path(), "test").unwrap();
    let object = producer.stage_body(FILED).unwrap();
    let add = PimdirAction::Add {
        link_id: Some(PimdirLinkId::from("filed-a")),
        flags: PimdirFlags::default(),
        object: Some(object.hash.clone()),
        supersedes: Some(PimdirSupersedes {
            collection: PimdirCollectionId::from("inbox"),
            seq,
        }),
    };
    producer.enqueue("archive", &add, Some(&object)).unwrap();
}

fn on_server(client: &Client, collection: &str) -> Vec<String> {
    client
        .remote()
        .handles(collection)
        .into_iter()
        .map(|handle| handle.as_str().to_string())
        .collect()
}

fn live(client: &Client, collection: &str) -> Vec<String> {
    client
        .store
        .list_items(collection, None, 10)
        .unwrap()
        .into_iter()
        .map(|item| item.link_id.0)
        .collect()
}

#[test]
fn a_parked_add_leaves_the_source_in_place() {
    let (mut client, seq) = synced();
    // NOTE: the archive already holds the key the add stages, a producer's
    // mistake the drain parks (STORAGE §15.3).
    client
        .remote_mut()
        .seed("archive", "9", "filed-a", &[], b"an older filing");
    client
        .sync("archive", PimdirSyncOptions::default())
        .unwrap();
    file(&client, seq);

    let reader = PimdirReader::open(client.path()).unwrap().with_pending();
    assert!(
        reader.list_summaries("inbox", None, 10).unwrap().is_empty(),
        "while the add is pending, the item it supersedes reads as removed",
    );

    let report = client.store.drain().unwrap();
    assert_eq!(report.parked, 1, "{report:?}");
    assert_eq!(live(&client, "inbox"), ["msg-a"], "the source is live");
    assert_eq!(reader.list_summaries("inbox", None, 10).unwrap().len(), 1);

    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    assert_eq!(on_server(&client, "inbox"), ["1"], "and on its server");
}

#[test]
fn a_refused_push_leaves_the_source_in_place() {
    let (mut client, seq) = synced();
    file(&client, seq);
    let report = client.store.drain().unwrap();
    assert_eq!(report.parked, 0, "{report:?}");
    assert!(live(&client, "inbox").is_empty(), "tombstoned with the add");
    assert_eq!(live(&client, "archive"), ["filed-a"]);
    client
        .remote_mut()
        .refused_appends
        .insert(PimdirLinkId::from("filed-a"));

    // NOTE: the source's sync first, while the add was never pushed.
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    assert_eq!(on_server(&client, "inbox"), ["1"], "the delete is held");

    // NOTE: the target refuses the create, which stays pending.
    client
        .sync("archive", PimdirSyncOptions::default())
        .unwrap();
    assert!(on_server(&client, "archive").is_empty());
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    assert_eq!(
        on_server(&client, "inbox"),
        ["1"],
        "a refused create keeps the item it replaces on its server",
    );

    // NOTE: once the create lands, the next sync of the source deletes.
    client.remote_mut().refused_appends.clear();
    client
        .sync("archive", PimdirSyncOptions::default())
        .unwrap();
    assert_eq!(on_server(&client, "archive").len(), 1);
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    assert!(
        on_server(&client, "inbox").is_empty(),
        "the delete is pushed"
    );
    assert!(
        !client
            .storage()
            .contains("inbox", PimdirHandle::from("1").as_str())
    );
}

/// The item [`file_keeping`] files: `msg-a` again, its body rewritten.
const KEPT: &[u8] = b"Message-ID: <msg-a>\r\nSubject: The lease, clarified\r\n\r\nDone.\r\n";

/// Queues an add of [`KEPT`] into `archive` under the identity of the item
/// `seq` of `inbox`, which it supersedes: a task filed with its `UID` kept.
fn file_keeping(client: &Client, seq: i64) {
    let mut producer = PimdirProducer::open(client.path(), "test").unwrap();
    let object = producer.stage_body(KEPT).unwrap();
    let add = PimdirAction::Add {
        link_id: Some(PimdirLinkId::from("msg-a")),
        flags: PimdirFlags::default(),
        object: Some(object.hash.clone()),
        supersedes: Some(PimdirSupersedes {
            collection: PimdirCollectionId::from("inbox"),
            seq,
        }),
    };
    producer.enqueue("archive", &add, Some(&object)).unwrap();
}

#[test]
fn a_filing_keeping_its_identity_is_moved_on_a_server_keeping_one() {
    // NOTE: Fastmail's shape: an append whose identity another collection
    // holds is refused, a move is taken.
    let (mut client, seq) = synced();
    client.remote_mut().one_identity = true;
    client.remote_mut().moves = true;
    file_keeping(&client, seq);
    client.store.drain().unwrap();

    client
        .sync("archive", PimdirSyncOptions::default())
        .unwrap();
    assert_eq!(
        on_server(&client, "archive"),
        ["1"],
        "moved, its handle kept"
    );
    assert!(
        on_server(&client, "inbox").is_empty(),
        "never in two places"
    );

    // NOTE: the held delete released, it finds the member gone and passes.
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    assert!(on_server(&client, "inbox").is_empty());
    assert!(
        !client
            .storage()
            .contains("inbox", PimdirHandle::from("1").as_str()),
        "the source's binding is gone",
    );
    client
        .sync("archive", PimdirSyncOptions::default())
        .unwrap();
    assert_eq!(live(&client, "archive"), ["msg-a"]);
    assert_eq!(on_server(&client, "archive"), ["1"], "and stays put");
}

#[test]
fn a_filing_keeping_its_identity_without_a_move_keeps_its_source() {
    let (mut client, seq) = synced();
    client.remote_mut().one_identity = true;
    file_keeping(&client, seq);
    client.store.drain().unwrap();

    client
        .sync("archive", PimdirSyncOptions::default())
        .unwrap();
    client.sync("inbox", PimdirSyncOptions::default()).unwrap();
    assert!(
        on_server(&client, "archive").is_empty(),
        "the create refused"
    );
    assert_eq!(on_server(&client, "inbox"), ["1"], "the source kept");
}
