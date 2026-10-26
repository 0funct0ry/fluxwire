//! Domain types: spans, diagnostics, parsed `.flux.md` documents, collections.
//!
//! Nothing here depends on GPUI or on any protocol. Parsed values keep the
//! byte spans they came from so the span editor can replace only what it owns.

mod collection;
mod diag;
mod doc;

pub use collection::*;
pub use diag::*;
pub use doc::*;
