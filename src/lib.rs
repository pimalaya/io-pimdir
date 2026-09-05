#![no_std]
#![deny(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! # io-pimdir
//!
//! Rust implementation of the [pimdir](https://github.com/pimalaya/pimdir)
//! standard: the store on disk and the sync engine that keeps it an
//! offline replica of every source. The I/O-free core holds the whole
//! logic; SQLite and the filesystem enter behind the `client` feature.
//!
//! ## The store
//!
//! A store is a SQLite database plus a content-addressed blob directory
//! (STORAGE.md). [`sql`] inlines the canonical schema and statements,
//! [`codec`] the column encodings and the queue payload, [`hash`] names a
//! body, and [`summary`] derives what a writer records about an item
//! before its row reaches the store (Annex A): its key, its summary row
//! with the people it names, and its sort key.
//!
//! ## The model
//!
//! [`collection`], [`object`] and [`placement`] are the shared item model:
//! a body stored once, a placement per source per collection pinned by a
//! handle and keyed across collections by a link id, at one rung of the
//! detail ladder, reconciled against a per-source base. [`hub`] is the
//! shared item with a binding per source, which a store projects for one
//! source and absorbs a source's writes back into (SYNC.md §3, §9).
//!
//! ## The engine
//!
//! The five verbs are I/O-free coroutines under the [`coroutine`]
//! contract: [`open`], [`upgrade`], [`mutate`], [`sync`] and [`rekey`]
//! (SYNC.md §1). Each yields requests to the two seams, [`load`] and
//! [`change`] on the storage side and [`remote`] on the connector side,
//! and a caller resumes it with the answers. Nothing here opens a socket
//! or a file.
//!
//! ## The client
//!
//! [`client`] (feature `client`) is the std store, one handle per profile
//! STORAGE.md names: one owner that runs the statements against SQLite
//! and the blob files and runs the verbs against a consumer's
//! [`remote::PimdirRemote`], any number of producers that enqueue under
//! the staging lock, and readers that take no lock. The `pimdir` binary
//! (feature `cli`) is the operator tool over it.
//!
//! ## Conventions
//!
//! The crate is `no_std` with `alloc`; `std` enters behind `client`.
//! Public items carry the `Pimdir` prefix. Section references (`§n`) are
//! to STORAGE.md unless prefixed, and the crate's own design history is
//! under cairn/.

extern crate alloc;
#[cfg(feature = "client")]
extern crate std;

/// Declares one of the crate's string-newtype identities.
macro_rules! pimdir_id {
    (
        $(#[$meta:meta])*
        $name:ident $(, $derive:ident)* $(,)?
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Eq, PartialEq $(, $derive)*)]
        pub struct $name(pub alloc::string::String);

        impl $name {
            #[doc = concat!("Borrows the ", stringify!($name), " as a string slice.")]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.into())
            }
        }

        impl From<alloc::string::String> for $name {
            fn from(value: alloc::string::String) -> Self {
                Self(value)
            }
        }
    };
}

pub(crate) use pimdir_id;

pub mod change;
pub mod codec;
pub mod collection;
pub mod coroutine;
pub mod hash;
pub mod hub;
pub mod load;
pub mod mutate;
pub mod object;
pub mod open;
pub mod placement;
pub mod rekey;
pub mod remote;
pub mod sql;
pub mod summary;
pub mod sync;
#[cfg(test)]
mod testlog;
pub mod upgrade;

#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub mod client;
