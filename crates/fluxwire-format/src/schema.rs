//! JSON Schema for front matter (SPEC §7.4), generated from these types for editors and agents.
//! Regenerate `docs/schema/front-matter.schema.json` with `FLUXWIRE_UPDATE_SCHEMA=1 cargo test -p fluxwire-format schema`.

use schemars::JsonSchema;
use std::collections::BTreeMap;

/// Front matter of a `.flux.md` file. Keys starting with `x-` are extensions and are preserved, never interpreted.
#[derive(JsonSchema)]
#[schemars(title = "Fluxwire front matter", extend("patternProperties" = {"^x-": {}}))]
#[allow(dead_code)]
pub struct FrontMatterSchema {
    /// Features this file needs to run. Maintained by Fluxwire; unsupported ones make the file non-runnable.
    pub requires: Option<Vec<String>>,
    /// Display name (required for requests).
    pub name: Option<String>,
    /// `request` (default for `*.flux.md`), `folder`, or `collection`. Other kinds need `requires`.
    pub kind: Option<String>,
    /// Ordering within the parent.
    pub seq: Option<i64>,
    /// Used by filters such as `--filter tag:smoke`.
    pub tags: Option<Vec<String>>,
    /// OpenAPI operationId or `METHOD /path`.
    pub operation: Option<String>,
    /// Mirrors OpenAPI `deprecated`.
    pub deprecated: Option<bool>,
    /// `inherit` (default), `none`, or a map with `type` and fields.
    pub auth: Option<AuthSchema>,
    /// Values for `:name` path parameters.
    pub path: Option<BTreeMap<String, String>>,
    /// Variables set before the send.
    pub vars: Option<BTreeMap<String, String>>,
    /// Variables set from the response: `$.json.path`, `header <Name>`, `cookie <name>`, `status`.
    pub capture: Option<BTreeMap<String, String>>,
    /// Per-request settings.
    pub settings: Option<SettingsSchema>,
}

#[derive(JsonSchema)]
#[serde(untagged)]
#[allow(dead_code)]
pub enum AuthSchema {
    Keyword(String),
    Map(BTreeMap<String, String>),
}

#[derive(JsonSchema)]
#[allow(dead_code)]
pub struct SettingsSchema {
    pub timeout_ms: Option<u64>,
    pub follow_redirects: Option<bool>,
    pub max_redirects: Option<u32>,
    pub http_version: Option<String>,
    pub verify_tls: Option<bool>,
    /// Folders only: `sequential` or `concurrent`.
    pub run_mode: Option<String>,
}

pub fn front_matter_schema_json() -> String {
    let schema = schemars::schema_for!(FrontMatterSchema);
    // Sort keys so output does not depend on serde_json's `preserve_order` feature, which other
    // workspace crates may enable.
    let mut s = serde_json::to_string_pretty(&sorted(
        serde_json::to_value(&schema).expect("schema serialises"),
    ))
    .expect("schema serialises");
    s.push('\n');
    s
}

fn sorted(v: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::Object(m) => {
            let mut keys: Vec<_> = m.into_iter().collect();
            keys.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(keys.into_iter().map(|(k, v)| (k, sorted(v))).collect())
        }
        Value::Array(a) => Value::Array(a.into_iter().map(sorted).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_file_is_up_to_date() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/schema/front-matter.schema.json");
        let want = front_matter_schema_json();
        if std::env::var_os("FLUXWIRE_UPDATE_SCHEMA").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &want).unwrap();
        }
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(
            have, want,
            "run with FLUXWIRE_UPDATE_SCHEMA=1 to regenerate"
        );
    }
}
