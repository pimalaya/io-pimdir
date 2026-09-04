//! # Sync properties
//!
//! Property-based safety net over the sync engine, whose input space is
//! operation sequences rather than bytes.
//!
//! Random interleavings of local mutations, server-side mutations and
//! syncs must show no panic on protocol misuse, no user intent silently
//! lost, convergence to the server state once quiescent, and idempotence
//! of a quiescent sync.
//!
//! Local ops act on named rows only, the ones a consumer can list, so
//! every model hydrates to `Meta` after each sync as a consumer would.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    convert::Infallible,
    rc::Rc,
};

use io_pimdir::{
    change::PimdirChange,
    collection::{PimdirCheckpoint, PimdirCollectionId},
    coroutine::{PimdirArg, PimdirCoroutine, PimdirCoroutineState},
    load::PimdirLoaded,
    mutate::{PimdirMutate, PimdirMutation},
    object::{PimdirHash, PimdirObject},
    open::PimdirOpen,
    placement::{PimdirFlags, PimdirHandle, PimdirLinkId, PimdirPlacement, PimdirStatus},
    remote::{PimdirFetchedItem, PimdirPushResult, PimdirRemote, PimdirRemoteSnapshot, PimdirTier},
    sync::{PimdirSync, PimdirSyncOptions, PimdirSyncReport},
    upgrade::PimdirUpgrade,
};
use proptest::{prelude::*, test_runner::TestCaseError};

use crate::common::{Client, MemRemote, hash};

/// A small flag universe, so the sets overlap and the merge has work.
fn arb_flags() -> impl Strategy<Value = PimdirFlags> {
    proptest::collection::btree_set(
        prop_oneof![
            Just("seen"),
            Just("flagged"),
            Just("draft"),
            Just("answered")
        ],
        0..4,
    )
    .prop_map(PimdirFlags::from_iter)
}

proptest! {
    /// Every change a side made against the base survives the merge.
    ///
    /// An addition is present and a removal absent: the no-silent-loss
    /// property of the flag axis.
    #[test]
    fn flags_merge_loses_no_intent(
        base in arb_flags(),
        local in arb_flags(),
        remote in arb_flags(),
    ) {
        let merged = PimdirFlags::merge(&base, &local, &remote);

        let base_set = base.known().expect("generated known").clone();
        for side in [&local, &remote] {
            let side = side.known().expect("generated known");
            for added in side.difference(&base_set) {
                prop_assert!(merged.contains(added), "{added} added by one side");
            }
            for removed in base_set.difference(side) {
                prop_assert!(!merged.contains(removed), "{removed} removed by one side");
            }
        }
    }

    /// The merge is symmetric in its sides and keeps what nobody touched.
    #[test]
    fn flags_merge_is_symmetric_and_stable(
        base in arb_flags(),
        local in arb_flags(),
        remote in arb_flags(),
    ) {
        let ab = PimdirFlags::merge(&base, &local, &remote);
        let ba = PimdirFlags::merge(&base, &remote, &local);
        prop_assert_eq!(&ab, &ba);

        let stable = PimdirFlags::merge(&base, &base, &base);
        prop_assert_eq!(&stable, &base);
    }
}

/// Any coroutine arg with an empty payload, to generate protocol misuse.
fn arb_arg() -> impl Strategy<Value = Option<PimdirArg>> {
    prop_oneof![
        Just(None),
        Just(Some(PimdirArg::Write)),
        Just(Some(PimdirArg::Push(vec![]))),
        Just(Some(PimdirArg::Fetch(vec![]))),
        Just(Some(PimdirArg::LookupObject(Default::default()))),
        Just(Some(PimdirArg::Load(PimdirLoaded::default()))),
        Just(Some(PimdirArg::Enumerate(PimdirRemoteSnapshot {
            items: vec![],
            vanished: vec![],
            complete: true,
            checkpoint: Default::default(),
        }))),
    ]
}

/// Feeds the sequence until the coroutine completes, without panicking.
fn feed<C: PimdirCoroutine>(mut coroutine: C, args: Vec<Option<PimdirArg>>) {
    for arg in args {
        if let PimdirCoroutineState::Complete(_) = coroutine.resume(arg) {
            return;
        }
    }
}

proptest! {
    #[test]
    fn coroutines_survive_any_arg_sequence(args in proptest::collection::vec(arb_arg(), 1..8)) {
        feed(PimdirOpen::new("inbox"), args.clone());
        feed(
            PimdirMutate::new("inbox", PimdirMutation::Remove(PimdirHandle::from("1"))),
            args.clone(),
        );
        feed(
            PimdirUpgrade::new("inbox", vec![PimdirHandle::from("1")], PimdirTier::Full),
            args.clone(),
        );
        feed(PimdirSync::new("inbox", PimdirSyncOptions::default()), args);
    }
}

/// Names every live member of `collection`, as a consumer listing it does.
fn hydrate<R: PimdirRemote>(client: &mut Client<R>, collection: &str)
where
    R::Error: std::fmt::Debug + std::fmt::Display,
{
    let handles: Vec<PimdirHandle> = client
        .storage()
        .rows(collection)
        .into_iter()
        .filter(|p| p.status != PimdirStatus::Tombstone)
        .map(|p| p.handle)
        .collect();
    let _ = client.upgrade(collection, handles, PimdirTier::Meta);
}

/// The named, live rows of `collection`: what a consumer can act on.
fn named<R: PimdirRemote>(client: &Client<R>, collection: &str) -> BTreeSet<PimdirHandle>
where
    R::Error: std::fmt::Debug + std::fmt::Display,
{
    client
        .storage()
        .rows(collection)
        .into_iter()
        .filter(|p| p.status != PimdirStatus::Tombstone && p.link_id.is_some())
        .map(|p| p.handle)
        .collect()
}

/// One step of the random scenario.
///
/// Handle picks are indices resolved modulo the live set at execution
/// time, so every generated op is valid by construction and shrinking
/// stays meaningful.
#[derive(Clone, Debug)]
enum Op {
    /// Replace the flags of the i-th local placement.
    LocalSetFlags(usize, PimdirFlags),
    /// Delete the i-th local placement offline.
    LocalRemove(usize),
    /// Replace the flags of the i-th server item.
    ServerSetFlags(usize, PimdirFlags),
    /// Delete the i-th server item.
    ServerRemove(usize),
    /// A new message arrives server-side.
    ServerAdd(u8),
    /// Reconcile.
    Sync,
}

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (any::<usize>(), arb_flags()).prop_map(|(i, f)| Op::LocalSetFlags(i, f)),
        any::<usize>().prop_map(Op::LocalRemove),
        (any::<usize>(), arb_flags()).prop_map(|(i, f)| Op::ServerSetFlags(i, f)),
        any::<usize>().prop_map(Op::ServerRemove),
        any::<u8>().prop_map(Op::ServerAdd),
        Just(Op::Sync),
    ]
}

fn nth(handles: &BTreeSet<PimdirHandle>, i: usize) -> Option<PimdirHandle> {
    if handles.is_empty() {
        return None;
    }
    handles.iter().nth(i % handles.len()).cloned()
}

proptest! {
    /// Two quiescent syncs converge onto the server, a third is a no-op.
    ///
    /// The fake remote accepts every push and its content is immutable,
    /// so no conflict can excuse a divergence.
    #[test]
    fn random_interleavings_converge(ops in proptest::collection::vec(arb_op(), 0..25)) {
        let mut client = Client::new(MemRemote::default());
        client.remote_mut().seed("inbox", "m1", "l1", &[], b"one");
        client.remote_mut().seed("inbox", "m2", "l2", &["seen"], b"two");
        client.remote_mut().seed("inbox", "m3", "l3", &["flagged"], b"three");
        client.remote_mut().seed("inbox", "m4", "l4", &["seen", "draft"], b"four");
        client.remote_mut().seed("inbox", "m5", "l5", &[], b"five");
        let opts = PimdirSyncOptions::default();
        client.sync("inbox", opts).unwrap();
        hydrate(&mut client, "inbox");

        for op in ops {
            match op {
                Op::LocalSetFlags(i, flags) => {
                    if let Some(handle) = nth(&named(&client, "inbox"), i) {
                        client
                            .mutate("inbox", PimdirMutation::SetFlags { handle, flags })
                            .unwrap();
                    }
                }
                Op::LocalRemove(i) => {
                    if let Some(handle) = nth(&named(&client, "inbox"), i) {
                        client.mutate("inbox", PimdirMutation::Remove(handle)).unwrap();
                    }
                }
                Op::ServerSetFlags(i, flags) => {
                    let handles = client.remote().handles("inbox");
                    if let Some(handle) = nth(&handles, i) {
                        let flags: Vec<&str> = flags.known().into_iter().flatten().map(|f| f.as_str()).collect();
                        client.remote_mut().set_flags("inbox", handle.as_str(), &flags);
                    }
                }
                Op::ServerRemove(i) => {
                    let handles = client.remote().handles("inbox");
                    if let Some(handle) = nth(&handles, i) {
                        client.remote_mut().remove("inbox", handle.as_str());
                    }
                }
                Op::ServerAdd(n) => {
                    let handle = format!("srv-{n}");
                    let link = format!("lnk-{n}");
                    client.remote_mut().seed("inbox", &handle, &link, &[], b"new");
                }
                Op::Sync => {
                    client.sync("inbox", opts).unwrap();
                    hydrate(&mut client, "inbox");
                }
            }
        }

        client.sync("inbox", opts).unwrap();
        client.sync("inbox", opts).unwrap();

        let placements = client.open("inbox").unwrap().placements;
        let local: BTreeSet<PimdirHandle> = placements.iter().map(|p| p.handle.clone()).collect();
        let server = client.remote().handles("inbox");
        prop_assert_eq!(&local, &server, "replica mirrors the server members");

        for placement in &placements {
            prop_assert_eq!(
                placement.status,
                PimdirStatus::Clean,
                "nothing left dirty after quiescence: {:?}",
                placement,
            );
            let server_flags = client
                .remote()
                .flags_of("inbox", placement.handle.as_str());
            prop_assert_eq!(&placement.flags, server_flags, "flags converged");
        }

        let report = client.sync("inbox", opts).unwrap();
        prop_assert_eq!(report, PimdirSyncReport::default());
    }
}

/// One step of the mutable-content scenario.
///
/// Indices resolve modulo the live set at execution time. Local ops
/// target the inbox, a copy or move the archive; server ops touch the
/// inbox only, so the archive changes through engine pushes alone.
#[derive(Clone, Debug)]
enum MutOp {
    LocalSetFlags(usize, PimdirFlags),
    LocalRemove(usize),
    /// Stage a local content edit on the i-th placement.
    LocalEdit(usize, u8),
    /// Copy the i-th live inbox placement into the archive.
    LocalCopy(usize),
    /// Move the i-th live inbox placement into the archive.
    LocalMove(usize),
    ServerSetFlags(usize, PimdirFlags),
    ServerRemove(usize),
    /// A server-side content edit: the revision advances.
    ServerEdit(usize, u8),
    /// A new message arrives server-side, always under a fresh handle.
    ServerAdd(u8),
    /// Raise the i-th live inbox placement to full detail.
    Upgrade(usize),
    /// The server renumbers every member and the replica rekeys.
    Bump,
    Sync,
    SyncArchive,
}

/// Weighted toward edits and upgrades, with syncs kept low.
///
/// A content conflict needs a local and a remote edit on one handle with
/// no sync between them, and `Upgrade` is what asks for a conflicted
/// item's diverging body.
fn arb_mut_op() -> impl Strategy<Value = MutOp> {
    prop_oneof![
        1 => (any::<usize>(), arb_flags()).prop_map(|(i, f)| MutOp::LocalSetFlags(i, f)),
        1 => any::<usize>().prop_map(MutOp::LocalRemove),
        4 => (any::<usize>(), any::<u8>()).prop_map(|(i, n)| MutOp::LocalEdit(i, n)),
        1 => any::<usize>().prop_map(MutOp::LocalCopy),
        1 => any::<usize>().prop_map(MutOp::LocalMove),
        1 => (any::<usize>(), arb_flags()).prop_map(|(i, f)| MutOp::ServerSetFlags(i, f)),
        1 => any::<usize>().prop_map(MutOp::ServerRemove),
        4 => (any::<usize>(), any::<u8>()).prop_map(|(i, n)| MutOp::ServerEdit(i, n)),
        1 => any::<u8>().prop_map(MutOp::ServerAdd),
        3 => any::<usize>().prop_map(MutOp::Upgrade),
        1 => Just(MutOp::Bump),
        2 => Just(MutOp::Sync),
        1 => Just(MutOp::SyncArchive),
    ]
}

/// What the user asked for, accounted for at the end.
///
/// Every intent must land, stay visibly pending, or be superseded by a
/// strictly later action on the same item. Entries are removed exactly
/// when a later op legitimately overrides them.
#[derive(Default)]
struct Ledger {
    /// Last staged edit per inbox handle: the body's hash.
    edits: BTreeMap<PimdirHandle, PimdirHash>,
    /// Last staged flag delta per inbox handle: (added, removed).
    ///
    /// Only elements changed against the held base carry an obligation.
    flags: BTreeMap<PimdirHandle, (BTreeSet<String>, BTreeSet<String>)>,
    /// Staged copies: the placeholder and the source's server link.
    copies: Vec<(PimdirHandle, Option<PimdirLinkId>)>,
    /// Staged moves: the source handle, its server link, and voided.
    moves: Vec<(PimdirHandle, Option<PimdirLinkId>, bool)>,
}

/// The live (non-tombstoned) named inbox placements.
fn live(client: &Client) -> BTreeSet<PimdirHandle> {
    named(client, "inbox")
}

/// The live inbox placements holding a body: what a copy or a move can
/// deliver, since a binding carries no origin and the create uploads.
fn hydrated(client: &Client) -> BTreeSet<PimdirHandle> {
    client
        .storage()
        .rows("inbox")
        .into_iter()
        .filter(|p| p.status != PimdirStatus::Tombstone && p.object.is_some())
        .map(|p| p.handle)
        .collect()
}

fn on_server(client: &Client, collection: &str) -> BTreeSet<PimdirHandle> {
    client.remote().handles(collection)
}

fn server_link(client: &Client, handle: &PimdirHandle) -> Option<PimdirLinkId> {
    client
        .remote()
        .items
        .get(&"inbox".into())?
        .get(handle)
        .map(|i| i.link_id.clone())
}

/// The current server-side body of an inbox member.
fn server_body(client: &Client, handle: &PimdirHandle) -> Vec<u8> {
    client
        .remote()
        .items
        .get(&"inbox".into())
        .and_then(|c| c.get(handle))
        .map(|i| i.body.clone())
        .unwrap_or_default()
}

/// The inbox placement under `handle`, if the source holds one.
fn inbox_row(client: &Client, handle: &PimdirHandle) -> Option<PimdirPlacement> {
    client.storage().get("inbox", handle.as_str())
}

/// The object hash an inbox placement currently points at.
fn held_object(client: &Client, handle: &PimdirHandle) -> Option<PimdirHash> {
    inbox_row(client, handle).and_then(|p| p.object)
}

/// The body a resolution keeps: local, ancestor, remote, or a merge.
///
/// Picked from the handle rather than generated, so a case replays the
/// same way. A body the store does not hold falls back to the merge.
fn resolution_body(client: &Client, handle: &PimdirHandle) -> (PimdirObject, Vec<u8>) {
    let placement = inbox_row(client, handle);
    let stored = |hash: Option<PimdirHash>| {
        let hash = hash?;
        let body = client
            .storage()
            .body(&hash)
            .filter(|body| !body.is_empty())?;
        let object = PimdirObject {
            hash,
            size: body.len(),
        };
        Some((object, body))
    };

    let choice = handle
        .as_str()
        .bytes()
        .fold(0u8, |acc, byte| acc.wrapping_add(byte));
    let kept = match choice % 4 {
        0 => stored(placement.as_ref().and_then(|p| p.object.clone())),
        1 => stored(
            placement
                .as_ref()
                .and_then(|p| p.base.as_ref())
                .and_then(|base| base.object.clone()),
        ),
        2 => stored(placement.as_ref().and_then(|p| p.conflict_object.clone())),
        _ => None,
    };

    kept.unwrap_or_else(|| {
        let body = format!("resolved-{}", handle.as_str()).into_bytes();
        let object = PimdirObject {
            hash: hash(&body),
            size: body.len(),
        };
        (object, body)
    })
}

/// The body an inbox placement pends for a push, as the engine reads it.
///
/// An edit restating the body the base holds stages nothing, so it is no
/// intent to account for.
fn pending_edit(client: &Client, handle: &PimdirHandle) -> Option<PimdirHash> {
    inbox_row(client, handle).and_then(|p| p.staged_edit().cloned())
}

/// Voids the edit intents whose content a server-side change destroyed.
///
/// A landed edit dies with its content, matched by body since a resurrect
/// may have re-keyed the handle, unless a local placement still pends it.
fn void_superseded_edits(ledger: &mut Ledger, client: &Client, destroyed: &[u8]) {
    let placements = client.storage().rows("inbox");
    ledger.edits.retain(|_, staged| {
        let landed_here = destroyed == staged.as_str().as_bytes();
        // NOTE: mirrors the engine's resurrect predicate: an unlanded
        // staged edit, not a placement dirty on its flag axis alone
        let pending = placements.iter().any(|p| {
            p.object.as_ref() == Some(staged)
                && (p.status == PimdirStatus::Created
                    || (matches!(p.status, PimdirStatus::Dirty | PimdirStatus::Conflict)
                        && p.base.as_ref().is_none_or(|b| b.object != p.object)))
        });
        !landed_here || pending
    });
}

fn collection_has_link(client: &Client, collection: &str, link: &PimdirLinkId) -> bool {
    client
        .remote()
        .items
        .get(&collection.into())
        .into_iter()
        .flatten()
        .any(|(_, item)| &item.link_id == link)
}

/// Runs the mutable-content scenario, then the convergence and ledger laws.
///
/// Every conflict left after quiescence is resolved with an edit.
fn check_mutable_model(ops: Vec<MutOp>) -> Result<(), TestCaseError> {
    let mut remote = MemRemote::default();
    remote.mutable = true;
    remote.seed("inbox", "m1", "l1", &[], b"one");
    remote.seed("inbox", "m2", "l2", &["seen"], b"two");
    remote.seed("inbox", "m3", "l3", &["flagged"], b"three");
    remote.seed("inbox", "m4", "l4", &["seen", "draft"], b"four");
    remote.seed("inbox", "m5", "l5", &[], b"five");

    let mut client = Client::new(remote);
    let opts = PimdirSyncOptions::default();
    let _ = client.sync("inbox", opts);
    hydrate(&mut client, "inbox");

    let mut ledger = Ledger::default();
    let mut placeholders = 0usize;
    let mut arrivals = 0usize;
    let mut bumps = 0usize;

    for op in ops {
        match op {
            MutOp::LocalSetFlags(i, flags) => {
                if let Some(handle) = nth(&live(&client), i) {
                    let base = inbox_row(&client, &handle)
                        .and_then(|p| p.base)
                        .map(|b| b.flags)
                        .unwrap_or_default();
                    let base = base.known().cloned().unwrap_or_default();
                    let known = flags.known().cloned().unwrap_or_default();
                    let added = known.difference(&base).cloned().collect();
                    let removed = base.difference(&known).cloned().collect();
                    let staged = client.mutate(
                        "inbox",
                        PimdirMutation::SetFlags {
                            handle: handle.clone(),
                            flags,
                        },
                    );
                    if staged.is_ok() {
                        ledger.flags.insert(handle, (added, removed));
                    }
                }
            }
            MutOp::LocalRemove(i) => {
                if let Some(handle) = nth(&live(&client), i) {
                    let held = held_object(&client, &handle);
                    let server_body = server_body(&client, &handle);
                    if client
                        .mutate("inbox", PimdirMutation::Remove(handle.clone()))
                        .is_ok()
                    {
                        if let Some(held) = held {
                            ledger.edits.retain(|_, staged| staged != &held);
                        }
                        void_superseded_edits(&mut ledger, &client, &server_body);
                        ledger.edits.remove(&handle);
                        ledger.flags.remove(&handle);

                        // NOTE: a pickable move source is one the engine
                        // resurrected, so deleting it is a later action
                        // voiding the move
                        for staged_move in &mut ledger.moves {
                            if staged_move.0 == handle {
                                staged_move.2 = true;
                            }
                        }
                    }
                }
            }
            MutOp::LocalEdit(i, n) => {
                if let Some(handle) = nth(&live(&client), i) {
                    let held = held_object(&client, &handle);
                    let server_body = server_body(&client, &handle);
                    let body = format!("edit-{n}-{}", handle.as_str()).into_bytes();
                    let object = PimdirObject {
                        hash: hash(&body),
                        size: body.len(),
                    };
                    let staged = client.mutate(
                        "inbox",
                        PimdirMutation::Edit {
                            sort_key: Default::default(),
                            handle: handle.clone(),
                            object,
                            body: body.clone(),
                            summary: None,
                        },
                    );
                    if staged.is_ok() {
                        if let Some(held) = held {
                            ledger.edits.retain(|_, staged| staged != &held);
                        }
                        void_superseded_edits(&mut ledger, &client, &server_body);
                        if pending_edit(&client, &handle) == Some(hash(&body)) {
                            ledger.edits.insert(handle, hash(&body));
                        }
                    }
                }
            }
            MutOp::LocalCopy(i) => {
                if let Some(handle) = nth(&hydrated(&client), i) {
                    placeholders += 1;
                    let placeholder = PimdirHandle::from(format!("tmp-{placeholders}"));
                    let link = server_link(&client, &handle);
                    let staged = client.mutate(
                        "inbox",
                        PimdirMutation::Copy {
                            handle,
                            target: "archive".into(),
                            placeholder: placeholder.clone(),
                        },
                    );
                    if staged.is_ok() {
                        ledger.copies.push((placeholder, link));
                    }
                }
            }
            MutOp::LocalMove(i) => {
                if let Some(handle) = nth(&hydrated(&client), i) {
                    let link = server_link(&client, &handle);
                    let server_body = server_body(&client, &handle);
                    let doomed = inbox_row(&client, &handle)
                        .and_then(|p| p.base)
                        .and_then(|b| b.revision)
                        != client
                            .remote()
                            .items
                            .get(&"inbox".into())
                            .and_then(|c| c.get(&handle))
                            .map(|i| i.rev.to_string());
                    let held = held_object(&client, &handle);
                    let staged = client.mutate(
                        "inbox",
                        PimdirMutation::Move {
                            handle: handle.clone(),
                            target: "archive".into(),
                            placeholder: PimdirHandle::from(format!(
                                "move:archive:{}",
                                handle.as_str()
                            )),
                        },
                    );
                    if staged.is_ok() {
                        if let Some(held) = held {
                            ledger.edits.retain(|_, staged| staged != &held);
                        }
                        void_superseded_edits(&mut ledger, &client, &server_body);
                        ledger.edits.remove(&handle);
                        ledger.flags.remove(&handle);
                        ledger.moves.push((handle, link, doomed));
                    }
                }
            }
            MutOp::ServerSetFlags(i, flags) => {
                if let Some(handle) = nth(&on_server(&client, "inbox"), i) {
                    let flags: Vec<&str> = flags
                        .known()
                        .into_iter()
                        .flatten()
                        .map(|f| f.as_str())
                        .collect();
                    client
                        .remote_mut()
                        .set_flags("inbox", handle.as_str(), &flags);
                    ledger.flags.remove(&handle);
                }
            }
            MutOp::ServerRemove(i) => {
                if let Some(handle) = nth(&on_server(&client, "inbox"), i) {
                    let doomed = client
                        .remote()
                        .items
                        .get(&"inbox".into())
                        .and_then(|c| c.get(&handle))
                        .map(|i| i.body.clone())
                        .unwrap_or_default();
                    client.remote_mut().remove("inbox", handle.as_str());
                    void_superseded_edits(&mut ledger, &client, &doomed);
                    ledger.flags.remove(&handle);
                    for staged_move in &mut ledger.moves {
                        if staged_move.0 == handle {
                            staged_move.2 = true;
                        }
                    }
                }
            }
            MutOp::ServerEdit(i, n) => {
                if let Some(handle) = nth(&on_server(&client, "inbox"), i) {
                    let overwritten = client
                        .remote()
                        .items
                        .get(&"inbox".into())
                        .and_then(|c| c.get(&handle))
                        .map(|i| i.body.clone())
                        .unwrap_or_default();
                    let body = format!("srv-edit-{n}").into_bytes();
                    client.remote_mut().edit("inbox", handle.as_str(), &body);
                    void_superseded_edits(&mut ledger, &client, &overwritten);
                    for staged_move in &mut ledger.moves {
                        if staged_move.0 == handle {
                            staged_move.2 = true;
                        }
                    }
                }
            }
            MutOp::ServerAdd(n) => {
                arrivals += 1;
                let handle = format!("srv-{arrivals}");
                let link = format!("lnk-{arrivals}");
                let body = format!("new-{n}").into_bytes();
                client
                    .remote_mut()
                    .seed("inbox", &handle, &link, &[], &body);
            }
            MutOp::Upgrade(i) => {
                if let Some(handle) = nth(&live(&client), i) {
                    let _ = client.upgrade("inbox", vec![handle], PimdirTier::Full);
                }
            }
            MutOp::Bump => {
                bumps += 1;

                // NOTE: the rekey matches by link id, so claims on
                // link-less placements void with them; staged edits
                // always survive, through carry or resurrect
                let linked: BTreeSet<PimdirHandle> = client
                    .storage()
                    .rows("inbox")
                    .into_iter()
                    .filter(|p| p.link_id.is_some())
                    .map(|p| p.handle)
                    .collect();
                ledger.flags.retain(|handle, _| linked.contains(handle));
                ledger
                    .moves
                    .retain(|(handle, _, _)| linked.contains(handle));

                let mapping = client.remote_mut().renumber("inbox", bumps);
                let _ = client.rekey("inbox");

                ledger.flags = std::mem::take(&mut ledger.flags)
                    .into_iter()
                    .map(|(h, v)| (mapping.get(&h).cloned().unwrap_or(h), v))
                    .collect();
                for staged_move in &mut ledger.moves {
                    if let Some(new) = mapping.get(&staged_move.0) {
                        staged_move.0 = new.clone();
                    }
                }
            }
            MutOp::Sync => {
                let _ = client.sync("inbox", opts);
                hydrate(&mut client, "inbox");
            }
            MutOp::SyncArchive => {
                let _ = client.sync("archive", opts);
            }
        }
    }

    for _ in 0..3 {
        client.sync("inbox", opts).unwrap();
        hydrate(&mut client, "inbox");
        client.sync("archive", opts).unwrap();
    }

    for round in 0..3 {
        let conflicted: Vec<PimdirHandle> = client
            .storage()
            .rows("inbox")
            .into_iter()
            .filter(|p| p.status == PimdirStatus::Conflict)
            .map(|p| p.handle)
            .collect();
        if conflicted.is_empty() {
            break;
        }
        prop_assert!(round < 2, "conflict resolution must terminate");

        // NOTE: the engine fetches the diverging remote body only when a
        // conflict is upgraded
        let _ = client.upgrade("inbox", conflicted.clone(), PimdirTier::Full);
        for handle in conflicted {
            let held = held_object(&client, &handle);
            let server_body = server_body(&client, &handle);
            let (object, body) = resolution_body(&client, &handle);
            if let Some(held) = held {
                ledger.edits.retain(|_, staged| staged != &held);
            }
            void_superseded_edits(&mut ledger, &client, &server_body);
            // NOTE: a resolution adopting the remote body is satisfied by
            // the run doing nothing, so it claims none
            let owed = server_body != body;
            if owed {
                ledger.edits.insert(handle.clone(), hash(&body));
            }
            client
                .mutate(
                    "inbox",
                    PimdirMutation::Edit {
                        sort_key: Default::default(),
                        handle,
                        object,
                        body,
                        summary: None,
                    },
                )
                .unwrap();
        }
        client.sync("inbox", opts).unwrap();
        client.sync("inbox", opts).unwrap();
        hydrate(&mut client, "inbox");
    }

    for _ in 0..2 {
        client.sync("inbox", opts).unwrap();
        hydrate(&mut client, "inbox");
        client.sync("archive", opts).unwrap();
    }

    // NOTE: a copy whose source vanished can never land, so its
    // placeholder staying visibly pending is the accounted end state
    let inbox_server = on_server(&client, "inbox");
    let lingering = |p: &PimdirPlacement| {
        p.status == PimdirStatus::Created
            && p.origin
                .as_ref()
                .is_some_and(|o| !inbox_server.contains(&o.handle))
    };

    for collection in ["inbox", "archive"] {
        let placements = client.storage().rows(collection);

        let local: BTreeSet<PimdirHandle> = placements
            .iter()
            .filter(|p| !lingering(p))
            .map(|p| p.handle.clone())
            .collect();
        prop_assert_eq!(
            &local,
            &on_server(&client, collection),
            "{} mirrors the server",
            collection,
        );

        for placement in placements.iter().filter(|p| !lingering(p)) {
            prop_assert_eq!(
                placement.status,
                PimdirStatus::Clean,
                "nothing pending after resolution: {:?}",
                placement,
            );
            let handle = placement.handle.as_str();
            prop_assert_eq!(
                &placement.flags,
                client.remote().flags_of(collection, handle)
            );
            let server_rev = client.remote().rev_of(collection, handle).to_string();
            prop_assert_eq!(
                placement.base.as_ref().and_then(|b| b.revision.as_deref()),
                Some(server_rev.as_str()),
                "the base revision tracks the server: {:?}",
                placement,
            );
        }
    }

    for (handle, staged) in &ledger.edits {
        let found = client
            .remote()
            .items
            .get(&"inbox".into())
            .into_iter()
            .flatten()
            .any(|(_, item)| item.body == staged.as_str().as_bytes());
        prop_assert!(found, "edit intent on {handle:?} lost: {staged:?}");
    }

    // NOTE: a flag claim holds while its handle exists; a resurrect
    // re-keys it and ends the claim
    for (handle, (added, removed)) in &ledger.flags {
        if let Some(items) = client.remote().items.get(&"inbox".into()) {
            if let Some(item) = items.get(handle) {
                for flag in added {
                    prop_assert!(
                        item.flags.contains(flag),
                        "added flag {flag} on {handle:?} lost",
                    );
                }
                for flag in removed {
                    prop_assert!(
                        !item.flags.contains(flag),
                        "removed flag {flag} on {handle:?} came back",
                    );
                }
            }
        }
    }

    for (placeholder, link) in &ledger.copies {
        let Some(link) = link else { continue };
        let pending = client
            .storage()
            .get("archive", placeholder.as_str())
            .is_some_and(|p| p.status == PimdirStatus::Created);
        prop_assert!(
            collection_has_link(&client, "archive", link) || pending,
            "copy intent {placeholder:?} lost",
        );
    }

    // NOTE: a move that never pushed cannot hide here, since its
    // surviving tombstone fails the all-clean assertion above
    for (handle, link, voided) in &ledger.moves {
        if *voided {
            continue;
        }
        let Some(link) = link else { continue };
        prop_assert!(
            collection_has_link(&client, "archive", link)
                || collection_has_link(&client, "inbox", link),
            "move intent {handle:?} lost",
        );
    }

    let report = client.sync("inbox", opts).unwrap();
    prop_assert_eq!(report, PimdirSyncReport::default());
    let dead_placeholders = client
        .storage()
        .rows("archive")
        .iter()
        .filter(|p| lingering(p))
        .count();
    let report = client.sync("archive", opts).unwrap();
    let expected = PimdirSyncReport {
        rejected: dead_placeholders,
        ..Default::default()
    };
    prop_assert_eq!(report, expected);
    Ok(())
}

proptest! {
    /// After quiescence only content conflicts survive.
    ///
    /// Resolving each with an edit brings the replica to an exact mirror
    /// of the server.
    #[test]
    fn mutable_interleavings_converge_after_resolution(
        ops in proptest::collection::vec(arb_mut_op(), 0..25),
    ) {
        check_mutable_model(ops)?;
    }
}

/// A local edit restating the synced body stages nothing.
///
/// The model must not claim an intent for it and the placement must not
/// read as a pending push.
#[test]
fn a_content_identical_edit_claims_nothing() {
    check_mutable_model(vec![
        MutOp::LocalEdit(1338203356464132091, 125),
        MutOp::Sync,
        MutOp::LocalRemove(1892733528096728582),
        MutOp::ServerRemove(1245933664089759521),
        MutOp::LocalEdit(11104427113558993974, 125),
    ])
    .unwrap();
}

/// A resolution keeping the ancestor is a decision like any other.
///
/// It has to reach the remote, which holds its own diverging body and
/// would otherwise stay ahead of a replica reporting itself in sync.
#[test]
fn a_resolution_keeping_the_ancestor_reaches_the_remote() {
    check_mutable_model(vec![
        MutOp::Upgrade(14210287063717275722),
        MutOp::LocalEdit(8823440174339698637, 0),
        MutOp::ServerRemove(2473669229220409580),
        MutOp::LocalSetFlags(0, PimdirFlags::from_iter(Vec::<String>::new())),
        MutOp::Bump,
        MutOp::ServerEdit(4132879379286324521, 0),
    ])
    .unwrap();
}

/// A fake remote shared by two replicas, one server behind two devices.
#[derive(Clone)]
struct SharedRemote(Rc<RefCell<MemRemote>>);

impl PimdirRemote for SharedRemote {
    type Error = Infallible;

    fn enumerate(
        &mut self,
        collection: &PimdirCollectionId,
        cursor: Option<PimdirCheckpoint>,
    ) -> Result<PimdirRemoteSnapshot, Infallible> {
        self.0.borrow_mut().enumerate(collection, cursor)
    }

    fn fetch(
        &mut self,
        collection: &PimdirCollectionId,
        handles: Vec<PimdirHandle>,
        tier: PimdirTier,
    ) -> Result<Vec<PimdirFetchedItem>, Infallible> {
        self.0.borrow_mut().fetch(collection, handles, tier)
    }

    fn push(
        &mut self,
        collection: &PimdirCollectionId,
        changes: Vec<PimdirChange>,
    ) -> Result<Vec<PimdirPushResult>, Infallible> {
        self.0.borrow_mut().push(collection, changes)
    }
}

type Replica = Client<SharedRemote>;

/// One step of the two-replica scenario.
///
/// Replica A edits locally and syncs full every time, replica B is a
/// passive incremental mirror.
#[derive(Clone, Debug)]
enum PairOp {
    LocalASetFlags(usize, PimdirFlags),
    LocalARemove(usize),
    ServerSetFlags(usize, PimdirFlags),
    ServerEdit(usize, u8),
    ServerRemove(usize),
    ServerAdd(u8),
    SyncA,
    SyncB,
}

fn arb_pair_op() -> impl Strategy<Value = PairOp> {
    prop_oneof![
        (any::<usize>(), arb_flags()).prop_map(|(i, f)| PairOp::LocalASetFlags(i, f)),
        any::<usize>().prop_map(PairOp::LocalARemove),
        (any::<usize>(), arb_flags()).prop_map(|(i, f)| PairOp::ServerSetFlags(i, f)),
        (any::<usize>(), any::<u8>()).prop_map(|(i, n)| PairOp::ServerEdit(i, n)),
        any::<usize>().prop_map(PairOp::ServerRemove),
        any::<u8>().prop_map(PairOp::ServerAdd),
        Just(PairOp::SyncA),
        Just(PairOp::SyncB),
    ]
}

proptest! {
    /// A full-sync replica and a delta replica end in the same state.
    ///
    /// The delta path is equivalent to re-reading the whole collection.
    #[test]
    fn full_and_delta_replicas_agree(ops in proptest::collection::vec(arb_pair_op(), 0..25)) {
        let mut server = MemRemote::default();
        server.mutable = true;
        server.seed("inbox", "m1", "l1", &[], b"one");
        server.seed("inbox", "m2", "l2", &["seen"], b"two");
        server.seed("inbox", "m3", "l3", &["flagged"], b"three");
        server.seed("inbox", "m4", "l4", &["seen", "draft"], b"four");
        server.seed("inbox", "m5", "l5", &[], b"five");
        let server = Rc::new(RefCell::new(server));

        let full_opts = PimdirSyncOptions {
            full: true,
            ..PimdirSyncOptions::default()
        };
        let delta_opts = PimdirSyncOptions::default();
        let mut a = Client::new(SharedRemote(server.clone()));
        let mut b = Client::new(SharedRemote(server.clone()));
        a.sync("inbox", full_opts).unwrap();
        hydrate(&mut a, "inbox");
        b.sync("inbox", delta_opts).unwrap();
        hydrate(&mut b, "inbox");

        let on_server = || -> BTreeSet<PimdirHandle> { server.borrow().handles("inbox") };

        for op in ops {
            match op {
                PairOp::LocalASetFlags(i, flags) => {
                    if let Some(handle) = nth(&named(&a, "inbox"), i) {
                        a.mutate("inbox", PimdirMutation::SetFlags { handle, flags }).unwrap();
                    }
                }
                PairOp::LocalARemove(i) => {
                    if let Some(handle) = nth(&named(&a, "inbox"), i) {
                        a.mutate("inbox", PimdirMutation::Remove(handle)).unwrap();
                    }
                }
                PairOp::ServerSetFlags(i, flags) => {
                    if let Some(handle) = nth(&on_server(), i) {
                        let flags: Vec<&str> = flags.known().into_iter().flatten().map(|f| f.as_str()).collect();
                        server.borrow_mut().set_flags("inbox", handle.as_str(), &flags);
                    }
                }
                PairOp::ServerEdit(i, n) => {
                    if let Some(handle) = nth(&on_server(), i) {
                        let body = format!("srv-edit-{n}").into_bytes();
                        server.borrow_mut().edit("inbox", handle.as_str(), &body);
                    }
                }
                PairOp::ServerRemove(i) => {
                    if let Some(handle) = nth(&on_server(), i) {
                        server.borrow_mut().remove("inbox", handle.as_str());
                    }
                }
                PairOp::ServerAdd(n) => {
                    let handle = format!("srv-{n}");
                    let link = format!("lnk-{n}");
                    server.borrow_mut().seed("inbox", &handle, &link, &[], b"new");
                }
                PairOp::SyncA => {
                    a.sync("inbox", full_opts).unwrap();
                    hydrate(&mut a, "inbox");
                }
                PairOp::SyncB => {
                    b.sync("inbox", delta_opts).unwrap();
                    hydrate(&mut b, "inbox");
                }
            }
        }

        for _ in 0..3 {
            a.sync("inbox", full_opts).unwrap();
            hydrate(&mut a, "inbox");
            b.sync("inbox", delta_opts).unwrap();
            hydrate(&mut b, "inbox");
        }

        let placements_a: Vec<PimdirPlacement> = a.storage().placements().into_values().collect();
        let placements_b: Vec<PimdirPlacement> = b.storage().placements().into_values().collect();
        prop_assert_eq!(
            placements_a,
            placements_b,
            "the full-sync replica and the delta replica diverged",
        );
    }
}

/// One step with two replicas editing the same server concurrently.
#[derive(Clone, Debug)]
enum DuoOp {
    ASetFlags(usize, PimdirFlags),
    AEdit(usize, u8),
    ARemove(usize),
    BSetFlags(usize, PimdirFlags),
    BEdit(usize, u8),
    BRemove(usize),
    ServerAdd(u8),
    SyncA,
    SyncB,
}

fn arb_duo_op() -> impl Strategy<Value = DuoOp> {
    prop_oneof![
        (any::<usize>(), arb_flags()).prop_map(|(i, f)| DuoOp::ASetFlags(i, f)),
        (any::<usize>(), any::<u8>()).prop_map(|(i, n)| DuoOp::AEdit(i, n)),
        any::<usize>().prop_map(DuoOp::ARemove),
        (any::<usize>(), arb_flags()).prop_map(|(i, f)| DuoOp::BSetFlags(i, f)),
        (any::<usize>(), any::<u8>()).prop_map(|(i, n)| DuoOp::BEdit(i, n)),
        any::<usize>().prop_map(DuoOp::BRemove),
        any::<u8>().prop_map(DuoOp::ServerAdd),
        Just(DuoOp::SyncA),
        Just(DuoOp::SyncB),
    ]
}

fn duo_mutate(
    client: &mut Replica,
    index: usize,
    mutation: impl Fn(PimdirHandle) -> PimdirMutation,
) {
    if let Some(handle) = nth(&named(client, "inbox"), index) {
        client.mutate("inbox", mutation(handle)).unwrap();
    }
}

fn duo_edit(client: &mut Replica, index: usize, tag: &str, n: u8) {
    if let Some(handle) = nth(&named(client, "inbox"), index) {
        let body = format!("edit-{tag}-{n}-{}", handle.as_str()).into_bytes();
        let object = PimdirObject {
            hash: hash(&body),
            size: body.len(),
        };
        client
            .mutate(
                "inbox",
                PimdirMutation::Edit {
                    sort_key: Default::default(),
                    handle,
                    object,
                    body,
                    summary: None,
                },
            )
            .unwrap();
    }
}

/// Resolves every content conflict on one replica with an edit.
fn duo_resolve(client: &mut Replica, tag: &str) -> bool {
    let conflicted: Vec<PimdirHandle> = client
        .storage()
        .rows("inbox")
        .into_iter()
        .filter(|p| p.status == PimdirStatus::Conflict)
        .map(|p| p.handle)
        .collect();
    if conflicted.is_empty() {
        return false;
    }
    for handle in conflicted {
        let body = format!("resolved-{tag}-{}", handle.as_str()).into_bytes();
        let object = PimdirObject {
            hash: hash(&body),
            size: body.len(),
        };
        client
            .mutate(
                "inbox",
                PimdirMutation::Edit {
                    sort_key: Default::default(),
                    handle,
                    object,
                    body,
                    summary: None,
                },
            )
            .unwrap();
    }
    true
}

/// Syncs one replica and names what it pulled.
fn duo_sync(client: &mut Replica, opts: PimdirSyncOptions) {
    client.sync("inbox", opts).unwrap();
    hydrate(client, "inbox");
}

proptest! {
    /// Two replicas actively editing one server converge onto its state.
    ///
    /// Quiescence plus per-replica conflict resolution leaves nothing
    /// pending anywhere and the final syncs idempotent.
    #[test]
    fn two_active_replicas_converge(ops in proptest::collection::vec(arb_duo_op(), 0..25)) {
        let mut server = MemRemote::default();
        server.mutable = true;
        server.seed("inbox", "m1", "l1", &[], b"one");
        server.seed("inbox", "m2", "l2", &["seen"], b"two");
        server.seed("inbox", "m3", "l3", &["flagged"], b"three");
        server.seed("inbox", "m4", "l4", &["seen", "draft"], b"four");
        server.seed("inbox", "m5", "l5", &[], b"five");
        let server = Rc::new(RefCell::new(server));

        let opts = PimdirSyncOptions::default();
        let mut a = Client::new(SharedRemote(server.clone()));
        let mut b = Client::new(SharedRemote(server.clone()));
        duo_sync(&mut a, opts);
        duo_sync(&mut b, opts);

        let mut arrivals = 0usize;
        for op in ops {
            match op {
                DuoOp::ASetFlags(i, flags) => {
                    duo_mutate(&mut a, i, |handle| PimdirMutation::SetFlags {
                        handle,
                        flags: flags.clone(),
                    });
                }
                DuoOp::AEdit(i, n) => duo_edit(&mut a, i, "a", n),
                DuoOp::ARemove(i) => duo_mutate(&mut a, i, PimdirMutation::Remove),
                DuoOp::BSetFlags(i, flags) => {
                    duo_mutate(&mut b, i, |handle| PimdirMutation::SetFlags {
                        handle,
                        flags: flags.clone(),
                    });
                }
                DuoOp::BEdit(i, n) => duo_edit(&mut b, i, "b", n),
                DuoOp::BRemove(i) => duo_mutate(&mut b, i, PimdirMutation::Remove),
                DuoOp::ServerAdd(n) => {
                    arrivals += 1;
                    let handle = format!("srv-{arrivals}");
                    let link = format!("lnk-{arrivals}");
                    let body = format!("new-{n}").into_bytes();
                    server.borrow_mut().seed("inbox", &handle, &link, &[], &body);
                }
                DuoOp::SyncA => duo_sync(&mut a, opts),
                DuoOp::SyncB => duo_sync(&mut b, opts),
            }
        }

        // NOTE: one resolution can conflict with the other, so the rounds
        // ping-pong at most once before settling
        for _ in 0..4 {
            duo_sync(&mut a, opts);
            duo_sync(&mut b, opts);
        }
        for round in 0..4 {
            let unresolved_a = duo_resolve(&mut a, "a");
            if unresolved_a {
                duo_sync(&mut a, opts);
                duo_sync(&mut a, opts);
            }
            let unresolved_b = duo_resolve(&mut b, "b");
            if unresolved_b {
                duo_sync(&mut b, opts);
                duo_sync(&mut b, opts);
            }
            if !unresolved_a && !unresolved_b {
                break;
            }
            prop_assert!(round < 3, "conflict resolution must terminate");
            duo_sync(&mut a, opts);
            duo_sync(&mut b, opts);
        }
        for _ in 0..2 {
            duo_sync(&mut a, opts);
            duo_sync(&mut b, opts);
        }

        let on_server: BTreeSet<PimdirHandle> = server.borrow().handles("inbox");

        for (name, replica) in [("a", &a), ("b", &b)] {
            let placements = replica.storage().rows("inbox");
            let handles: BTreeSet<PimdirHandle> = placements
                .iter()
                .map(|p| p.handle.clone())
                .collect();
            prop_assert_eq!(&handles, &on_server, "replica {} mirrors the server", name);

            for placement in &placements {
                prop_assert_eq!(
                    placement.status,
                    PimdirStatus::Clean,
                    "nothing pending on replica {}: {:?}",
                    name,
                    placement,
                );
                let server = server.borrow();
                let item = server
                    .items
                    .get(&"inbox".into())
                    .and_then(|c| c.get(&placement.handle))
                    .expect("mirrored member");
                prop_assert_eq!(&placement.flags, &item.flags);
                let server_rev = item.rev.to_string();
                prop_assert_eq!(
                    placement.base.as_ref().and_then(|base| base.revision.as_deref()),
                    Some(server_rev.as_str()),
                    "the base revision tracks the server",
                );
            }
        }

        let report = a.sync("inbox", opts).unwrap();
        prop_assert_eq!(report, PimdirSyncReport::default());
        let report = b.sync("inbox", opts).unwrap();
        prop_assert_eq!(report, PimdirSyncReport::default());
    }
}
