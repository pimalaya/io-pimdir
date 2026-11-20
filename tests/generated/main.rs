//! # Generated suite
//!
//! io-pimdir against the stores of the standard's own generator
//! (spec/generator, the freeze plan's section 3): each store is written by
//! the canonical statements alone from a seed's model, then read through
//! io-pimdir and compared with what the generator's oracle states.

#![cfg(feature = "client")]

mod common;

mod drain;
#[cfg(feature = "search")]
mod index;
mod overlay;
mod reader;
