//! # Open
//!
//! I/O-free coroutine opening a collection fully offline: one storage
//! read of the placements and checkpoint, handed straight back.

use log::{debug, trace};

use crate::{
    collection::PimdirCollectionId,
    coroutine::*,
    load::{PimdirLoadScope, PimdirLoaded},
};

/// I/O-free OPEN coroutine.
pub struct PimdirOpen {
    collection: PimdirCollectionId,
    state: State,
}

impl PimdirOpen {
    /// Creates a coroutine that loads `collection` from storage.
    pub fn new(collection: impl Into<PimdirCollectionId>) -> Self {
        let collection = collection.into();
        debug!("open collection {}", collection.as_str());

        Self {
            collection,
            state: State::Start,
        }
    }
}

impl PimdirCoroutine for PimdirOpen {
    type Yield = PimdirYield;
    type Return = Result<PimdirLoaded, PimdirArgError>;

    fn resume(
        &mut self,
        arg: Option<PimdirArg>,
    ) -> PimdirCoroutineState<Self::Yield, Self::Return> {
        match (&self.state, arg) {
            (State::Start, None) => {
                debug!("load collection from storage");
                self.state = State::Loading;
                PimdirCoroutineState::Yielded(PimdirYield::WantsLoad {
                    collection: self.collection.clone(),
                    scope: PimdirLoadScope::All,
                })
            }
            (State::Loading, Some(PimdirArg::Load(loaded))) => {
                debug!("opened collection with {} items", loaded.placements.len());
                trace!("loaded placements: {:?}", loaded.placements);
                self.state = State::Done;
                PimdirCoroutineState::Complete(Ok(loaded))
            }
            (State::Done, _) | (_, Some(_)) => {
                PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg))
            }
            (_, None) => PimdirCoroutineState::Complete(Err(PimdirArgError::MissingArg)),
        }
    }
}

/// What the coroutine is doing while it waits for the caller.
enum State {
    Start,
    Loading,
    Done,
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use crate::{
        collection::PimdirCheckpoint,
        open::*,
        placement::{PimdirFlags, PimdirHandle, PimdirLevel, PimdirPlacement, PimdirStatus},
    };

    fn placement(handle: &str) -> PimdirPlacement {
        PimdirPlacement {
            sort_key: Default::default(),
            collection: "inbox".into(),
            handle: PimdirHandle::from(handle),
            link_id: None,
            object: None,
            level: PimdirLevel::Probed,
            summary: None,
            flags: PimdirFlags::default(),
            status: PimdirStatus::Clean,
            conflict_revision: None,
            conflict_object: None,
            base: None,
            origin: None,
        }
    }

    #[test]
    fn start_yields_load() {
        let mut open = PimdirOpen::new("inbox");
        match open.resume(None) {
            PimdirCoroutineState::Yielded(PimdirYield::WantsLoad { collection, .. }) => {
                assert_eq!(collection.as_str(), "inbox");
            }
            state => panic!("expected WantsLoad, got {state:?}"),
        }
    }

    #[test]
    fn load_completes_with_placements() {
        crate::testlog::init();
        let mut open = PimdirOpen::new("inbox");
        let _ = open.resume(None);

        let loaded = PimdirLoaded {
            placements: vec![placement("1"), placement("2")],
            checkpoint: Some(PimdirCheckpoint(b"tok".to_vec())),
        };
        match open.resume(Some(PimdirArg::Load(loaded))) {
            PimdirCoroutineState::Complete(Ok(out)) => assert_eq!(out.placements.len(), 2),
            state => panic!("expected Complete(Ok), got {state:?}"),
        }
    }

    /// A caller resuming a finished coroutine is told, not handed a success.
    #[test]
    fn a_completed_open_does_not_resume() {
        let mut open = PimdirOpen::new("inbox");
        let _ = open.resume(None);
        let _ = open.resume(Some(PimdirArg::Load(PimdirLoaded {
            placements: vec![placement("1")],
            checkpoint: None,
        })));

        match open.resume(Some(PimdirArg::Write)) {
            PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => {}
            state => panic!("expected UnexpectedArg, got {state:?}"),
        }
        match open.resume(None) {
            PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => {}
            state => panic!("expected UnexpectedArg, got {state:?}"),
        }
    }

    #[test]
    fn unexpected_arg_at_start_errors() {
        let mut open = PimdirOpen::new("inbox");
        match open.resume(Some(PimdirArg::Write)) {
            PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => {}
            state => panic!("expected UnexpectedArg, got {state:?}"),
        }
    }

    #[test]
    fn missing_arg_at_pending_load_errors() {
        let mut open = PimdirOpen::new("inbox");
        let _ = open.resume(None);
        match open.resume(None) {
            PimdirCoroutineState::Complete(Err(PimdirArgError::MissingArg)) => {}
            state => panic!("expected MissingArg, got {state:?}"),
        }
    }

    #[test]
    fn wrong_arg_kind_at_pending_load_errors() {
        let mut open = PimdirOpen::new("inbox");
        let _ = open.resume(None);
        match open.resume(Some(PimdirArg::Write)) {
            PimdirCoroutineState::Complete(Err(PimdirArgError::UnexpectedArg)) => {}
            state => panic!("expected UnexpectedArg, got {state:?}"),
        }
    }
}
