#![no_std]

//! A [pimdir](https://github.com/pimalaya/pimdir) store: an
//! [`io_replica`] [`ReplicaStorage`] backed by a SQLite index plus a
//! content-addressed blob directory.
//!
//! The crate is split so the logic stays I/O-free and the I/O sits behind a
//! feature gate, matching the pimdir spec's "operations are canonical" model:
//!
//! - [`sql`] — the canonical pimdir schema and statements, verbatim from the
//!   spec's `migrations/` and `queries.sql`.
//! - [`codec`] — the pure, I/O-free encodings between the [`io_replica`] model
//!   and the store's columns (flag JSON, the `level`/`status` integer maps).
//! - [`client`] (feature `client`) — [`PimdirStore`], which runs the statements
//!   against SQLite (`rusqlite`) and the blob files, servicing the storage seam.
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
pub use client::{PimdirBlobs, PimdirError, PimdirStore};
