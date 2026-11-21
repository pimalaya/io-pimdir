//! # Search
//!
//! The I/O-free half of pimdir SEARCH.md: [`query`] and [`coverage`] need
//! no feature, the text layer comes with `search` or any domain, `mail`
//! and `card` with their domains, and the refresh with `indexer`.

#[cfg(feature = "contact")]
pub mod card;
#[cfg(any(
    feature = "search",
    feature = "mail",
    feature = "contact",
    feature = "calendar"
))]
pub mod chunk;
pub mod coverage;
#[cfg(any(
    feature = "search",
    feature = "mail",
    feature = "contact",
    feature = "calendar"
))]
pub mod extract;
#[cfg(any(
    feature = "search",
    feature = "mail",
    feature = "contact",
    feature = "calendar"
))]
pub mod fold;
#[cfg(feature = "mail")]
mod html;
#[cfg(feature = "mail")]
pub mod mail;
#[cfg(feature = "indexer")]
pub mod occurrence;
pub mod query;
#[cfg(feature = "indexer")]
pub mod refresh;
#[cfg(feature = "search")]
pub mod tag;
#[cfg(feature = "indexer")]
mod thread;
