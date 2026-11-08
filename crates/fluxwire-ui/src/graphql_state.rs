//! GraphQL view state and the pure parts of its editor integration (M16): the schema held per
//! endpoint, the completion provider over `fluxwire_graphql::complete`, and the text of the
//! problems list. No GPUI here; `graphql_tab.rs` draws it.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use fluxwire_graphql::complete::{self, Suggestion, SuggestionKind};
use fluxwire_graphql::parse::line_col;
use fluxwire_graphql::schema::Schema;
use fluxwire_graphql::validate::{self, Level, Problem};

use crate::completion::{CompletionItem, CompletionProvider, Kind, Trigger};

/// Key of a schema: the endpoint and the active environment (SPEC M16.2).
pub fn schema_key(url: &str, environment: &str) -> String {
    format!("{environment}\u{0}{url}")
}

#[derive(Default)]
pub struct GraphqlState {
    // Interior mutability: the view reads the on-disk cache once per endpoint while rendering.
    schemas: RefCell<HashMap<String, Arc<Schema>>>,
    probed: RefCell<HashSet<String>>,
    loading: HashSet<String>,
    /// Last introspection message for the toolbar: success counts or the failure.
    pub notice: Option<String>,
    /// Show the query builder beside the editors.
    pub builder_open: bool,
    /// Builder fields the user opened, by dotted path.
    pub expanded: HashSet<String>,
    /// Builder field filter text and its input.
    pub filter: String,
    pub filter_input: crate::text_input::LazyInput,
    /// Argument value inputs by `doc|path.arg`, with the model value they last showed.
    pub arg_inputs: RefCell<HashMap<String, (gpui::Entity<crate::text_input::TextInput>, String)>>,
    /// Variables pane: view mode, height in pixels (0 = default) and the drag in progress as
    /// `(pointer y, height)` at its start.
    pub vars_mode: crate::graphql_vars::VarsMode,
    pub vars_h: f32,
    pub vars_drag: Option<(f32, f32)>,
    /// Documents whose empty query already got a stub, so clearing the editor stays possible.
    pub stubbed: HashSet<String>,
}

impl GraphqlState {
    /// Height of the variables pane.
    pub fn vars_height(&self) -> f32 {
        if self.vars_h <= 0.0 {
            150.0
        } else {
            self.vars_h
        }
    }

    pub fn schema(&self, key: &str) -> Option<Arc<Schema>> {
        self.schemas.borrow().get(key).cloned()
    }

    /// `true` the first time it is asked for `key`: the caller then tries the disk cache.
    pub fn probe(&self, key: &str) -> bool {
        self.probed.borrow_mut().insert(key.to_string())
    }

    /// Store a schema read from the disk cache, without touching the notice.
    pub fn put_cached(&self, key: &str, schema: Schema) {
        self.schemas
            .borrow_mut()
            .insert(key.to_string(), Arc::new(schema));
    }

    pub fn set_schema(&mut self, key: String, schema: Schema) {
        self.loading.remove(&key);
        self.notice = Some(format!(
            "Schema loaded: {} types.",
            schema.visible_types().count()
        ));
        self.schemas.borrow_mut().insert(key, Arc::new(schema));
    }

    pub fn fail(&mut self, key: &str, message: String) {
        self.loading.remove(key);
        self.notice = Some(format!("Introspection failed: {message}"));
    }

    /// Marks a fetch as running; `false` when one already is.
    pub fn start(&mut self, key: &str) -> bool {
        self.notice = Some("Fetching the schema…".into());
        self.loading.insert(key.to_string())
    }

    pub fn is_loading(&self, key: &str) -> bool {
        self.loading.contains(key)
    }
}

fn kind_of(k: SuggestionKind) -> Kind {
    match k {
        SuggestionKind::Field => Kind::Property,
        SuggestionKind::Argument => Kind::Variable,
        SuggestionKind::Value => Kind::Value,
        SuggestionKind::Variable => Kind::Variable,
        SuggestionKind::Type => Kind::Namespace,
        SuggestionKind::Keyword => Kind::Keyword,
    }
}

fn word_start(text: &str, offset: usize) -> usize {
    text[..offset]
        .rfind(|c: char| !(c == '_' || c.is_ascii_alphanumeric()))
        .map_or(0, |i| i + 1)
}

fn item(s: Suggestion, replace: std::ops::Range<usize>) -> CompletionItem {
    // An argument inserts `name: ` so the next keystroke is its value.
    let insert_text = match s.kind {
        SuggestionKind::Argument => format!("{}: ", s.insert),
        _ => s.insert.clone(),
    };
    CompletionItem {
        label: s.label,
        kind: kind_of(s.kind),
        detail: if s.deprecated {
            format!("{} (deprecated)", s.detail)
        } else {
            s.detail
        },
        documentation: String::new(),
        insert_text,
        replace,
        group: "",
    }
}

/// Schema completion for the `graphql query` block.
pub struct GraphqlProvider {
    pub schema: Arc<Schema>,
}

impl CompletionProvider for GraphqlProvider {
    fn completions(&self, text: &str, offset: usize, _trigger: Trigger) -> Vec<CompletionItem> {
        let offset = offset.min(text.len());
        let start = word_start(text, offset);
        complete::complete(text, offset, &self.schema)
            .into_iter()
            .map(|s| item(s, start..offset))
            .collect()
    }

    fn is_trigger_char(&self, c: char) -> bool {
        matches!(c, '{' | '(' | '$' | ':' | ' ')
    }
}

pub fn provider(schema: Arc<Schema>) -> Rc<dyn CompletionProvider> {
    Rc::new(GraphqlProvider { schema })
}

/// One line of the problems list under the query editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProblemLine {
    pub text: String,
    pub error: bool,
    /// Byte offset to put the caret at when the line is clicked.
    pub offset: usize,
}

pub fn problem_lines(query: &str, schema: &Schema) -> Vec<ProblemLine> {
    validate::validate(query, schema)
        .into_iter()
        .map(|p: Problem| {
            let (l, c) = line_col(query, p.span.start);
            ProblemLine {
                text: format!("{l}:{c}  {}  {}", p.code, p.message),
                error: p.level == Level::Error,
                offset: p.span.start,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluxwire_graphql::schema::sample_schema;

    #[test]
    fn provider_replaces_the_typed_word_and_inserts_argument_colons() {
        let p = GraphqlProvider {
            schema: Arc::new(sample_schema()),
        };
        let src = "{ payments(fi) { id } }";
        let off = src.find("fi)").unwrap() + 2;
        let items = p.completions(src, off, Trigger::Typing);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "first");
        assert_eq!(items[0].insert_text, "first: ");
        assert_eq!(&src[items[0].replace.clone()], "fi");
    }

    #[test]
    fn deprecated_fields_are_marked() {
        let p = GraphqlProvider {
            schema: Arc::new(sample_schema()),
        };
        let src = "{ payment(id: 1) { am } }";
        let off = src.find("am }").unwrap() + 2;
        let items = p.completions(src, off, Trigger::Typing);
        assert!(items[0].detail.contains("deprecated"));
    }

    #[test]
    fn problem_lines_carry_position_and_severity() {
        let lines = problem_lines("{\n  payment(id: 1) { nope }\n}", &sample_schema());
        assert_eq!(lines.len(), 1);
        assert!(lines[0].text.starts_with("2:20  G201"), "{}", lines[0].text);
        assert!(lines[0].error);
    }

    #[test]
    fn state_tracks_loading_and_schema_per_endpoint_and_environment() {
        let mut s = GraphqlState::default();
        let k = schema_key("https://a/graphql", "dev");
        assert_ne!(k, schema_key("https://a/graphql", "prod"));
        assert!(s.start(&k));
        assert!(!s.start(&k));
        assert!(s.is_loading(&k));
        s.set_schema(k.clone(), sample_schema());
        assert!(!s.is_loading(&k));
        assert!(s.schema(&k).is_some());
        assert!(s.schema(&schema_key("https://a/graphql", "prod")).is_none());
        s.fail(&k, "boom".into());
        assert!(s.notice.unwrap().contains("boom"));
    }
}
