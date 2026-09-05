//! # Hub over the real engine
//!
//! Runs the store's hub through real syncs, which the crate's unit
//! tests over hand-built writes cannot do.
//!
//! Convergence lives in the loop the projection and the write exist for:
//! project a source's view, let a sync merge and push it, absorb what it
//! wrote, project again. One store stands for the shared storage and each
//! source gets a handle of it plus its own remote.

use io_pimdir::{
    hub::{PimdirHub, PimdirSourceId},
    mutate::PimdirMutation,
    object::{PimdirHash, PimdirObject},
    placement::{PimdirHandle, PimdirLinkId, PimdirStatus},
    remote::PimdirTier,
    sync::{PimdirDeletePolicy, PimdirPushRights, PimdirSyncOptions},
};

use crate::common::{Client, MemRemote, hash};

/// Two sources over one store, each with its own server.
struct Mirror {
    a: Client,
    b: Client,
}

impl Mirror {
    fn new() -> Self {
        let a = Client::with_source(MemRemote::default(), "a");
        let b = a.sharing(MemRemote::default(), "b");

        Self { a, b }
    }

    /// The inbox hub, every source included.
    fn hub(&self) -> PimdirHub {
        self.a.hub("inbox")
    }

    /// Syncs and hydrates both sources until the hub stops changing.
    ///
    /// Hydration is not optional: a pulled row carries no link id, so the
    /// hub cannot key it, and a body the hub does not hold cannot be
    /// offered to another source. A mirror is a sync plus an upgrade.
    fn quiesce(&mut self, opts: PimdirSyncOptions) {
        for round in 0..8 {
            let before = self.hub();
            self.round(opts);
            if self.hub() == before {
                return;
            }
            assert!(round < 7, "the hub never settled");
        }
    }

    /// One pass over both sources: sync, then hydrate every row it left.
    fn round(&mut self, opts: PimdirSyncOptions) {
        self.round_with(opts, opts);
    }

    /// A pass with each source tuned differently, the point of a hub.
    fn round_with(&mut self, a: PimdirSyncOptions, b: PimdirSyncOptions) {
        for (source, opts) in [('a', a), ('b', b)] {
            let client = match source {
                'a' => &mut self.a,
                _ => &mut self.b,
            };
            client.sync("inbox", opts).unwrap();

            let handles: Vec<PimdirHandle> = client
                .open("inbox")
                .unwrap()
                .placements
                .iter()
                .map(|p| p.handle.clone())
                .collect();
            client.upgrade("inbox", handles, PimdirTier::Full).unwrap();
        }
    }

    /// The body the hub holds for `link`, which every source converges on.
    fn shared_body(&self, link: &str) -> Option<Vec<u8>> {
        let hub = self.hub();
        let item = hub.items.get(&PimdirLinkId::from(link))?;
        let object = item.object.as_ref()?;
        self.a.storage().body(object)
    }

    /// Whether the hub reads the item as a cross-source divergence.
    fn conflicted(&self, link: &str) -> bool {
        self.hub()
            .items
            .get(&PimdirLinkId::from(link))
            .is_some_and(|item| item.conflicted)
    }

    /// The body a source's server holds for its only member.
    ///
    /// The fake remote records a pushed object as its hash, not bytes.
    fn object_on(&self, source: char) -> Option<PimdirHash> {
        let remote = match source {
            'a' => self.a.remote(),
            _ => self.b.remote(),
        };
        let stored = remote
            .items
            .get(&"inbox".into())
            .and_then(|c| c.values().next())?;

        Some(PimdirHash::from(
            String::from_utf8(stored.body.clone()).expect("a pushed object"),
        ))
    }

    /// Whether the hub knows the item was deleted on some source.
    fn deleted(&self, link: &str) -> Option<bool> {
        let hub = self.hub();
        let item = hub.items.get(&PimdirLinkId::from(link))?;
        Some(item.deleted)
    }

    /// Whether the store retains the item: bound by nobody, in the trash view.
    fn retained(&self, link: &str) -> bool {
        self.a
            .storage()
            .retained("inbox")
            .iter()
            .any(|item| item.link_id.as_str() == link)
    }

    /// The handles a source's server holds, in order.
    fn server(&self, source: char) -> Vec<String> {
        let remote = match source {
            'a' => self.a.remote(),
            _ => self.b.remote(),
        };
        remote
            .items
            .get(&"inbox".into())
            .map(|c| c.keys().map(|h| h.as_str().to_string()).collect())
            .unwrap_or_default()
    }

    /// Every link the hub holds live, with the sources bound to it.
    fn bindings(&self) -> Vec<(String, Vec<String>)> {
        self.hub()
            .items
            .iter()
            .filter(|(_, item)| !item.sources.is_empty())
            .map(|(link, item)| {
                let sources = item
                    .sources
                    .keys()
                    .map(|s| s.as_str().to_string())
                    .collect();
                (link.as_str().to_string(), sources)
            })
            .collect()
    }
}

/// A member one source holds is appended to the other as one shared item.
#[test]
fn a_member_one_source_holds_is_mirrored_to_the_other() {
    let mut mirror = Mirror::new();
    mirror
        .a
        .remote_mut()
        .seed("inbox", "a1", "msg-a", &[], b"body a");

    mirror.quiesce(PimdirSyncOptions::default());

    assert_eq!(mirror.server('a'), ["a1"], "the source keeps its member");
    assert_eq!(
        mirror.server('b').len(),
        1,
        "the member is appended to the other source: {:?}",
        mirror.server('b'),
    );
    assert_eq!(
        mirror.bindings(),
        [("msg-a".to_string(), vec!["a".to_string(), "b".to_string()])],
        "one shared item, bound to both sources",
    );
}

/// A flag set on one source reaches the other through the hub, and settles.
#[test]
fn a_flag_change_propagates_across_sources() {
    let mut mirror = Mirror::new();
    mirror
        .a
        .remote_mut()
        .seed("inbox", "a1", "msg-a", &[], b"body a");
    mirror.quiesce(PimdirSyncOptions::default());

    let handle = mirror.server('b')[0].clone();
    mirror.a.remote_mut().set_flags("inbox", "a1", &["seen"]);
    mirror.quiesce(PimdirSyncOptions::default());

    assert!(
        mirror
            .b
            .remote()
            .flags_of("inbox", &handle)
            .contains("seen"),
        "the flag reached the other source's server",
    );
}

/// A delete on one source propagates to the other.
#[test]
fn a_delete_propagates_across_sources() {
    let mut mirror = Mirror::new();
    mirror
        .a
        .remote_mut()
        .seed("inbox", "a1", "msg-a", &[], b"body a");
    mirror.quiesce(PimdirSyncOptions::default());
    assert_eq!(mirror.server('b').len(), 1);

    mirror.a.remote_mut().remove("inbox", "a1");
    mirror.quiesce(PimdirSyncOptions::default());

    assert!(mirror.server('b').is_empty(), "the delete reached b");
    assert!(
        mirror.retained("msg-a"),
        "no source holds it, so the store retains it",
    );
    assert_eq!(mirror.a.storage().retained("inbox").len(), 1);
}

/// Under `Keep`, a source refusing removes just holds its copy.
///
/// The deletion stands for every source that took it.
#[test]
fn a_source_refusing_removes_holds_its_copy_under_keep() {
    let mut mirror = Mirror::new();
    mirror
        .a
        .remote_mut()
        .seed("inbox", "a1", "msg-a", &[], b"body a");
    mirror.quiesce(PimdirSyncOptions::default());
    let held = mirror.server('b')[0].clone();

    let no_removes = PimdirSyncOptions {
        rights: PimdirPushRights {
            remove: false,
            ..PimdirPushRights::all()
        },
        delete: PimdirDeletePolicy::Keep,
        ..Default::default()
    };
    mirror.a.remote_mut().remove("inbox", "a1");
    for _ in 0..3 {
        mirror.round_with(PimdirSyncOptions::default(), no_removes);
    }

    assert_eq!(
        mirror.server('b'),
        [held],
        "b's server still holds the member it refuses to delete",
    );
    assert_eq!(
        mirror.deleted("msg-a"),
        Some(true),
        "the hub keeps the deletion, so no source is offered it back",
    );
    assert!(
        mirror.server('a').is_empty(),
        "and the source that deleted it does not get it back: {:?}",
        mirror.server('a'),
    );
}

/// Under an explicit `Revert` policy the same scenario resurrects the item.
///
/// The revert reads as add-beats-delete across sources, so the hub mirrors
/// the item back to the source that deleted it. A hub-bound source wants
/// `Keep`, which is what the default `Auto` resolves to beside another
/// source. Both readings are coherent, so this is pinned rather than fixed.
#[test]
fn a_reverted_delete_resurrects_the_item_across_the_hub() {
    let mut mirror = Mirror::new();
    mirror
        .a
        .remote_mut()
        .seed("inbox", "a1", "msg-a", &[], b"body a");
    mirror.quiesce(PimdirSyncOptions::default());

    let no_removes = PimdirSyncOptions {
        rights: PimdirPushRights {
            remove: false,
            ..PimdirPushRights::all()
        },
        delete: PimdirDeletePolicy::Revert,
        ..Default::default()
    };
    mirror.a.remote_mut().remove("inbox", "a1");
    for _ in 0..3 {
        mirror.round_with(PimdirSyncOptions::default(), no_removes);
    }

    assert_eq!(
        mirror.deleted("msg-a"),
        Some(false),
        "the revert cleared the deletion for every source",
    );
    assert_eq!(
        mirror.server('a').len(),
        1,
        "so the item comes back to the source it was deleted on",
    );
}

/// A source that only pulls is mirrored into, never out of.
#[test]
fn a_read_only_source_receives_nothing_it_cannot_push() {
    let mut mirror = Mirror::new();
    mirror
        .a
        .remote_mut()
        .seed("inbox", "a1", "msg-a", &[], b"body a");

    let read_only = PimdirSyncOptions {
        push: false,
        ..Default::default()
    };
    for _ in 0..3 {
        mirror.round_with(PimdirSyncOptions::default(), read_only);
    }

    assert!(
        mirror.server('b').is_empty(),
        "a read-only source is never appended to: {:?}",
        mirror.server('b'),
    );
    let hub = mirror.hub();
    let item = hub.items.get(&PimdirLinkId::from("msg-a")).unwrap();
    let binding = item.sources.get(&PimdirSourceId::from("b"));
    assert!(
        binding.is_none_or(|b| b.base.is_none()),
        "b never synced it, so it holds no base for it",
    );
}

/// A pending create the hub offers reads as staged until pushed.
#[test]
fn an_offered_member_reads_as_a_pending_create_until_pushed() {
    let mut mirror = Mirror::new();
    mirror
        .a
        .remote_mut()
        .seed("inbox", "a1", "msg-a", &[], b"body a");
    mirror
        .a
        .sync("inbox", PimdirSyncOptions::default())
        .unwrap();
    mirror
        .a
        .upgrade("inbox", vec![PimdirHandle::from("a1")], PimdirTier::Full)
        .unwrap();

    let offered = mirror.b.open("inbox").unwrap().placements;

    assert_eq!(offered.len(), 1, "the hub offers b the member a holds");
    assert_eq!(offered[0].status, PimdirStatus::Created);
    assert!(
        offered[0].object.is_some(),
        "with the body, so pushing it needs no fetch",
    );
}

/// A second source's own copy of an offered identity binds the shared
/// item rather than taking a minted key.
///
/// The offer is what a whole-collection load projects so the merge
/// derives the append; read by key while a fetched identity is settled,
/// it would pass for a holding and mint `dup:` for the resource the
/// source has always held (SYNC §6: minted when this source binds the
/// hint under another handle).
#[test]
fn a_second_source_holding_an_offered_item_binds_it_rather_than_minting() {
    let mut mirror = Mirror::new();
    mirror
        .a
        .remote_mut()
        .seed("inbox", "a1", "msg-a", &[], b"body a");
    mirror
        .b
        .remote_mut()
        .seed("inbox", "b1", "msg-a", &[], b"body a");

    mirror
        .a
        .sync("inbox", PimdirSyncOptions::default())
        .unwrap();
    mirror
        .a
        .upgrade("inbox", vec![PimdirHandle::from("a1")], PimdirTier::Full)
        .unwrap();

    let pull_only = PimdirSyncOptions {
        push: false,
        ..Default::default()
    };
    mirror.b.sync("inbox", pull_only).unwrap();
    mirror
        .b
        .upgrade("inbox", vec![PimdirHandle::from("b1")], PimdirTier::Full)
        .unwrap();

    let bound = mirror.b.storage().placement("inbox", "b1");
    assert_eq!(
        bound.link_id.as_ref().map(|l| l.as_str()),
        Some("msg-a"),
        "the identity itself, not a mint over the offer: {bound:?}",
    );
    assert_eq!(
        mirror.bindings(),
        [("msg-a".to_string(), vec!["a".to_string(), "b".to_string()])],
        "one shared item, bound to both sources",
    );
}

/// One source holding an identity twice offers both copies to the other.
///
/// Two resources are two members, and which of them the user may want
/// on the other side is not the engine's judgement.
#[test]
fn both_copies_of_an_identity_reach_the_other_source() {
    let mut mirror = Mirror::new();
    mirror
        .a
        .remote_mut()
        .seed("inbox", "a1", "msg-a", &[], b"the meeting");
    mirror
        .a
        .remote_mut()
        .seed("inbox", "a2", "msg-a", &[], b"another meeting");

    mirror.quiesce(PimdirSyncOptions::default());

    assert_eq!(
        mirror.bindings(),
        [
            ("dup:msg-a#a2".to_string(), vec!["a".into(), "b".into()]),
            ("msg-a".to_string(), vec!["a".into(), "b".into()]),
        ],
        "two items, each bound to both sources",
    );
    assert_eq!(mirror.server('a').len(), 2, "the source keeps both");
    assert_eq!(
        mirror.server('b').len(),
        2,
        "and both are appended to the other: {:?}",
        mirror.server('b'),
    );
}

/// A target refusing the duplicate says so, and the refusal costs neither copy.
///
/// The rejected push is retried and both items stay as the source holds them.
#[test]
fn a_refused_duplicate_leaves_both_items_intact() {
    let mut mirror = Mirror::new();
    mirror
        .a
        .remote_mut()
        .seed("inbox", "a1", "msg-a", &[], b"the meeting");
    mirror
        .a
        .remote_mut()
        .seed("inbox", "a2", "msg-a", &[], b"another meeting");
    mirror
        .b
        .remote_mut()
        .refused_appends
        .insert(PimdirLinkId::from("dup:msg-a#a2"));

    mirror.quiesce(PimdirSyncOptions::default());

    assert_eq!(
        mirror.server('b'),
        ["app-1"],
        "b took the copy it can hold and refused the other",
    );
    assert_eq!(mirror.server('a').len(), 2, "a still holds both");
    assert_eq!(
        mirror.bindings(),
        [
            ("dup:msg-a#a2".to_string(), vec!["a".to_string()]),
            ("msg-a".to_string(), vec!["a".into(), "b".into()]),
        ],
        "the refused copy is still an item, bound to the source that has it",
    );
    assert_eq!(mirror.deleted("dup:msg-a#a2"), Some(false));
}

/// A local edit of `handle` on one source, as a consumer's editor stages it.
///
/// The body is stored and the placement repointed, its base left where
/// the last sync put it.
fn edit(client: &mut Client, handle: &str, body: &[u8]) {
    let object = PimdirObject {
        hash: hash(body),
        size: body.len(),
    };

    client
        .mutate(
            "inbox",
            PimdirMutation::Edit {
                handle: PimdirHandle::from(handle),
                object,
                body: body.to_vec(),
                summary: None,
                sort_key: None,
            },
        )
        .unwrap();
}

/// Two edits on one source before a push are not two sources disagreeing.
///
/// The first edit moves the shared body ahead of the base the source last
/// synced, the gap another source folding in leaves. Read as that gap, the
/// second edit would be filed as a divergence and never pushed.
#[test]
fn a_second_offline_edit_reaches_the_shared_item() {
    let mut mirror = Mirror::new();
    mirror.a.remote_mut().mutable = true;
    mirror.b.remote_mut().mutable = true;
    mirror
        .a
        .remote_mut()
        .seed("inbox", "a1", "msg-a", &[], b"the meeting");
    mirror.quiesce(PimdirSyncOptions::default());

    edit(&mut mirror.a, "a1", b"the meeting, moved");
    edit(&mut mirror.a, "a1", b"the meeting, moved again");

    assert_eq!(
        mirror.shared_body("msg-a").as_deref(),
        Some(&b"the meeting, moved again"[..]),
        "the newest edit is the shared body",
    );
    assert!(!mirror.conflicted("msg-a"), "one source, so no divergence");

    mirror.quiesce(PimdirSyncOptions::default());

    for source in ['a', 'b'] {
        assert_eq!(
            mirror.object_on(source),
            Some(hash(b"the meeting, moved again")),
            "both servers hold the newest edit",
        );
    }
}

/// The edit resolving a conflicted binding becomes the shared body.
///
/// Dropping it keeps the body the merge replaced, and the next push sends
/// that body over the remote the merge was made against.
#[test]
fn a_resolving_edit_is_what_gets_pushed() {
    let mut mirror = Mirror::new();
    mirror.a.remote_mut().mutable = true;
    mirror.b.remote_mut().mutable = true;
    mirror
        .a
        .remote_mut()
        .seed("inbox", "a1", "msg-a", &[], b"the meeting");
    mirror.quiesce(PimdirSyncOptions::default());

    edit(&mut mirror.a, "a1", b"the meeting, moved");
    mirror
        .a
        .remote_mut()
        .edit("inbox", "a1", b"the meeting, cancelled");
    mirror
        .a
        .sync("inbox", PimdirSyncOptions::default())
        .unwrap();

    let conflicted = mirror
        .a
        .open("inbox")
        .unwrap()
        .placements
        .iter()
        .any(|p| p.status == PimdirStatus::Conflict);
    assert!(conflicted, "the source and its own server diverged");

    edit(&mut mirror.a, "a1", b"the meeting, moved and cancelled");

    assert_eq!(
        mirror.shared_body("msg-a").as_deref(),
        Some(&b"the meeting, moved and cancelled"[..]),
        "the merged body is the shared body",
    );

    mirror.quiesce(PimdirSyncOptions::default());

    for source in ['a', 'b'] {
        assert_eq!(
            mirror.object_on(source),
            Some(hash(b"the meeting, moved and cancelled")),
            "and the merge is what every server ends up with",
        );
    }
}

/// A create persisted through the hub still reads as a create.
///
/// The hub binds every live upsert, and a bound placement once read back
/// as `Dirty`, while the merge derives an add for a `Created` one alone,
/// so the item never reached the source that authored it.
#[test]
fn a_create_persisted_through_the_hub_still_reads_as_one() {
    let mut mirror = Mirror::new();
    let body = b"authored on a";
    let object = PimdirObject {
        hash: hash(body),
        size: body.len(),
    };
    mirror
        .a
        .mutate(
            "inbox",
            PimdirMutation::Add {
                handle: PimdirHandle::from("tmp-1"),
                link_id: PimdirLinkId::from("msg-a"),
                flags: Default::default(),
                object,
                body: body.to_vec(),
                summary: None,
                sort_key: Default::default(),
            },
        )
        .unwrap();

    let projected = mirror.a.open("inbox").unwrap().placements;
    assert_eq!(projected.len(), 1, "the staged create is the only member");
    assert_eq!(
        projected[0].status,
        PimdirStatus::Created,
        "a binding with no base has never reached its source: {:?}",
        projected[0],
    );

    mirror.quiesce(PimdirSyncOptions::default());

    assert_eq!(
        mirror.server('a').len(),
        1,
        "the source it was authored on holds it: {:?}",
        mirror.server('a'),
    );
    assert_eq!(mirror.server('b').len(), 1, "and so does the other");
}

/// A member a rebuild carried nowhere is one that left the source.
///
/// A rebuild's two drops look alike from the storage's side and mean
/// opposite things: a renumbered row is replaced by one the same batch
/// writes, and one the new space accounts for nowhere is gone.
#[test]
fn a_member_a_rebuild_lost_is_deleted_across_the_hub() {
    let mut mirror = Mirror::new();
    mirror
        .a
        .remote_mut()
        .seed("inbox", "a1", "msg-a", &[], b"body a");
    mirror
        .a
        .remote_mut()
        .seed("inbox", "a2", "msg-b", &[], b"body b");
    mirror.quiesce(PimdirSyncOptions::default());
    assert_eq!(mirror.server('b').len(), 2, "both members reached b");

    mirror.a.remote_mut().remove("inbox", "a1");
    mirror.a.remote_mut().renumber("inbox", 1);
    mirror.a.rekey("inbox").unwrap();
    mirror.quiesce(PimdirSyncOptions {
        delete: PimdirDeletePolicy::Keep,
        ..Default::default()
    });

    assert!(
        mirror.retained("msg-a"),
        "the expunged member is retained by the store, not offered back",
    );
    assert_eq!(mirror.server('a').len(), 1, "and not re-appended to a");
    assert_eq!(
        mirror.server('b').len(),
        1,
        "while b hears about the deletion: {:?}",
        mirror.server('b'),
    );
}
