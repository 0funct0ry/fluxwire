//! Three-way merge (SPEC §15.2). For each operation and field: one side changed → take it;
//! both changed to the same value → take it; both changed differently → conflict.

use crate::sync_model::{FIELDS, Op, SyncModel};
use serde_json::Value;

/// Where a change must be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Spec changed: apply to the collection (pull).
    Incoming,
    /// Collection changed: apply to the spec (push).
    Outgoing,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChangeKind {
    Field { field: String, value: Value },
    Add(Op),
    Delete,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub operation: String,
    pub direction: Direction,
    pub kind: ChangeKind,
}

/// Both sides changed the same thing differently. `field` is `None` for add/delete conflicts.
#[derive(Debug, Clone, PartialEq)]
pub struct Conflict {
    pub operation: String,
    pub field: Option<String>,
    pub base: Value,
    pub spec: Value,
    pub collection: Value,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MergeResult {
    pub changes: Vec<Change>,
    pub conflicts: Vec<Conflict>,
}

impl MergeResult {
    pub fn incoming(&self) -> impl Iterator<Item = &Change> {
        self.changes
            .iter()
            .filter(|c| c.direction == Direction::Incoming)
    }
    pub fn outgoing(&self) -> impl Iterator<Item = &Change> {
        self.changes
            .iter()
            .filter(|c| c.direction == Direction::Outgoing)
    }
}

fn get(op: Option<&Op>, f: &str) -> Value {
    op.and_then(|o| o.get(f)).cloned().unwrap_or(Value::Null)
}

fn op_as_value(op: &Op) -> Value {
    Value::Object(op.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
}

/// Merge base, spec and collection mappings.
pub fn merge(base: &SyncModel, spec: &SyncModel, coll: &SyncModel) -> MergeResult {
    let mut keys: Vec<&String> = base
        .operations
        .keys()
        .chain(spec.operations.keys())
        .chain(coll.operations.keys())
        .collect();
    keys.sort();
    keys.dedup();
    let mut out = MergeResult::default();
    for k in keys {
        let (b, s, c) = (
            base.operations.get(k),
            spec.operations.get(k),
            coll.operations.get(k),
        );
        let name = k.clone();
        let push = |out: &mut MergeResult, direction, kind| {
            out.changes.push(Change {
                operation: name.clone(),
                direction,
                kind,
            })
        };
        match (b, s, c) {
            (_, Some(_), Some(_)) => merge_fields(&mut out, k, b, s, c),
            (None, Some(s), None) => {
                push(&mut out, Direction::Incoming, ChangeKind::Add(s.clone()))
            }
            (None, None, Some(c)) => {
                push(&mut out, Direction::Outgoing, ChangeKind::Add(c.clone()))
            }
            (Some(b), None, Some(c)) => {
                // Deleted in the spec: propose deletion unless the collection also edited it.
                if b == c {
                    push(&mut out, Direction::Incoming, ChangeKind::Delete);
                } else {
                    out.conflicts
                        .push(del_conflict(k, b, Value::Null, op_as_value(c)));
                }
            }
            (Some(b), Some(s), None) => {
                if b == s {
                    push(&mut out, Direction::Outgoing, ChangeKind::Delete);
                } else {
                    out.conflicts
                        .push(del_conflict(k, b, op_as_value(s), Value::Null));
                }
            }
            (_, None, None) => {}
        }
    }
    out
}

fn del_conflict(k: &str, b: &Op, spec: Value, collection: Value) -> Conflict {
    Conflict {
        operation: k.to_string(),
        field: None,
        base: op_as_value(b),
        spec,
        collection,
    }
}

fn merge_fields(out: &mut MergeResult, k: &str, b: Option<&Op>, s: Option<&Op>, c: Option<&Op>) {
    for f in FIELDS {
        let (bv, sv, cv) = (get(b, f), get(s, f), get(c, f));
        if sv == cv {
            continue;
        }
        let (direction, value) = if sv == bv {
            (Direction::Outgoing, cv)
        } else if cv == bv {
            (Direction::Incoming, sv)
        } else {
            out.conflicts.push(Conflict {
                operation: k.to_string(),
                field: Some(f.to_string()),
                base: bv,
                spec: sv,
                collection: cv,
            });
            continue;
        };
        out.changes.push(Change {
            operation: k.to_string(),
            direction,
            kind: ChangeKind::Field {
                field: f.to_string(),
                value,
            },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn model(ops: &[(&str, Value)]) -> SyncModel {
        let mut m = SyncModel::default();
        for (k, v) in ops {
            let op: Op = v
                .as_object()
                .unwrap()
                .iter()
                .map(|(a, b)| (a.clone(), b.clone()))
                .collect();
            m.operations.insert(k.to_string(), op);
        }
        m
    }

    fn one(name: &str) -> SyncModel {
        model(&[("a", json!({"method": "GET", "name": name}))])
    }

    #[test]
    fn table_of_field_rules() {
        // (base, spec, collection) -> (incoming, outgoing, conflicts)
        let cases = [
            ("x", "x", "x", 0, 0, 0), // nothing changed
            ("x", "y", "x", 1, 0, 0), // only spec changed
            ("x", "x", "y", 0, 1, 0), // only collection changed
            ("x", "y", "y", 0, 0, 0), // both changed to the same value
            ("x", "y", "z", 0, 0, 1), // both changed differently
        ];
        for (b, s, c, i, o, k) in cases {
            let r = merge(&one(b), &one(s), &one(c));
            assert_eq!(
                (
                    r.incoming().count(),
                    r.outgoing().count(),
                    r.conflicts.len()
                ),
                (i, o, k),
                "{b}/{s}/{c}"
            );
        }
    }

    #[test]
    fn incoming_carries_the_spec_value_outgoing_the_collection_value() {
        let r = merge(&one("x"), &one("y"), &one("x"));
        assert_eq!(
            r.changes[0].kind,
            ChangeKind::Field {
                field: "name".into(),
                value: json!("y")
            }
        );
        let r = merge(&one("x"), &one("x"), &one("z"));
        assert_eq!(r.changes[0].direction, Direction::Outgoing);
        assert_eq!(
            r.changes[0].kind,
            ChangeKind::Field {
                field: "name".into(),
                value: json!("z")
            }
        );
    }

    #[test]
    fn additions_flow_to_the_other_side() {
        let empty = SyncModel::default();
        let r = merge(&empty, &one("x"), &empty);
        assert_eq!((r.incoming().count(), r.outgoing().count()), (1, 0));
        assert!(matches!(r.changes[0].kind, ChangeKind::Add(_)));
        let r = merge(&empty, &empty, &one("x"));
        assert_eq!((r.incoming().count(), r.outgoing().count()), (0, 1));
    }

    #[test]
    fn both_added_same_is_quiet_and_different_conflicts() {
        let empty = SyncModel::default();
        assert_eq!(merge(&empty, &one("x"), &one("x")), MergeResult::default());
        assert_eq!(merge(&empty, &one("x"), &one("y")).conflicts.len(), 1);
    }

    #[test]
    fn deletions_propose_and_modified_deletions_conflict() {
        let empty = SyncModel::default();
        // spec deleted, collection untouched → incoming delete
        let r = merge(&one("x"), &empty, &one("x"));
        assert_eq!(r.changes[0].kind, ChangeKind::Delete);
        assert_eq!(r.changes[0].direction, Direction::Incoming);
        // collection deleted, spec untouched → outgoing delete
        let r = merge(&one("x"), &one("x"), &empty);
        assert_eq!(r.changes[0].direction, Direction::Outgoing);
        // spec deleted but collection edited → conflict
        let r = merge(&one("x"), &empty, &one("y"));
        assert_eq!((r.changes.len(), r.conflicts.len()), (0, 1));
        assert_eq!(r.conflicts[0].field, None);
        // deleted on both sides → nothing
        assert_eq!(merge(&one("x"), &empty, &empty), MergeResult::default());
    }

    #[test]
    fn missing_field_equals_null() {
        let a = model(&[("a", json!({"method": "GET"}))]);
        let b = model(&[("a", json!({"method": "GET", "description": null}))]);
        assert_eq!(merge(&a, &b, &a), MergeResult::default());
    }
}
