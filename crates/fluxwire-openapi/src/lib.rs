//! OpenAPI two-way sync core (SPEC §15): the SyncModel and the three-way merge.
//! GPUI-free; used by the CLI and the app.

pub mod folder;
pub mod link;
pub mod merge;
pub mod pull;
pub mod push;
pub mod spec;
pub mod sync_model;
pub mod yaml;

pub use merge::{Change, ChangeKind, Conflict, Direction, MergeResult, merge};
pub use sync_model::{FIELDS, Op, SyncModel};
