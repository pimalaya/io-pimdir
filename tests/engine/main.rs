//! # Engine suite
//!
//! The sync engine run end to end against the real store through the
//! fake remote in `common`: one binary, one module per scenario.

mod common;

mod chunked_pushes;
mod client;
mod conflict_property;
mod conflict_rekey;
mod conflict_resolution;
mod conflict_retention;
mod duplicate_link_id;
mod hostile_property;
mod hub;
mod hub_property;
mod identity_property;
mod integration;
mod membership;
mod policy_property;
mod property;
mod soft_delete;
