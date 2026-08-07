#![no_std]
#![deny(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! A [pimdir](https://github.com/pimalaya/pimdir) store: an
//! [`io_replica`] [`ReplicaStorage`] backed by a SQLite index plus a
//! content-addressed blob directory.
//!
//! The crate is split so the logic stays I/O-free and the I/O sits behind a
//! feature gate, matching the pimdir spec's "operations are canonical" model.
//! The [`sql`] module inlines the canonical pimdir schema, migrations and
//! statements from the spec's migrations/ and queries/. The [`codec`] module
//! holds the pure, I/O-free encodings between the [`io_replica`] model and the
//! store columns (the flag JSON, the detail-level integer map and the action
//! queue's versioned payload JSON). The [`client`] module (feature `client`)
//! is [`PimdirStore`], which runs the statements against SQLite (rusqlite) and
//! the blob files, servicing the storage seam, plus [`PimdirProducer`], the
//! enqueue-only handle for processes that do not own the store.
//!
//! The crate is `no_std` with `alloc`; `std` only enters behind the `client`
//! feature, where the SQLite driver lives.
//!
//! [`ReplicaStorage`]: io_replica::client::ReplicaStorage

extern crate alloc;
#[cfg(feature = "client")]
extern crate std;

pub mod codec;
pub mod sql;

#[cfg(feature = "client")]
pub mod client;

#[cfg(feature = "client")]
pub use client::{
    PimdirBlobWriter, PimdirBlobs, PimdirCollection, PimdirDrainReport, PimdirError, PimdirItem,
    PimdirParkedAction, PimdirPendingAction, PimdirProducer, PimdirPurgeReport, PimdirRetainedItem,
    PimdirStore,
};
