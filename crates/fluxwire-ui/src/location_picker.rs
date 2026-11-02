//! The `Save in` choices of the New request dialog: Collections as roots, Folders indented.
//! Pure listing and filtering; the dialog renders the list.

use fluxwire_model::TreeNode;
use std::path::PathBuf;

use crate::workspace_model::Workspace;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub dir: PathBuf,
    pub label: String,
    pub depth: usize,
}

/// Every Collection and Folder in the Workspace, in tree order.
pub fn locations(ws: &Workspace) -> Vec<Location> {
    let mut out = Vec::new();
    for c in &ws.collections {
        let name = if c.collection.config.collection.name.is_empty() {
            ws.name.clone()
        } else {
            c.collection.config.collection.name.clone()
        };
        out.push(Location {
            dir: c.root.clone(),
            label: name,
            depth: 0,
        });
        walk(ws, &c.tree, 1, &mut out);
    }
    out
}

fn walk(ws: &Workspace, nodes: &[TreeNode], depth: usize, out: &mut Vec<Location>) {
    for n in nodes {
        if let TreeNode::Folder {
            name,
            path,
            children,
        } = n
        {
            out.push(Location {
                dir: ws.abs_of(path),
                label: name.clone(),
                depth,
            });
            walk(ws, children, depth + 1, out);
        }
    }
}

/// Locations whose label contains `query` (case-insensitive); everything for an empty query.
pub fn filter<'a>(all: &'a [Location], query: &str) -> Vec<&'a Location> {
    let q = query.trim().to_lowercase();
    all.iter()
        .filter(|l| q.is_empty() || l.label.to_lowercase().contains(&q))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(label: &str, depth: usize) -> Location {
        Location {
            dir: PathBuf::from(label),
            label: label.into(),
            depth,
        }
    }

    #[test]
    fn filter_is_case_insensitive_and_empty_keeps_all() {
        let all = vec![loc("Payments", 0), loc("auth", 1), loc("Orders", 1)];
        assert_eq!(filter(&all, "").len(), 3);
        assert_eq!(filter(&all, " AUTH ")[0].label, "auth");
        assert!(filter(&all, "zzz").is_empty());
    }

    #[test]
    fn lists_collections_then_nested_folders() {
        let ws = crate::workspace_model::Workspace::open(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/payments-api"),
        )
        .unwrap();
        let l = locations(&ws);
        assert_eq!(l[0].depth, 0);
        assert!(l.iter().skip(1).all(|x| x.depth >= 1));
        assert!(l.len() > 1);
    }
}
