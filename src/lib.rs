#![no_std]
#![deny(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! A [pimdir](https://github.com/pimalaya/pimdir) store: an
//! [`io_replica`] [`ReplicaStorage`] backed by a SQLite index plus a
//! content-addressed blob directory.
//!
//! The crate is split so the logic stays I/O-free and the I/O sits
//! behind a feature gate. [`sql`] inlines the canonical pimdir schema,
//! migrations and statements from the spec's migrations/ and queries/.
//! [`codec`] holds the I/O-free encodings between the [`io_replica`]
//! model and the store columns: the flag JSON, the detail-level integer
//! map and the action queue's versioned payload JSON. [`conventions`]
//! derives a writer's `link_id`, `meta` and `sort_key` from an item's
//! bytes, and [`hash`] names a body. [`client`] (feature `client`) is
//! [`PimdirStore`], which runs the statements against SQLite and the
//! blob files to service the storage seam, plus [`PimdirProducer`], the
//! enqueue-only handle for processes that do not own the store.
//!
//! The crate is `no_std` with `alloc`; `std` only enters behind the
//! `client` feature, where the SQLite driver lives.
//!
//! [`ReplicaStorage`]: io_replica::client::ReplicaStorage

extern crate alloc;
#[cfg(feature = "client")]
extern crate std;

pub mod codec;
pub mod conventions;
pub mod hash;
pub mod sql;

#[cfg(feature = "client")]
pub mod client;

#[cfg(feature = "client")]
pub use client::{
    PimdirBlobFile, PimdirBlobWriter, PimdirBlobs, PimdirCollection, PimdirDrainReport,
    PimdirError, PimdirGcReport, PimdirItem, PimdirParkedAction, PimdirPendingAction,
    PimdirProducer, PimdirPurgeReport, PimdirRetention, PimdirSourceStore, PimdirStore,
    diagnostics::{PimdirAmbiguous, PimdirDangling, PimdirObjectStats, PimdirRefcountDrift},
};
