//! Search without the client (SEARCH §5, §6, §8): the query language, the
//! extraction and the refresh coroutine, built with `search` alone, so no
//! SQLite is linked and nothing touches a file.

#![cfg(feature = "search")]

use std::{
    collections::{BTreeMap, BTreeSet},
    slice,
};

use io_pimdir::{
    coroutine::PimdirCoroutineState,
    object::PimdirHash,
    placement::{PimdirFlags, PimdirLinkId},
    search::{
        extract::{PimdirExtract, PimdirExtractStatus},
        query::{PimdirQuery, PimdirQueryPredicate},
        refresh::{
            PimdirIndexFeedRow, PimdirIndexMessage, PimdirIndexMeta, PimdirIndexOp,
            PimdirIndexRefresh, PimdirIndexRefreshArg, PimdirIndexRefreshYield,
        },
    },
    summary::mail::{MIME_DEPTH_MAX, MIME_ENTITIES_MAX},
};

const MESSAGE: &[u8] = b"Message-ID: <a@x>\r\n\
From: Jane Doe <jane@example.org>\r\n\
To: bob@example.org\r\n\
Subject: R\xc3\xa9union budget\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
The quarterly figures are in.\r\n";

#[test]
fn a_query_parses_and_a_message_extracts() {
    let query = PimdirQuery::parse("from:jane subject:reunion").unwrap();
    let PimdirQuery::And(terms) = &query else {
        panic!("a conjunction: {query:?}");
    };
    assert_eq!(terms.len(), 2);
    let PimdirQuery::Term(term) = &terms[0] else {
        panic!("a term: {:?}", terms[0]);
    };
    assert!(matches!(
        term.predicate,
        PimdirQueryPredicate::Address { .. }
    ));

    let extract = PimdirExtract::mail(MESSAGE);
    assert_eq!(extract.status, PimdirExtractStatus::Ok);
    assert_eq!(extract.title, "Réunion budget");
    assert!(
        extract.from.contains("jane@example.org"),
        "{}",
        extract.from
    );
    assert!(extract.body.contains("quarterly figures"));
}

/// The extraction reads the parts the walk of Annex A.1 reads: a text part at
/// depth 64, not one deeper, and none after the 10,000th entity.
#[test]
fn the_extraction_reads_the_parts_the_walk_reads() {
    let nested = |depth: usize| {
        let mut body = String::from("Message-ID: <deep@x>\r\n");
        for level in 0..depth {
            body.push_str(&format!(
                "Content-Type: multipart/mixed; boundary=b{level}x\r\n\r\n--b{level}x\r\n"
            ));
        }
        body.push_str("Content-Type: text/plain\r\n\r\nbottom\r\n");
        for level in (0..depth).rev() {
            body.push_str(&format!("--b{level}x--\r\n"));
        }
        PimdirExtract::mail(body.as_bytes()).body
    };
    assert!(nested(MIME_DEPTH_MAX).contains("bottom"));
    assert!(!nested(MIME_DEPTH_MAX + 1).contains("bottom"));

    let mut body = String::from("Content-Type: multipart/mixed; boundary=b\r\n\r\n");
    for _ in 2..MIME_ENTITIES_MAX {
        body.push_str("--b\r\nContent-Type: image/png\r\n\r\nx\r\n");
    }
    body.push_str("--b\r\nContent-Type: text/plain\r\n\r\nlast read\r\n");
    body.push_str("--b\r\nContent-Type: text/plain\r\n\r\nunread\r\n--b--\r\n");
    let extract = PimdirExtract::mail(body.as_bytes());
    assert!(extract.body.contains("last read"));
    assert!(!extract.body.contains("unread"));
}

/// One run over a one-message feed, answered by hand: the body indexed
/// with its text folded, the placement put, the thread written, the cursor
/// recorded past the row.
#[test]
fn a_refresh_runs_without_a_database() {
    let hash = PimdirHash("h1".into());
    let row = PimdirIndexFeedRow {
        collection: "INBOX".into(),
        seq: 1,
        link_id: PimdirLinkId("a@x".into()),
        object: Some(hash.clone()),
        flags: PimdirFlags::default(),
        deleted: false,
        retained: false,
        kind: "message/rfc822".into(),
        account: None,
        changed: 1,
        summary: None,
    };

    let mut run = PimdirIndexRefresh::new(1_790_000_000, 16);
    let mut ops = Vec::new();
    let mut arg = None;
    let output = loop {
        let want = match run.resume(arg.take()) {
            PimdirCoroutineState::Yielded(want) => want,
            PimdirCoroutineState::Complete(output) => break output.unwrap(),
        };
        arg = Some(match want {
            PimdirIndexRefreshYield::WantsIndexState => {
                PimdirIndexRefreshArg::IndexState(PimdirIndexMeta {
                    head: 1,
                    ..Default::default()
                })
            }
            PimdirIndexRefreshYield::WantsFeed { since: 0, .. } => {
                PimdirIndexRefreshArg::Feed(vec![row.clone()])
            }
            PimdirIndexRefreshYield::WantsMissing => PimdirIndexRefreshArg::Feed(Vec::new()),
            PimdirIndexRefreshYield::WantsHeld { .. } => PimdirIndexRefreshArg::Held {
                placements: Vec::new(),
                objects: BTreeSet::new(),
            },
            PimdirIndexRefreshYield::WantsBodies { hashes } => {
                assert_eq!(hashes, slice::from_ref(&hash));
                PimdirIndexRefreshArg::Bodies(BTreeMap::from([(hash.clone(), MESSAGE.to_vec())]))
            }
            PimdirIndexRefreshYield::WantsMessages { link_ids } => PimdirIndexRefreshArg::Messages(
                link_ids
                    .into_iter()
                    .map(|link_id| {
                        let mut message = PimdirIndexMessage::default();
                        if link_id == "a@x" {
                            message.presence = Some(Default::default());
                            message.hash = Some(hash.clone());
                        }
                        (link_id, message)
                    })
                    .collect(),
            ),
            PimdirIndexRefreshYield::Writes(written) => {
                ops.extend(written);
                PimdirIndexRefreshArg::Written
            }
            want => panic!("unexpected {want:?}"),
        });
    };

    assert!(output.exhausted);
    assert_eq!((output.cursor, output.folded, output.indexed), (1, 1, 1));
    let object = ops
        .iter()
        .find_map(|op| match op {
            PimdirIndexOp::PutObject(object) => Some(object),
            _ => None,
        })
        .expect("the body is indexed");
    assert_eq!(
        object.text.title, "reunion budget",
        "the text arrives folded"
    );
    assert!(ops.contains(&PimdirIndexOp::PutThread {
        id: "a@x".into(),
        first: String::new(),
        last: String::new(),
        count: 1,
    }));
    assert_eq!(
        ops.last(),
        Some(&PimdirIndexOp::SetCursor {
            store_change: 1,
            store_purges: 0,
        })
    );
}
