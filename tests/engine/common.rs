//! # Fake remote and store harness
//!
//! Shared backends for the engine tests: a fake remote, and a client
//! pairing it with one source handle over a real store.
//!
//! The fake remote models both backend families: with `mutable` unset it
//! behaves like IMAP, with it set like WebDAV (per-item revisions, stale
//! if_match rejections). It serves delta snapshots from a numeric
//! checkpoint with explicit vanished tracking.

use std::{
    collections::{BTreeMap, BTreeSet},
    convert::Infallible,
    fmt,
    rc::Rc,
};

use io_pimdir::{
    change::{PimdirChange, PimdirChangeKind},
    client::{PimdirSourceStore, PimdirStore, reader::PimdirItem},
    collection::{PimdirCheckpoint, PimdirCollectionId},
    hub::PimdirHub,
    load::{PimdirLoadScope, PimdirLoaded},
    mutate::PimdirMutation,
    object::PimdirHash,
    placement::{
        PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement, PimdirStatus,
    },
    rekey::PimdirRekeyReport,
    remote::{
        PimdirFetchedBody, PimdirFetchedItem, PimdirPushOutcome, PimdirPushResult, PimdirRemote,
        PimdirRemoteItem, PimdirRemoteSnapshot, PimdirTier,
    },
    summary::{PimdirSummary, mail::PimdirMailSummary},
    sync::{PimdirSyncOptions, PimdirSyncReport},
    upgrade::PimdirUpgradeReport,
};
use tempfile::TempDir;

/// One source handle over a real store, paired with its remote.
///
/// Every verb runs through the store's runner; errors come back as their
/// display, the run error type being unnameable from outside the crate.
pub struct Client<R = MemRemote> {
    pub store: PimdirSourceStore,
    pub remote: R,
    dir: Rc<TempDir>,
}

impl<R: PimdirRemote> Client<R>
where
    R::Error: fmt::Debug + fmt::Display,
{
    /// A fresh store acting as the source `left`.
    pub fn new(remote: R) -> Self {
        let dir = Rc::new(tempfile::tempdir().expect("a temp dir"));
        Self::over(dir, remote, "left")
    }

    /// A fresh store acting as `source`.
    pub fn with_source(remote: R, source: &str) -> Self {
        let dir = Rc::new(tempfile::tempdir().expect("a temp dir"));
        Self::over(dir, remote, source)
    }

    /// A second source over this client's store: the hub shape.
    pub fn sharing(&self, remote: R, source: &str) -> Self {
        Self::over(Rc::clone(&self.dir), remote, source)
    }

    fn over(dir: Rc<TempDir>, remote: R, source: &str) -> Self {
        let store = PimdirStore::open(dir.path())
            .expect("the store opens")
            .for_source(source);

        Self { store, remote, dir }
    }

    pub fn remote(&self) -> &R {
        &self.remote
    }

    pub fn remote_mut(&mut self) -> &mut R {
        &mut self.remote
    }

    /// The store as this client reads it.
    pub fn storage(&self) -> Storage<'_> {
        Storage(&self.store)
    }

    /// The collection's whole hub, every source included.
    pub fn hub(&self, collection: &str) -> PimdirHub {
        self.store.load_hub(collection).expect("the hub loads")
    }

    pub fn sync(
        &mut self,
        collection: &str,
        opts: PimdirSyncOptions,
    ) -> Result<PimdirSyncReport, String> {
        self.store
            .sync(collection, opts, &mut self.remote)
            .map_err(|err| err.to_string())
    }

    pub fn upgrade(
        &mut self,
        collection: &str,
        handles: Vec<PimdirHandle>,
        tier: PimdirTier,
    ) -> Result<PimdirUpgradeReport, String> {
        self.store
            .upgrade(collection, handles, tier, &mut self.remote)
            .map_err(|err| err.to_string())
    }

    pub fn mutate(&mut self, collection: &str, mutation: PimdirMutation) -> Result<(), String> {
        self.store
            .mutate(collection, mutation)
            .map_err(|err| err.to_string())
    }

    pub fn rekey(&mut self, collection: &str) -> Result<PimdirRekeyReport, String> {
        self.store
            .rekey(collection, &mut self.remote)
            .map_err(|err| err.to_string())
    }

    pub fn open(&mut self, collection: &str) -> Result<PimdirLoaded, String> {
        self.store
            .open_collection(collection)
            .map_err(|err| err.to_string())
    }
}

/// The store as one source projects it: what `load` answers, read whole.
pub struct Storage<'a>(&'a PimdirSourceStore);

impl Storage<'_> {
    /// Every placement of `collection`, probes included.
    pub fn rows(&self, collection: &str) -> Vec<PimdirPlacement> {
        self.0
            .load(&collection.into(), &PimdirLoadScope::All)
            .expect("the collection loads")
            .placements
    }

    /// Every placement of every collection, keyed like a row.
    pub fn placements(&self) -> BTreeMap<(PimdirCollectionId, PimdirHandle), PimdirPlacement> {
        self.0
            .list_collections()
            .expect("the collections list")
            .into_iter()
            .flat_map(|collection| self.rows(&collection.id))
            .map(|p| ((p.collection.clone(), p.handle.clone()), p))
            .collect()
    }

    /// The placement of `handle` in `collection`, if the source holds one.
    pub fn get(&self, collection: &str, handle: &str) -> Option<PimdirPlacement> {
        let handle = PimdirHandle::from(handle);
        self.0
            .load(
                &collection.into(),
                &PimdirLoadScope::Handles(vec![handle.clone()]),
            )
            .expect("the collection loads")
            .placements
            .into_iter()
            .find(|p| p.handle == handle)
    }

    /// The placement of `handle` in `collection`.
    pub fn placement(&self, collection: &str, handle: &str) -> PimdirPlacement {
        self.get(collection, handle)
            .unwrap_or_else(|| panic!("placement {collection}/{handle} exists"))
    }

    pub fn contains(&self, collection: &str, handle: &str) -> bool {
        self.get(collection, handle).is_some()
    }

    /// How many bodies the store indexes.
    pub fn objects(&self) -> u64 {
        self.0.object_stats().expect("the stats read").count
    }

    /// The bytes stored under `hash`, if any.
    pub fn body(&self, hash: &PimdirHash) -> Option<Vec<u8>> {
        self.0.blobs().get(hash).expect("the blob reads")
    }

    pub fn checkpoint(&self, collection: &str) -> Option<PimdirCheckpoint> {
        self.0
            .load(&collection.into(), &PimdirLoadScope::Links(vec![]))
            .expect("the collection loads")
            .checkpoint
    }

    /// The items retention holds for `collection`, the trash view.
    pub fn retained(&self, collection: &str) -> Vec<PimdirItem> {
        self.0
            .list_retained(&collection.into(), None, usize::MAX)
            .expect("the trash reads")
    }
}

#[derive(Clone)]
pub struct ServerItem {
    pub link_id: PimdirLinkId,
    pub flags: PimdirFlags,
    pub body: Vec<u8>,
    /// The change counter value of the item's last change, feeding deltas.
    pub seq: usize,
    /// The content revision, reported only when the remote is `mutable`.
    pub rev: usize,
}

#[derive(Default)]
pub struct MemRemote {
    pub items: BTreeMap<PimdirCollectionId, BTreeMap<PimdirHandle, ServerItem>>,
    pub full_fetches: Vec<PimdirHandle>,
    pub calls: usize,
    /// The handle counter for the next accepted append without an origin.
    pub next_appended: usize,
    /// The size of each push batch handed over, in order.
    pub push_batches: Vec<usize>,
    /// Whether to report revisions and reject stale if_match, like WebDAV.
    pub mutable: bool,
    /// The identities this remote refuses to append.
    ///
    /// How a DAV server answers `no-uid-conflict` to a resource whose
    /// `UID` its collection already holds.
    pub refused_appends: BTreeSet<PimdirLinkId>,
    /// The global change counter; a delta serves everything past it.
    seq: usize,
    /// Handles removed, stamped with the counter value of their removal.
    vanished: BTreeMap<PimdirCollectionId, Vec<(usize, PimdirHandle)>>,
}

impl MemRemote {
    fn bump(&mut self) -> usize {
        self.seq += 1;
        self.seq
    }

    fn revision(&self, rev: usize) -> Option<String> {
        self.mutable.then(|| rev.to_string())
    }

    pub fn seed(
        &mut self,
        collection: &str,
        handle: &str,
        link: &str,
        flags: &[&str],
        body: &[u8],
    ) {
        let seq = self.bump();
        let collection = self.items.entry(collection.into()).or_default();
        let rev = collection
            .get(&PimdirHandle::from(handle))
            .map(|i| i.rev + 1)
            .unwrap_or(0);

        collection.insert(
            PimdirHandle::from(handle),
            ServerItem {
                link_id: PimdirLinkId::from(link),
                flags: PimdirFlags::from_iter(flags.iter().copied()),
                body: body.to_vec(),
                seq,
                rev,
            },
        );
    }

    pub fn set_flags(&mut self, collection: &str, handle: &str, flags: &[&str]) {
        let seq = self.bump();
        let item = self
            .items
            .get_mut(&collection.into())
            .and_then(|c| c.get_mut(&PimdirHandle::from(handle)))
            .expect("server item exists");
        item.flags = PimdirFlags::from_iter(flags.iter().copied());
        item.seq = seq;
    }

    /// A server-side content edit: the revision advances.
    pub fn edit(&mut self, collection: &str, handle: &str, body: &[u8]) {
        let seq = self.bump();
        let item = self
            .items
            .get_mut(&collection.into())
            .and_then(|c| c.get_mut(&PimdirHandle::from(handle)))
            .expect("server item exists");
        item.body = body.to_vec();
        item.seq = seq;
        item.rev += 1;
    }

    pub fn remove(&mut self, collection: &str, handle: &str) {
        let seq = self.bump();
        let collection = PimdirCollectionId::from(collection);
        self.items
            .get_mut(&collection)
            .and_then(|c| c.remove(&PimdirHandle::from(handle)))
            .expect("server item exists");
        self.vanished
            .entry(collection)
            .or_default()
            .push((seq, PimdirHandle::from(handle)));
    }

    /// Renumbers every member onto a fresh handle, as a UIDVALIDITY bump.
    ///
    /// Contents are untouched. Returns the old-to-new mapping.
    pub fn renumber(
        &mut self,
        collection: &str,
        generation: usize,
    ) -> BTreeMap<PimdirHandle, PimdirHandle> {
        let seq = self.bump();
        let members = self.items.entry(collection.into()).or_default();
        let mut mapping = BTreeMap::new();

        let old = std::mem::take(members);
        for (index, (handle, mut item)) in old.into_iter().enumerate() {
            let new = PimdirHandle::from(format!("v{generation}-{index}"));
            item.seq = seq;
            mapping.insert(handle, new.clone());
            members.insert(new, item);
        }

        mapping
    }

    pub fn flags_of(&self, collection: &str, handle: &str) -> &PimdirFlags {
        &self.item(collection, handle).flags
    }

    pub fn rev_of(&self, collection: &str, handle: &str) -> usize {
        self.item(collection, handle).rev
    }

    /// The handles `collection` holds, in order.
    pub fn handles(&self, collection: &str) -> BTreeSet<PimdirHandle> {
        self.items
            .get(&collection.into())
            .map(|c| c.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// The member of `collection` holding `link`, ignoring `except`.
    ///
    /// The delivery key both halves of a move check before delivering.
    fn link_holder(
        &self,
        collection: &PimdirCollectionId,
        link: Option<&PimdirLinkId>,
        except: Option<&PimdirHandle>,
    ) -> Option<PimdirHandle> {
        let link = link?;
        self.items
            .get(collection)?
            .iter()
            .find(|(handle, item)| &item.link_id == link && Some(*handle) != except)
            .map(|(handle, _)| handle.clone())
    }

    fn holds_link(
        &self,
        collection: &PimdirCollectionId,
        link: Option<&PimdirLinkId>,
        except: Option<&PimdirHandle>,
    ) -> bool {
        self.link_holder(collection, link, except).is_some()
    }

    fn item(&self, collection: &str, handle: &str) -> &ServerItem {
        self.items
            .get(&collection.into())
            .and_then(|c| c.get(&PimdirHandle::from(handle)))
            .expect("server item exists")
    }
}

/// A tiny deterministic hash: identical bytes collapse to one object.
pub fn hash(body: &[u8]) -> PimdirHash {
    let mut acc: u64 = 1469598103934665603;
    for byte in body {
        acc ^= *byte as u64;
        acc = acc.wrapping_mul(1099511628211);
    }
    PimdirHash::from(format!("{acc:016x}"))
}

/// The summary the fake remote derives for a handle: a mail row whose
/// subject names it, enough for the `Meta` tier to count as reached.
pub fn summary_of(handle: &PimdirHandle) -> PimdirSummary {
    PimdirSummary::Mail(PimdirMailSummary {
        subject: format!("headers:{}", handle.as_str()),
        ..Default::default()
    })
}

impl PimdirRemote for MemRemote {
    type Error = Infallible;

    fn enumerate(
        &mut self,
        collection: &PimdirCollectionId,
        cursor: Option<PimdirCheckpoint>,
    ) -> Result<PimdirRemoteSnapshot, Infallible> {
        self.calls += 1;
        let checkpoint = PimdirCheckpoint(self.seq.to_string().into_bytes());

        let since = cursor
            .as_ref()
            .and_then(|c| std::str::from_utf8(&c.0).ok())
            .and_then(|s| s.parse::<usize>().ok());

        let members = self.items.get(collection);

        let (items, vanished, complete) = match since {
            Some(since) => {
                let items = members
                    .into_iter()
                    .flatten()
                    .filter(|(_, item)| item.seq > since)
                    .map(|(handle, item)| PimdirRemoteItem {
                        handle: handle.clone(),
                        flags: item.flags.clone(),
                        revision: self.revision(item.rev),
                    })
                    .collect();
                // NOTE: a handle listed again is not vanished, the fake
                // reporting current truth where a server never reuses one.
                let vanished = self
                    .vanished
                    .get(collection)
                    .into_iter()
                    .flatten()
                    .filter(|(seq, handle)| {
                        *seq > since && !members.is_some_and(|c| c.contains_key(handle))
                    })
                    .map(|(_, handle)| handle.clone())
                    .collect();
                (items, vanished, false)
            }
            None => {
                let items = members
                    .into_iter()
                    .flatten()
                    .map(|(handle, item)| PimdirRemoteItem {
                        handle: handle.clone(),
                        flags: item.flags.clone(),
                        revision: self.revision(item.rev),
                    })
                    .collect();
                (items, Vec::new(), true)
            }
        };

        Ok(PimdirRemoteSnapshot {
            items,
            vanished,
            complete,
            checkpoint,
        })
    }

    fn fetch(
        &mut self,
        collection: &PimdirCollectionId,
        handles: Vec<PimdirHandle>,
        tier: PimdirTier,
    ) -> Result<Vec<PimdirFetchedItem>, Infallible> {
        self.calls += 1;

        let collection = self.items.get(collection).cloned().unwrap_or_default();
        let mut out = Vec::new();

        for handle in handles {
            let Some(item) = collection.get(&handle) else {
                continue;
            };

            let body = match tier {
                PimdirTier::Meta => None,
                PimdirTier::Full => {
                    self.full_fetches.push(handle.clone());
                    Some(PimdirFetchedBody::Inline {
                        hash: hash(&item.body),
                        bytes: item.body.clone(),
                    })
                }
            };

            out.push(PimdirFetchedItem {
                summary: Some(summary_of(&handle)),
                sort_key: Default::default(),
                handle,
                link_id: item.link_id.clone(),
                body,
                revision: self.revision(item.rev),
            });
        }

        Ok(out)
    }

    fn push(
        &mut self,
        collection: &PimdirCollectionId,
        changes: Vec<PimdirChange>,
    ) -> Result<Vec<PimdirPushResult>, Infallible> {
        self.calls += 1;
        self.push_batches.push(changes.len());
        let mut results = Vec::new();

        for change in changes {
            let result = match change.kind {
                PimdirChangeKind::SetFlags { handle, flags } => {
                    let seq = self.bump();
                    if let Some(item) = self
                        .items
                        .get_mut(collection)
                        .and_then(|c| c.get_mut(&handle))
                    {
                        item.flags = flags;
                        item.seq = seq;
                    }
                    accepted(handle, None, None)
                }
                PimdirChangeKind::Remove {
                    handle,
                    to,
                    link_id,
                    if_match,
                } => {
                    // NOTE: a target already holding the item got it from
                    // the move's other half, so this is a plain delete.
                    let to = to
                        .filter(|target| !self.holds_link(target, link_id.as_ref(), Some(&handle)));
                    let stale = self.mutable
                        && self
                            .items
                            .get(collection)
                            .and_then(|c| c.get(&handle))
                            .is_some_and(|item| {
                                if_match.as_deref() != Some(item.rev.to_string().as_str())
                            });
                    if stale {
                        rejected(handle)
                    } else {
                        let seq = self.bump();
                        // NOTE: a missing member is an accept by contract,
                        // the delete having already landed.
                        if let Some(item) = self
                            .items
                            .get_mut(collection)
                            .and_then(|c| c.remove(&handle))
                        {
                            self.vanished
                                .entry(collection.clone())
                                .or_default()
                                .push((seq, handle.clone()));
                            if let Some(target) = to {
                                let moved =
                                    PimdirHandle::from(format!("{}-moved", handle.as_str()));
                                let mut item = item;
                                item.seq = seq;
                                self.items.entry(target).or_default().insert(moved, item);
                            }
                        }
                        accepted(handle, None, None)
                    }
                }
                PimdirChangeKind::Update {
                    handle,
                    object,
                    if_match,
                } => {
                    let Some(item) = self
                        .items
                        .get_mut(collection)
                        .and_then(|c| c.get_mut(&handle))
                    else {
                        results.push(rejected(handle));
                        continue;
                    };
                    let stale =
                        self.mutable && if_match.as_deref() != Some(item.rev.to_string().as_str());
                    if stale {
                        rejected(handle)
                    } else {
                        item.body = object.as_str().as_bytes().to_vec();
                        item.rev += 1;
                        let rev = item.rev;
                        let seq = self.bump();
                        let item = self
                            .items
                            .get_mut(collection)
                            .and_then(|c| c.get_mut(&handle))
                            .expect("just updated");
                        item.seq = seq;
                        let revision = self.revision(rev);
                        accepted(handle, None, revision)
                    }
                }
                PimdirChangeKind::Add {
                    handle,
                    link_id,
                    flags,
                    origin,
                    object,
                } => {
                    if link_id
                        .as_ref()
                        .is_some_and(|link| self.refused_appends.contains(link))
                    {
                        results.push(rejected(handle));
                        continue;
                    }
                    let assigned = match origin {
                        Some(o) => {
                            let item = self
                                .items
                                .get(&o.collection)
                                .and_then(|c| c.get(&o.handle))
                                .cloned();
                            let Some(mut item) = item else {
                                results.push(rejected(handle));
                                continue;
                            };
                            let seq = self.bump();
                            let new = PimdirHandle::from(format!("{}-copy", o.handle.as_str()));
                            item.seq = seq;
                            self.items
                                .entry(collection.clone())
                                .or_default()
                                .insert(new.clone(), item);
                            Some(new)
                        }
                        None => object.as_ref().map(|object| {
                            let seq = self.bump();
                            self.next_appended += 1;
                            let new = PimdirHandle::from(format!("app-{}", self.next_appended));
                            let link = link_id
                                .clone()
                                .unwrap_or_else(|| PimdirLinkId::from(new.as_str()));
                            self.items.entry(collection.clone()).or_default().insert(
                                new.clone(),
                                ServerItem {
                                    link_id: link,
                                    flags: flags.clone(),
                                    body: object.as_str().as_bytes().to_vec(),
                                    seq,
                                    rev: 0,
                                },
                            );
                            new
                        }),
                    };
                    let revision = assigned
                        .as_ref()
                        .and_then(|new| self.items.get(collection)?.get(new))
                        .map(|item| item.rev)
                        .and_then(|rev| self.revision(rev));
                    accepted(handle, assigned, revision)
                }
            };

            results.push(result);
        }

        Ok(results)
    }
}

fn accepted(
    handle: PimdirHandle,
    assigned: Option<PimdirHandle>,
    revision: Option<String>,
) -> PimdirPushResult {
    PimdirPushResult {
        handle,
        outcome: PimdirPushOutcome::Accepted,
        assigned,
        revision,
    }
}

fn rejected(handle: PimdirHandle) -> PimdirPushResult {
    PimdirPushResult {
        handle,
        outcome: PimdirPushOutcome::Rejected,
        assigned: None,
        revision: None,
    }
}

/// The shapes no verb may leave a row in, naming the first rule broken.
///
/// Shared by the property models, since these are engine-side laws: a row
/// breaking one is a row no later run can act on.
pub fn malformed(placement: &PimdirPlacement) -> Option<String> {
    let broken = |rule: &str| Some(format!("{rule}: {placement:?}"));

    if placement.staged_edit().is_some() && placement.status == PimdirStatus::Clean {
        return broken("a clean row holds a staged body");
    }
    if placement.level == PimdirLevel::Full && placement.object.is_none() {
        return broken("a full row holds no body");
    }
    if placement.status == PimdirStatus::Created && placement.base.is_some() {
        return broken("a create carries a base");
    }
    if placement.conflict_object.is_some() && placement.conflict_revision.is_none() {
        return broken("a conflict body outlives its revision");
    }
    // NOTE: a tombstone keeps the divergence it is deleting, so a refused
    // delete restores the conflict rather than settling it locally.
    if placement.conflict_revision.is_some()
        && !matches!(
            placement.status,
            PimdirStatus::Conflict | PimdirStatus::Tombstone
        )
    {
        return broken("an unconflicted row tracks a conflict revision");
    }
    if placement.origin.is_some()
        && !matches!(
            placement.status,
            PimdirStatus::Created | PimdirStatus::Tombstone
        )
    {
        return broken("a settled row carries a move destination");
    }

    None
}

/// The law a collection is keyed on: no two live rows carry one link id.
///
/// Tombstones are exempt on purpose: a row on its way out holds no key
/// against a create, so an `Add` may re-create an identity the user
/// deleted before the remove was pushed.
pub fn duplicate_key(placements: &[PimdirPlacement]) -> Option<String> {
    let mut holders: BTreeMap<(PimdirCollectionId, PimdirLinkId), Vec<PimdirHandle>> =
        BTreeMap::new();

    for placement in placements {
        if placement.status == PimdirStatus::Tombstone {
            continue;
        }
        if let Some(link) = placement.link_id.clone() {
            holders
                .entry((placement.collection.clone(), link))
                .or_default()
                .push(placement.handle.clone());
        }
    }

    holders
        .into_iter()
        .find(|(_, handles)| handles.len() > 1)
        .map(|((collection, link), handles)| {
            format!("{} holds {link:?} on {handles:?}", collection.as_str(),)
        })
}
