//! The SyncModel (SPEC §15.1): a normalized, ordered map of operations, each a map of the
//! synced fields. Both the spec side and the collection side are mapped into it.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// The synced fields, in display order.
pub const FIELDS: [&str; 9] = [
    "method",
    "path",
    "name",
    "description",
    "tags",
    "parameters",
    "body",
    "expected_status",
    "deprecated",
];

/// One operation: field name → normalized value. A missing field equals `null`.
pub type Op = BTreeMap<String, Value>;

/// Version written to `.fluxwire/sync/<name>.base.json`. Version 1 is M11's snapshot.
pub const BASE_VERSION: u32 = 2;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SyncModel {
    /// Keyed by `operationId`, or `METHOD /path` when the spec has none.
    pub operations: BTreeMap<String, Op>,
}

#[derive(Serialize, Deserialize)]
struct BaseFile {
    version: u32,
    operations: BTreeMap<String, Op>,
}

impl SyncModel {
    /// Read a base snapshot (v1 from M11 or v2), dropping unknown fields.
    pub fn from_base_json(text: &str) -> Result<Self, String> {
        let f: BaseFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if f.version > BASE_VERSION {
            return Err(format!(
                "base snapshot version {} is newer than supported",
                f.version
            ));
        }
        let operations = f
            .operations
            .into_iter()
            .map(|(k, op)| {
                let op = op
                    .into_iter()
                    .filter(|(f, _)| FIELDS.contains(&f.as_str()))
                    .collect();
                (k, op)
            })
            .collect();
        Ok(Self { operations })
    }

    pub fn to_base_json(&self) -> String {
        let f = BaseFile {
            version: BASE_VERSION,
            operations: self.operations.clone(),
        };
        let mut s = serde_json::to_string_pretty(&f).expect("serializable");
        s.push('\n');
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_snapshot_upgrades_and_round_trips() {
        let v1 = r#"{"version":1,"operations":{"getA":{"method":"GET","path":"/a","junk":1}}}"#;
        let m = SyncModel::from_base_json(v1).unwrap();
        assert!(!m.operations["getA"].contains_key("junk"));
        let again = SyncModel::from_base_json(&m.to_base_json()).unwrap();
        assert_eq!(m, again);
        assert!(m.to_base_json().contains("\"version\": 2"));
    }

    #[test]
    fn newer_version_is_refused() {
        assert!(SyncModel::from_base_json(r#"{"version":9,"operations":{}}"#).is_err());
    }
}
