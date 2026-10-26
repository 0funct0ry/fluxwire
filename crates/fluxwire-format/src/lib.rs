//! `.flux.md` parser, span editor, canonical formatter, and collections on disk (SPEC §6, §7).
//!
//! The parser keeps the source text byte for byte and records spans; every change is a
//! replacement of the spans it owns, so prose, bodies, scripts and unknown content survive.

mod collection;
mod edit;
mod environments;
mod fence;
mod fmt;
mod frontmatter;
mod fsops;
mod head;
mod keymap;
mod lines;
mod parse;
mod schema;
mod scope;
mod settings;
mod template;
mod trust;
mod watch;

pub use collection::*;
pub use edit::*;
pub use environments::*;
pub use fence::parse_info;
pub use fmt::{format, is_canonical};
pub use frontmatter::{decode_scalar, encode_scalar};
pub use fsops::*;
pub use keymap::{Keymap, KeymapEntry, load_keymap, parse_keymap};
pub use lines::{Line, lines};
pub use parse::{parse, parse_with};
pub use schema::front_matter_schema_json;
pub use scope::{ScopeInputs, ancestor_docs, build_scope};
pub use settings::{UserSettings, load_settings, parse_settings};
pub use template::*;
pub use trust::{is_trusted, set_trusted, trusted_collections};
pub use watch::{DEBOUNCE, WatchHandle, watch};
