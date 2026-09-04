//! # The runner
//!
//! Runs a verb to completion (SYNC §1): the storage yields are serviced
//! by this store, the remote ones by the consumer's [`PimdirRemote`]. A
//! runner of its own, one servicing the remote over JNI say, resumes the
//! coroutine itself and hands the storage yields to [`service`].
//!
//! [`service`]: PimdirSourceStore::service

use core::{convert::Infallible, error, fmt};

use alloc::vec::Vec;

use crate::{
    client::{PimdirError, PimdirSourceStore},
    collection::{PimdirCheckpoint, PimdirCollectionId},
    coroutine::*,
    load::PimdirLoaded,
    mutate::{PimdirMutate, PimdirMutateError, PimdirMutation},
    open::PimdirOpen,
    placement::PimdirHandle,
    rekey::{PimdirRekey, PimdirRekeyReport},
    remote::{PimdirFetchedItem, PimdirPushResult, PimdirRemote, PimdirRemoteSnapshot, PimdirTier},
    sync::{PimdirSync, PimdirSyncOptions, PimdirSyncReport},
    upgrade::{PimdirUpgrade, PimdirUpgradeReport},
};

/// Why a run stopped short of its report.
#[derive(Debug)]
pub enum PimdirRunError<R, C> {
    /// The store refused a load or a write.
    Store(PimdirError),
    /// The remote seam failed.
    Remote(R),
    /// The coroutine itself completed with an error.
    Coroutine(C),
}

impl<R: fmt::Display, C: fmt::Display> fmt::Display for PimdirRunError<R, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(err) => write!(f, "Pimdir store failed: {err}"),
            Self::Remote(err) => write!(f, "Pimdir remote failed: {err}"),
            Self::Coroutine(err) => write!(f, "Pimdir engine failed: {err}"),
        }
    }
}

impl<R: fmt::Display + fmt::Debug, C: fmt::Display + fmt::Debug> error::Error
    for PimdirRunError<R, C>
{
}

impl<R, C> From<PimdirError> for PimdirRunError<R, C> {
    fn from(err: PimdirError) -> Self {
        Self::Store(err)
    }
}

/// The remote an offline verb runs against: never asked anything.
struct PimdirOffline;

impl PimdirRemote for PimdirOffline {
    type Error = Infallible;

    fn enumerate(
        &mut self,
        _: &PimdirCollectionId,
        _: Option<PimdirCheckpoint>,
    ) -> Result<PimdirRemoteSnapshot, Infallible> {
        unreachable!("an offline verb never enumerates")
    }

    fn fetch(
        &mut self,
        _: &PimdirCollectionId,
        _: Vec<PimdirHandle>,
        _: PimdirTier,
    ) -> Result<Vec<PimdirFetchedItem>, Infallible> {
        unreachable!("an offline verb never fetches")
    }

    fn push(
        &mut self,
        _: &PimdirCollectionId,
        _: Vec<crate::change::PimdirChange>,
    ) -> Result<Vec<PimdirPushResult>, Infallible> {
        unreachable!("an offline verb never pushes")
    }
}

impl PimdirSourceStore {
    /// Services one storage yield, or hands a remote one back untouched.
    pub fn service(
        &mut self,
        yielded: PimdirYield,
    ) -> Result<Result<PimdirArg, PimdirYield>, PimdirError> {
        Ok(Ok(match yielded {
            PimdirYield::WantsLoad { collection, scope } => {
                PimdirArg::Load(self.load(&collection, &scope)?)
            }
            PimdirYield::WantsLookupObject(links) => {
                PimdirArg::LookupObject(self.lookup_objects(&links)?)
            }
            PimdirYield::WantsWrite(ops) => {
                self.write(ops)?;
                PimdirArg::Write
            }
            remote => return Ok(Err(remote)),
        }))
    }

    /// Runs a coroutine to completion through this store and `remote`.
    pub fn run<C, T, E, R>(
        &mut self,
        mut coroutine: C,
        remote: &mut R,
    ) -> Result<T, PimdirRunError<R::Error, E>>
    where
        C: PimdirCoroutine<Yield = PimdirYield, Return = Result<T, E>>,
        R: PimdirRemote,
    {
        let mut arg: Option<PimdirArg> = None;

        loop {
            let yielded = match coroutine.resume(arg.take()) {
                PimdirCoroutineState::Complete(Ok(out)) => return Ok(out),
                PimdirCoroutineState::Complete(Err(err)) => {
                    return Err(PimdirRunError::Coroutine(err));
                }
                PimdirCoroutineState::Yielded(yielded) => yielded,
            };

            arg = Some(match self.service(yielded)? {
                Ok(arg) => arg,
                Err(PimdirYield::WantsEnumerate { collection, cursor }) => PimdirArg::Enumerate(
                    remote
                        .enumerate(&collection, cursor)
                        .map_err(PimdirRunError::Remote)?,
                ),
                Err(PimdirYield::WantsFetch {
                    collection,
                    handles,
                    tier,
                }) => PimdirArg::Fetch(
                    remote
                        .fetch(&collection, handles, tier)
                        .map_err(PimdirRunError::Remote)?,
                ),
                Err(PimdirYield::WantsPush {
                    collection,
                    changes,
                }) => PimdirArg::Push(
                    remote
                        .push(&collection, changes)
                        .map_err(PimdirRunError::Remote)?,
                ),
                Err(_) => unreachable!("a storage yield is serviced above"),
            });
        }
    }

    /// Opens a collection fully offline: its projection for this source.
    pub fn open_collection(
        &mut self,
        collection: impl Into<PimdirCollectionId>,
    ) -> Result<PimdirLoaded, PimdirRunError<Infallible, PimdirArgError>> {
        self.run(PimdirOpen::new(collection), &mut PimdirOffline)
    }

    /// Raises `handles` in `collection` to `tier`, linking bodies the
    /// store already holds before fetching (SYNC §6).
    pub fn upgrade<R: PimdirRemote>(
        &mut self,
        collection: impl Into<PimdirCollectionId>,
        handles: Vec<PimdirHandle>,
        tier: PimdirTier,
        remote: &mut R,
    ) -> Result<PimdirUpgradeReport, PimdirRunError<R::Error, PimdirArgError>> {
        self.run(PimdirUpgrade::new(collection, handles, tier), remote)
    }

    /// Stages a local edit with no network (SYNC §7).
    pub fn mutate(
        &mut self,
        collection: impl Into<PimdirCollectionId>,
        mutation: PimdirMutation,
    ) -> Result<(), PimdirRunError<Infallible, PimdirMutateError>> {
        self.run(PimdirMutate::new(collection, mutation), &mut PimdirOffline)
    }

    /// Reconciles a collection with its remote (SYNC §5).
    pub fn sync<R: PimdirRemote>(
        &mut self,
        collection: impl Into<PimdirCollectionId>,
        opts: PimdirSyncOptions,
        remote: &mut R,
    ) -> Result<PimdirSyncReport, PimdirRunError<R::Error, PimdirArgError>> {
        self.run(PimdirSync::new(collection, opts), remote)
    }

    /// Rebuilds a collection onto a new handle space, by link id (SYNC §8).
    pub fn rekey<R: PimdirRemote>(
        &mut self,
        collection: impl Into<PimdirCollectionId>,
        remote: &mut R,
    ) -> Result<PimdirRekeyReport, PimdirRunError<R::Error, PimdirArgError>> {
        self.run(PimdirRekey::new(collection), remote)
    }
}
