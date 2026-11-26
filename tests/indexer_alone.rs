//! The index statements without `client`: an indexer binding its own SQLite
//! (the Android app's) builds with `indexer` alone and reads the index's
//! schema, its format version and every statement by name from
//! `sql::search`, with no rusqlite in the build.

#![cfg(all(feature = "indexer", not(feature = "client")))]

use std::collections::BTreeSet;

use io_pimdir::sql::search;

#[test]
fn the_index_statements_build_without_client() {
    assert_eq!(
        search::FORMAT_VERSION,
        format!("search-v{}", search::FORMAT_MAJOR)
    );
    assert!(!search::MIGRATIONS.is_empty());
    assert!(search::MIGRATIONS.iter().all(|sql| !sql.trim().is_empty()));

    let all: Vec<_> = search::all().collect();
    assert_eq!(all.len(), search::CANONICAL.len() + search::OWN.len());
    assert!(
        all.iter()
            .all(|(name, sql)| !name.is_empty() && !sql.is_empty())
    );

    let names: BTreeSet<_> = all.iter().map(|(name, _)| *name).collect();
    assert_eq!(names.len(), all.len(), "every statement has one name");
    assert!(names.contains("THREAD_MEMBERS") && names.contains("HIT_THREADS"));
}
