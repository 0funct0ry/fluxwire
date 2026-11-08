//! The schema model, built from an introspection result, and its on-disk cache.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The standard introspection query (draft spec, no `specifiedByURL` so older servers accept it).
pub const INTROSPECTION_QUERY: &str = "query IntrospectionQuery { __schema { queryType { name } mutationType { name } subscriptionType { name } types { ...FullType } } } fragment FullType on __Type { kind name description fields(includeDeprecated: true) { name description args { ...InputValue } type { ...TypeRef } isDeprecated deprecationReason } inputFields { ...InputValue } interfaces { ...TypeRef } enumValues(includeDeprecated: true) { name description isDeprecated deprecationReason } possibleTypes { ...TypeRef } } fragment InputValue on __InputValue { name description type { ...TypeRef } defaultValue } fragment TypeRef on __Type { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name } } } } } } } }";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeRef {
    Named(String),
    List(Box<TypeRef>),
    NonNull(Box<TypeRef>),
}

impl TypeRef {
    /// The innermost named type.
    pub fn named(&self) -> &str {
        match self {
            TypeRef::Named(n) => n,
            TypeRef::List(t) | TypeRef::NonNull(t) => t.named(),
        }
    }

    pub fn is_non_null(&self) -> bool {
        matches!(self, TypeRef::NonNull(_))
    }

    pub fn is_list(&self) -> bool {
        match self {
            TypeRef::List(_) => true,
            TypeRef::NonNull(t) => t.is_list(),
            TypeRef::Named(_) => false,
        }
    }

    /// Parse a type written as in a variable definition: `[ID!]!`.
    pub fn parse(s: &str) -> Option<TypeRef> {
        let s = s.trim();
        if let Some(inner) = s.strip_suffix('!') {
            return TypeRef::parse(inner).map(|t| TypeRef::NonNull(Box::new(t)));
        }
        if let Some(inner) = s.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            return TypeRef::parse(inner).map(|t| TypeRef::List(Box::new(t)));
        }
        (!s.is_empty() && s.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()))
            .then(|| TypeRef::Named(s.to_string()))
    }
}

impl fmt::Display for TypeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeRef::Named(n) => f.write_str(n),
            TypeRef::List(t) => write!(f, "[{t}]"),
            TypeRef::NonNull(t) => write!(f, "{t}!"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Scalar,
    Object,
    Interface,
    Union,
    Enum,
    InputObject,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InputValue {
    pub name: String,
    pub description: Option<String>,
    pub ty: TypeRef,
    pub default: Option<String>,
}

impl InputValue {
    /// Must be supplied: non-null with no default.
    pub fn required(&self) -> bool {
        self.ty.is_non_null() && self.default.is_none()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldDef {
    pub name: String,
    pub description: Option<String>,
    pub args: Vec<InputValue>,
    pub ty: TypeRef,
    pub deprecated: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeDef {
    pub name: String,
    pub kind: Kind,
    pub description: Option<String>,
    pub fields: Vec<FieldDef>,
    pub input_fields: Vec<InputValue>,
    pub enum_values: Vec<String>,
    pub possible_types: Vec<String>,
}

impl TypeDef {
    pub fn field(&self, name: &str) -> Option<&FieldDef> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Composite types have a selection set.
    pub fn is_composite(&self) -> bool {
        matches!(self.kind, Kind::Object | Kind::Interface | Kind::Union)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Schema {
    pub query_type: String,
    pub mutation_type: Option<String>,
    pub subscription_type: Option<String>,
    pub types: BTreeMap<String, TypeDef>,
}

fn type_ref(v: &Value) -> Option<TypeRef> {
    match v.get("kind")?.as_str()? {
        "NON_NULL" => Some(TypeRef::NonNull(Box::new(type_ref(v.get("ofType")?)?))),
        "LIST" => Some(TypeRef::List(Box::new(type_ref(v.get("ofType")?)?))),
        _ => Some(TypeRef::Named(v.get("name")?.as_str()?.to_string())),
    }
}

fn opt_str(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

fn input_value(v: &Value) -> Option<InputValue> {
    Some(InputValue {
        name: v.get("name")?.as_str()?.to_string(),
        description: opt_str(v, "description"),
        ty: type_ref(v.get("type")?)?,
        default: opt_str(v, "defaultValue"),
    })
}

fn list<T>(v: &Value, key: &str, f: impl Fn(&Value) -> Option<T>) -> Vec<T> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(&f).collect())
        .unwrap_or_default()
}

impl Schema {
    /// Build from a response body (`{"data": {"__schema": ...}}`) or the `data` / `__schema`
    /// object itself.
    pub fn from_introspection(v: &Value) -> Result<Schema, String> {
        let s = v
            .pointer("/data/__schema")
            .or_else(|| v.pointer("/__schema"))
            .or_else(|| v.get("queryType").map(|_| v))
            .ok_or("the response has no `__schema` (is introspection disabled?)")?;
        let root = |k: &str| {
            s.get(k)
                .and_then(|t| t.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        let query_type = root("queryType").ok_or("the schema has no query type")?;
        let mut types = BTreeMap::new();
        for t in s
            .get("types")
            .and_then(Value::as_array)
            .ok_or("the schema lists no types")?
        {
            let Some(name) = t.get("name").and_then(Value::as_str) else {
                continue;
            };
            let kind = match t.get("kind").and_then(Value::as_str) {
                Some("SCALAR") => Kind::Scalar,
                Some("OBJECT") => Kind::Object,
                Some("INTERFACE") => Kind::Interface,
                Some("UNION") => Kind::Union,
                Some("ENUM") => Kind::Enum,
                Some("INPUT_OBJECT") => Kind::InputObject,
                _ => continue,
            };
            let fields = list(t, "fields", |f| {
                Some(FieldDef {
                    name: f.get("name")?.as_str()?.to_string(),
                    description: opt_str(f, "description"),
                    args: list(f, "args", input_value),
                    ty: type_ref(f.get("type")?)?,
                    deprecated: f
                        .get("isDeprecated")
                        .and_then(Value::as_bool)
                        .filter(|d| *d)
                        .map(|_| opt_str(f, "deprecationReason").unwrap_or_default()),
                })
            });
            types.insert(
                name.to_string(),
                TypeDef {
                    name: name.to_string(),
                    kind,
                    description: opt_str(t, "description"),
                    fields,
                    input_fields: list(t, "inputFields", input_value),
                    enum_values: list(t, "enumValues", |e| opt_str(e, "name")),
                    possible_types: list(t, "possibleTypes", |p| {
                        type_ref(p).map(|t| t.named().to_string())
                    }),
                },
            );
        }
        Ok(Schema {
            query_type,
            mutation_type: root("mutationType"),
            subscription_type: root("subscriptionType"),
            types,
        })
    }

    pub fn ty(&self, name: &str) -> Option<&TypeDef> {
        self.types.get(name)
    }

    /// The root type for an operation keyword.
    pub fn root(&self, kind: crate::parse::OpKind) -> Option<&TypeDef> {
        use crate::parse::OpKind::*;
        let name = match kind {
            Query => Some(&self.query_type),
            Mutation => self.mutation_type.as_ref(),
            Subscription => self.subscription_type.as_ref(),
        }?;
        self.ty(name)
    }

    /// User-facing types (introspection types `__*` hidden), sorted by name.
    pub fn visible_types(&self) -> impl Iterator<Item = &TypeDef> {
        self.types.values().filter(|t| !t.name.starts_with("__"))
    }
}

/// Stable 64-bit FNV-1a, so cache file names do not change between runs or versions.
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}

/// A cached introspection result, one file per URL + environment (SPEC M16.2).
pub struct SchemaCache {
    dir: PathBuf,
}

impl SchemaCache {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn path(&self, url: &str, environment: &str) -> PathBuf {
        self.dir.join(format!(
            "{:016x}.json",
            fnv(&format!("{environment}\u{0}{url}"))
        ))
    }

    pub fn load(&self, url: &str, environment: &str) -> Option<Schema> {
        let text = std::fs::read_to_string(self.path(url, environment)).ok()?;
        Schema::from_introspection(&serde_json::from_str(&text).ok()?).ok()
    }

    /// Store the raw introspection response; the schema is rebuilt from it on load.
    pub fn store(&self, url: &str, environment: &str, response: &Value) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.path(url, environment);
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec(response).unwrap_or_default())?;
        std::fs::rename(&tmp, &path)
    }

    pub fn clear(&self, url: &str, environment: &str) {
        let _ = std::fs::remove_file(self.path(url, environment));
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// A small schema used by tests across the crate and by the UI's model tests.
#[doc(hidden)]
pub fn sample_schema() -> Schema {
    let intro = serde_json::json!({"data": {"__schema": {
    "queryType": {"name": "Query"}, "mutationType": {"name": "Mutation"}, "subscriptionType": null,
    "types": [
        {"kind": "OBJECT", "name": "Query", "fields": [
            {"name": "payment", "description": "One payment", "args": [
                {"name": "id", "type": {"kind": "NON_NULL", "ofType": {"kind": "SCALAR", "name": "ID"}}}],
             "type": {"kind": "OBJECT", "name": "Payment"}, "isDeprecated": false},
            {"name": "payments", "args": [
                {"name": "first", "type": {"kind": "SCALAR", "name": "Int"}, "defaultValue": "10"},
                {"name": "status", "type": {"kind": "ENUM", "name": "Status"}}],
             "type": {"kind": "NON_NULL", "ofType": {"kind": "LIST", "ofType": {"kind": "OBJECT", "name": "Payment"}}},
             "isDeprecated": false}]},
        {"kind": "OBJECT", "name": "Mutation", "fields": [
            {"name": "refund", "args": [
                {"name": "input", "type": {"kind": "NON_NULL", "ofType": {"kind": "INPUT_OBJECT", "name": "RefundInput"}}}],
             "type": {"kind": "OBJECT", "name": "Payment"}, "isDeprecated": false}]},
        {"kind": "OBJECT", "name": "Payment", "fields": [
            {"name": "id", "args": [], "type": {"kind": "NON_NULL", "ofType": {"kind": "SCALAR", "name": "ID"}}, "isDeprecated": false},
            {"name": "status", "args": [], "type": {"kind": "ENUM", "name": "Status"}, "isDeprecated": false},
            {"name": "amount", "args": [], "type": {"kind": "SCALAR", "name": "Float"}, "isDeprecated": true, "deprecationReason": "use total"},
            {"name": "customer", "args": [], "type": {"kind": "OBJECT", "name": "Customer"}, "isDeprecated": false}]},
        {"kind": "OBJECT", "name": "Customer", "fields": [
            {"name": "name", "args": [], "type": {"kind": "SCALAR", "name": "String"}, "isDeprecated": false},
            {"name": "email", "args": [], "type": {"kind": "SCALAR", "name": "String"}, "isDeprecated": false}]},
        {"kind": "ENUM", "name": "Status", "enumValues": [{"name": "PAID"}, {"name": "PENDING"}]},
        {"kind": "INPUT_OBJECT", "name": "RefundInput", "inputFields": [
            {"name": "paymentId", "type": {"kind": "NON_NULL", "ofType": {"kind": "SCALAR", "name": "ID"}}},
            {"name": "reason", "type": {"kind": "SCALAR", "name": "String"}}]},
        {"kind": "SCALAR", "name": "ID"}, {"kind": "SCALAR", "name": "Int"},
        {"kind": "SCALAR", "name": "Float"}, {"kind": "SCALAR", "name": "String"},
        {"kind": "SCALAR", "name": "Boolean"}
    ]}}});
    Schema::from_introspection(&intro).expect("sample schema")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_types_fields_and_type_refs() {
        let s = sample_schema();
        assert_eq!(s.query_type, "Query");
        let q = s.ty("Query").unwrap();
        let payments = q.field("payments").unwrap();
        assert_eq!(payments.ty.to_string(), "[Payment]!");
        assert_eq!(payments.ty.named(), "Payment");
        assert!(payments.ty.is_list());
        assert!(!payments.args[0].required());
        assert!(q.field("payment").unwrap().args[0].required());
        assert_eq!(
            s.ty("Payment")
                .unwrap()
                .field("amount")
                .unwrap()
                .deprecated
                .as_deref(),
            Some("use total")
        );
        assert_eq!(s.ty("Status").unwrap().enum_values, ["PAID", "PENDING"]);
    }

    #[test]
    fn type_ref_parse_round_trips() {
        for t in ["ID", "ID!", "[ID!]!", "[[Int]]"] {
            assert_eq!(TypeRef::parse(t).unwrap().to_string(), t);
        }
        assert!(TypeRef::parse("[ID").is_none());
    }

    #[test]
    fn missing_schema_is_an_error() {
        let e = Schema::from_introspection(&serde_json::json!({"errors": []})).unwrap_err();
        assert!(e.contains("introspection"));
    }

    #[test]
    fn cache_is_keyed_by_url_and_environment() {
        let dir = std::env::temp_dir().join(format!("fw-gql-{}", std::process::id()));
        let c = SchemaCache::new(&dir);
        assert_ne!(
            c.path("https://a/graphql", "dev"),
            c.path("https://a/graphql", "prod")
        );
        assert_ne!(
            c.path("https://a/graphql", "dev"),
            c.path("https://b/graphql", "dev")
        );
        let raw = serde_json::json!({"data": {"__schema": {
            "queryType": {"name": "Q"}, "types": [{"kind": "OBJECT", "name": "Q", "fields": []}]}}});
        assert!(c.load("https://a/graphql", "dev").is_none());
        c.store("https://a/graphql", "dev", &raw).unwrap();
        assert_eq!(c.load("https://a/graphql", "dev").unwrap().query_type, "Q");
        assert!(c.load("https://a/graphql", "prod").is_none());
        c.clear("https://a/graphql", "dev");
        assert!(c.load("https://a/graphql", "dev").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
