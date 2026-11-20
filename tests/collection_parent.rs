//! Where a source places a collection and how it shows it (STORAGE §14).
//!
//! `collections.parent` keeps an account's hierarchy as a forest, and the
//! presentation columns (`color`, `description`, `sort_order`) are what the
//! source states, which `list_collections` reads and orders by.

#![cfg(feature = "client")]

use io_pimdir::client::{PimdirError, PimdirStore, reader::PimdirReader};

fn store(dir: &tempfile::TempDir) -> PimdirStore {
    let store = PimdirStore::open(dir.path()).unwrap().for_account("work");
    for id in ["INBOX", "Projects", "Q1"] {
        store.ensure_collection(id, "message/rfc822").unwrap();
    }
    PimdirStore::open(dir.path())
        .unwrap()
        .for_account("home")
        .ensure_collection("Other", "message/rfc822")
        .unwrap();
    store
}

fn parent_of(reader: &PimdirReader, id: &str) -> Option<String> {
    reader
        .list_collections()
        .unwrap()
        .into_iter()
        .find(|c| c.id == id)
        .unwrap()
        .parent
}

#[test]
fn a_parent_keeps_the_hierarchy_a_forest_within_an_account() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let reader = PimdirReader::open(dir.path()).unwrap();

    store
        .set_collection_parent("Projects", Some("INBOX"))
        .unwrap();
    store.set_collection_parent("Q1", Some("Projects")).unwrap();
    assert_eq!(parent_of(&reader, "Q1").as_deref(), Some("Projects"));

    let cycle = store
        .set_collection_parent("INBOX", Some("Q1"))
        .unwrap_err();
    assert!(matches!(cycle, PimdirError::Parent { .. }), "{cycle:?}");
    assert_eq!(cycle.code(), "collection-parent");
    let itself = store
        .set_collection_parent("INBOX", Some("INBOX"))
        .unwrap_err();
    assert!(matches!(itself, PimdirError::Parent { .. }), "{itself:?}");
    let across = store
        .set_collection_parent("Q1", Some("Other"))
        .unwrap_err();
    assert!(matches!(across, PimdirError::Parent { .. }), "{across:?}");
    assert_eq!(
        parent_of(&reader, "INBOX"),
        None,
        "the refusals wrote nothing"
    );
    assert_eq!(parent_of(&reader, "Q1").as_deref(), Some("Projects"));

    let unknown = store
        .set_collection_parent("Q1", Some("Nowhere"))
        .unwrap_err();
    assert!(
        matches!(&unknown, PimdirError::Undeclared { collection } if collection == "Nowhere"),
        "{unknown:?}"
    );
    assert_eq!(unknown.code(), "collection-undeclared");
    let undeclared = store.set_collection_parent("Nowhere", None).unwrap_err();
    assert!(matches!(undeclared, PimdirError::Undeclared { .. }));

    store.set_collection_parent("Q1", None).unwrap();
    assert_eq!(parent_of(&reader, "Q1"), None, "back at the top");
}

#[test]
fn a_presentation_is_written_whole_and_orders_the_listing() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let reader = PimdirReader::open(dir.path()).unwrap();

    store
        .set_collection_presentation("Projects", Some("#3a87ad"), Some("Work in flight"), Some(2))
        .unwrap();
    store
        .set_collection_presentation("Q1", None, None, Some(1))
        .unwrap();

    let listed: Vec<_> = reader
        .list_collections_by_account(Some("work"))
        .unwrap()
        .into_iter()
        .map(|c| (c.id, c.color, c.description, c.sort_order))
        .collect();
    assert_eq!(
        listed,
        [
            ("Q1".into(), None, None, Some(1)),
            (
                "Projects".into(),
                Some("#3a87ad".into()),
                Some("Work in flight".into()),
                Some(2)
            ),
            ("INBOX".into(), None, None, None),
        ]
    );

    store
        .set_collection_presentation("Projects", None, None, None)
        .unwrap();
    let projects = reader
        .list_collections()
        .unwrap()
        .into_iter()
        .find(|c| c.id == "Projects")
        .unwrap();
    assert_eq!(
        (projects.color, projects.description, projects.sort_order),
        (None, None, None),
        "a source stating none clears it"
    );

    let undeclared = store
        .set_collection_presentation("Nowhere", Some("red"), None, None)
        .unwrap_err();
    assert!(matches!(undeclared, PimdirError::Undeclared { .. }));
}
