//! Keeps the `requires` front matter in step with the blocks a file uses (SPEC §7.9).
//!
//! Only features the registry supports and derives from blocks are managed (today `graphql`).
//! Any other entry the user or another tool wrote is left exactly as it is.

use fluxwire_model::{BlockKind, Edit, FileKind, FluxDoc, FmValue, apply_edits};
use fluxwire_protocol::ProtocolRegistry;

use crate::edit::{remove_front_matter, set_front_matter};
use crate::parse::parse_with;

/// Features implied by the blocks of `doc`, among those this registry can derive.
fn derived(doc: &FluxDoc, reg: &ProtocolRegistry) -> Vec<&'static str> {
    let mut out = Vec::new();
    if reg.supports_feature("graphql")
        && doc
            .blocks
            .iter()
            .any(|b| b.top_level && b.kind == BlockKind::GraphqlQuery)
    {
        out.push("graphql");
    }
    out
}

/// Managed feature names: the ones `derived` can ever produce.
const MANAGED: &[&str] = &["graphql"];

/// Edits that add missing and drop stale managed entries; empty when already in sync.
pub fn requires_edits(doc: &FluxDoc, reg: &ProtocolRegistry) -> Vec<Edit> {
    // A build that cannot run `graphql` leaves the entry alone: it is not ours to drop.
    if doc.kind != FileKind::Request || !reg.supports_feature("graphql") {
        return Vec::new();
    }
    let want = derived(doc, reg);
    let have: Vec<String> = doc.front.requires();
    let mut next: Vec<String> = have
        .iter()
        .filter(|f| !MANAGED.contains(&f.as_str()) || want.contains(&f.as_str()))
        .cloned()
        .collect();
    for w in &want {
        if !next.iter().any(|f| f == w) {
            next.push((*w).to_string());
        }
    }
    if next == have {
        return Vec::new();
    }
    if next.is_empty() {
        remove_front_matter(doc, "requires")
    } else {
        set_front_matter(doc, "requires", &FmValue::List(next))
    }
}

/// `src` with `requires` synchronised; unchanged text when nothing needs to move.
pub fn sync_requires(src: &str, kind: FileKind) -> String {
    let reg = ProtocolRegistry::builtin();
    let doc = parse_with(src, kind, &reg);
    let edits = requires_edits(&doc, &reg);
    if edits.is_empty() {
        return src.to_string();
    }
    apply_edits(src, &edits).unwrap_or_else(|| src.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const Q: &str = "```http\nPOST https://x.test/graphql\n```\n\n```graphql query\n{ a }\n```\n";

    #[test]
    fn adds_requires_when_a_query_block_exists() {
        let out = sync_requires(&format!("---\nname: A\n---\n\n{Q}"), FileKind::Request);
        assert!(out.contains("requires: [graphql]"), "{out}");
        assert_eq!(sync_requires(&out, FileKind::Request), out);
    }

    #[test]
    fn creates_front_matter_when_missing() {
        let out = sync_requires(Q, FileKind::Request);
        assert!(out.starts_with("---\nrequires: [graphql]\n---\n"), "{out}");
    }

    #[test]
    fn removes_stale_entry_but_keeps_others() {
        let src = "---\nrequires: [graphql, grpc]\n---\n\n```http\nGET https://x.test\n```\n";
        let out = sync_requires(src, FileKind::Request);
        assert!(out.contains("requires: [grpc]"), "{out}");
        let src = "---\nname: A\nrequires: [graphql]\n---\n\n```http\nGET https://x.test\n```\n";
        let out = sync_requires(src, FileKind::Request);
        assert!(!out.contains("requires"), "{out}");
        assert!(out.contains("name: A"));
    }

    #[test]
    fn untouched_when_in_sync() {
        let src = format!("---\nrequires: [graphql]\n---\n\n{Q}");
        assert_eq!(sync_requires(&src, FileKind::Request), src);
    }
}
