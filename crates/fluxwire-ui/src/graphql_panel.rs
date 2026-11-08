//! Pure model of the GraphQL builder panel (M16): which fields the tree shows for a filter. No GPUI here; `graphql_tab.rs` draws it.

use fluxwire_graphql::schema::{FieldDef, Schema, TypeDef};

/// How deep the builder looks below a collapsed field for a filter match.
const SEARCH_DEPTH: usize = 3;

/// Case-insensitive substring match; an empty filter matches everything.
pub fn name_matches(name: &str, filter: &str) -> bool {
    let f = filter.trim();
    f.is_empty() || name.to_lowercase().contains(&f.to_lowercase())
}

/// Whether any field of `ty`, or of the types below it down to `depth` levels, matches.
pub fn branch_matches(schema: &Schema, ty: &TypeDef, filter: &str, depth: usize) -> bool {
    ty.fields.iter().any(|f| {
        name_matches(&f.name, filter)
            || (depth > 0
                && schema
                    .ty(f.ty.named())
                    .filter(|t| t.is_composite() && t.name != ty.name)
                    .is_some_and(|t| branch_matches(schema, t, filter, depth - 1)))
    })
}

/// A field is listed when it matches the filter itself or has a matching field below it.
pub fn field_visible(schema: &Schema, f: &FieldDef, filter: &str) -> bool {
    filter.trim().is_empty()
        || name_matches(&f.name, filter)
        || schema
            .ty(f.ty.named())
            .filter(|t| t.is_composite())
            .is_some_and(|t| branch_matches(schema, t, filter, SEARCH_DEPTH - 1))
}

/// A field is open when the user expanded it, when it is selected, or when the filter only
/// matches below it.
pub fn field_open(
    schema: &Schema,
    f: &FieldDef,
    filter: &str,
    expanded: bool,
    checked: bool,
) -> bool {
    if expanded || checked {
        return true;
    }
    !filter.trim().is_empty()
        && !name_matches(&f.name, filter)
        && schema
            .ty(f.ty.named())
            .filter(|t| t.is_composite())
            .is_some_and(|t| branch_matches(schema, t, filter, SEARCH_DEPTH - 1))
}

/// Stable id for a field path, used for element ids and the expanded set.
pub fn path_key(path: &[String]) -> String {
    path.join(".")
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluxwire_graphql::schema::sample_schema;

    #[test]
    fn names_match_case_insensitively_and_empty_matches_all() {
        assert!(name_matches("customer", "CUST"));
        assert!(name_matches("customer", "  "));
        assert!(!name_matches("customer", "zip"));
    }

    #[test]
    fn filter_finds_fields_below_a_collapsed_parent() {
        let s = sample_schema();
        let q = s.ty("Query").unwrap();
        let payment = q.field("payment").unwrap();
        let payments = q.field("payments").unwrap();
        // `email` lives at payment.customer.email, three levels down
        assert!(field_visible(&s, payment, "email"));
        assert!(field_visible(&s, payments, "email"));
        assert!(!field_visible(&s, payment, "nothing-like-this"));
        assert!(field_open(&s, payment, "email", false, false));
        assert!(!field_open(&s, payment, "", false, false));
        assert!(field_open(&s, payment, "", true, false));
        assert!(field_open(&s, payment, "", false, true));
        // a field that matches itself is not forced open
        assert!(!field_open(&s, payment, "payment", false, false));
    }

    #[test]
    fn path_keys_join_with_dots() {
        assert_eq!(path_key(&["a".into(), "b".into()]), "a.b");
    }
}
