//! # Search
//!
//! The I/O-free half of pimdir SEARCH.md: what a search answers about
//! what it could not see ([`coverage`]), and, behind the `search` feature,
//! everything the index derives without a database. The query language
//! parsed to an AST (`query`), the text folds (`fold`), a body's text by
//! field (`extract`) cut into chunks (`chunk`), calendar occurrences
//! within a horizon (`occurrence`), the plan of a tag (`tag`), and the
//! refresh folding the store's feed into the index as an I/O-free
//! coroutine yielding typed reads and writes (`refresh`), which threads
//! the mail it touches. The index over SQLite, index.db and its queries,
//! is the client's (`client::search`, features `client` and `search`).
//!
//! [`coverage`] is outside the feature: the store's `LIKE` reads answer
//! the same coverage before any index exists.

#[cfg(feature = "search")]
pub mod chunk;
pub mod coverage;
#[cfg(feature = "search")]
pub mod extract;
#[cfg(feature = "search")]
pub mod fold;
#[cfg(feature = "search")]
pub mod occurrence;
#[cfg(feature = "search")]
pub mod query;
#[cfg(feature = "search")]
pub mod refresh;
#[cfg(feature = "search")]
pub mod tag;
#[cfg(feature = "search")]
mod thread;
