//! A small GraphQL executable-document parser with byte spans (October 2021 spec, executable
//! definitions only). It backs validation, completion context and the query builder, and never
//! panics on partial input: a failure is a `ParseError` with the offset it stopped at.

use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Query,
    Mutation,
    Subscription,
}

impl OpKind {
    pub fn keyword(self) -> &'static str {
        match self {
            OpKind::Query => "query",
            OpKind::Mutation => "mutation",
            OpKind::Subscription => "subscription",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub definitions: Vec<Definition>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Definition {
    Operation(OperationDef),
    Fragment(FragmentDef),
}

#[derive(Debug, Clone, PartialEq)]
pub struct OperationDef {
    pub kind: OpKind,
    pub name: Option<String>,
    pub variables: Vec<VarDef>,
    pub selection: SelectionSet,
    pub span: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FragmentDef {
    pub name: String,
    pub type_condition: String,
    pub selection: SelectionSet,
    pub span: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VarDef {
    pub name: String,
    /// The type as written, e.g. `[ID!]!`.
    pub ty: String,
    pub default: Option<String>,
    pub span: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectionSet {
    pub items: Vec<Selection>,
    pub span: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Selection {
    Field(Field),
    Spread {
        name: String,
        span: Range<usize>,
    },
    Inline {
        type_condition: Option<String>,
        selection: SelectionSet,
        span: Range<usize>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub alias: Option<String>,
    pub name: String,
    pub args: Vec<Arg>,
    pub selection: Option<SelectionSet>,
    pub span: Range<usize>,
    pub name_span: Range<usize>,
}

impl Field {
    /// The key this field has in the response.
    pub fn response_key(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    pub name: String,
    pub value: Value,
    /// The value exactly as written.
    pub text: String,
    pub span: Range<usize>,
    pub name_span: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Variable(String),
    Int,
    Float,
    String,
    Bool,
    Null,
    Enum(String),
    List(Vec<Value>),
    Object(Vec<(String, Value)>),
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Punct(char),
    Spread,
    Name(String),
    Int,
    Float,
    Str,
    Eof,
}

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn skip_trivia(&mut self) {
        let b = self.src.as_bytes();
        while self.pos < b.len() {
            match b[self.pos] {
                b' ' | b'\t' | b'\n' | b'\r' | b',' => self.pos += 1,
                0xEF if self.src[self.pos..].starts_with('\u{feff}') => self.pos += 3,
                b'#' => {
                    while self.pos < b.len() && b[self.pos] != b'\n' {
                        self.pos += 1;
                    }
                }
                _ => break,
            }
        }
    }

    fn next(&mut self) -> Result<(Tok, Range<usize>), ParseError> {
        self.skip_trivia();
        let start = self.pos;
        let b = self.src.as_bytes();
        let Some(&c) = b.get(self.pos) else {
            return Ok((Tok::Eof, start..start));
        };
        let err = |m: &str, o: usize| ParseError {
            message: m.to_string(),
            offset: o,
        };
        let tok = match c {
            b'!' | b'$' | b'&' | b'(' | b')' | b':' | b'=' | b'@' | b'[' | b']' | b'{' | b'|'
            | b'}' => {
                self.pos += 1;
                Tok::Punct(c as char)
            }
            b'.' => {
                if self.src[self.pos..].starts_with("...") {
                    self.pos += 3;
                    Tok::Spread
                } else {
                    return Err(err("Unexpected '.'", start));
                }
            }
            b'"' => {
                if self.src[self.pos..].starts_with("\"\"\"") {
                    self.pos += 3;
                    loop {
                        let rest = &self.src[self.pos..];
                        if rest.is_empty() {
                            return Err(err("Unterminated block string", start));
                        }
                        if rest.starts_with("\\\"\"\"") {
                            self.pos += 4;
                        } else if rest.starts_with("\"\"\"") {
                            self.pos += 3;
                            break;
                        } else {
                            self.pos += rest.chars().next().map_or(1, char::len_utf8);
                        }
                    }
                } else {
                    self.pos += 1;
                    loop {
                        match b.get(self.pos) {
                            None | Some(b'\n') => return Err(err("Unterminated string", start)),
                            Some(b'\\') => self.pos += 2,
                            Some(b'"') => {
                                self.pos += 1;
                                break;
                            }
                            Some(_) => {
                                self.pos += self.src[self.pos..]
                                    .chars()
                                    .next()
                                    .map_or(1, char::len_utf8)
                            }
                        }
                    }
                }
                Tok::Str
            }
            b'-' | b'0'..=b'9' => {
                self.pos += 1;
                let mut float = false;
                while let Some(&d) = b.get(self.pos) {
                    match d {
                        b'0'..=b'9' => self.pos += 1,
                        b'.' | b'e' | b'E' => {
                            float = true;
                            self.pos += 1;
                            if matches!(b.get(self.pos), Some(b'+' | b'-')) {
                                self.pos += 1;
                            }
                        }
                        _ => break,
                    }
                }
                if self.pos == start + 1 && c == b'-' {
                    return Err(err("Expected a digit after '-'", start));
                }
                if float { Tok::Float } else { Tok::Int }
            }
            c if c == b'_' || c.is_ascii_alphabetic() => {
                while b
                    .get(self.pos)
                    .is_some_and(|&d| d == b'_' || d.is_ascii_alphanumeric())
                {
                    self.pos += 1;
                }
                Tok::Name(self.src[start..self.pos].to_string())
            }
            _ => {
                let ch = self.src[start..].chars().next().unwrap_or('?');
                return Err(err(&format!("Unexpected character '{ch}'"), start));
            }
        };
        Ok((tok, start..self.pos))
    }
}

struct Parser<'a> {
    src: &'a str,
    lex: Lexer<'a>,
    tok: Tok,
    span: Range<usize>,
    prev_end: usize,
}

type R<T> = Result<T, ParseError>;

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> R<Self> {
        let mut lex = Lexer { src, pos: 0 };
        let (tok, span) = lex.next()?;
        Ok(Self {
            src,
            lex,
            tok,
            span,
            prev_end: 0,
        })
    }

    fn bump(&mut self) -> R<Tok> {
        let (next, span) = self.lex.next()?;
        self.prev_end = self.span.end;
        self.span = span;
        Ok(std::mem::replace(&mut self.tok, next))
    }

    fn err<T>(&self, message: impl Into<String>) -> R<T> {
        Err(ParseError {
            message: message.into(),
            offset: self.span.start,
        })
    }

    fn describe(&self) -> String {
        match &self.tok {
            Tok::Eof => "end of the query".into(),
            Tok::Punct(c) => format!("'{c}'"),
            Tok::Spread => "'...'".into(),
            Tok::Name(n) => format!("'{n}'"),
            _ => format!("'{}'", &self.src[self.span.clone()]),
        }
    }

    fn eat(&mut self, c: char) -> R<bool> {
        if self.tok == Tok::Punct(c) {
            self.bump()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn expect(&mut self, c: char) -> R<()> {
        if self.eat(c)? {
            Ok(())
        } else {
            self.err(format!("Expected '{c}' but found {}", self.describe()))
        }
    }

    fn name(&mut self) -> R<(String, Range<usize>)> {
        if let Tok::Name(n) = &self.tok {
            let out = (n.clone(), self.span.clone());
            self.bump()?;
            Ok(out)
        } else {
            self.err(format!("Expected a name but found {}", self.describe()))
        }
    }

    fn document(&mut self) -> R<Document> {
        let mut definitions = Vec::new();
        while self.tok != Tok::Eof {
            definitions.push(self.definition()?);
        }
        if definitions.is_empty() {
            return self.err("Expected an operation");
        }
        Ok(Document { definitions })
    }

    fn definition(&mut self) -> R<Definition> {
        let start = self.span.start;
        match self.tok.clone() {
            Tok::Punct('{') => {
                let selection = self.selection_set()?;
                Ok(Definition::Operation(OperationDef {
                    kind: OpKind::Query,
                    name: None,
                    variables: Vec::new(),
                    selection,
                    span: start..self.prev_end,
                }))
            }
            Tok::Name(k) if matches!(k.as_str(), "query" | "mutation" | "subscription") => {
                self.bump()?;
                let kind = match k.as_str() {
                    "query" => OpKind::Query,
                    "mutation" => OpKind::Mutation,
                    _ => OpKind::Subscription,
                };
                let name = if let Tok::Name(_) = self.tok {
                    Some(self.name()?.0)
                } else {
                    None
                };
                let variables = if self.tok == Tok::Punct('(') {
                    self.var_defs()?
                } else {
                    Vec::new()
                };
                self.directives()?;
                let selection = self.selection_set()?;
                Ok(Definition::Operation(OperationDef {
                    kind,
                    name,
                    variables,
                    selection,
                    span: start..self.prev_end,
                }))
            }
            Tok::Name(k) if k == "fragment" => {
                self.bump()?;
                let (name, _) = self.name()?;
                match &self.tok {
                    Tok::Name(on) if on == "on" => {
                        self.bump()?;
                    }
                    _ => return self.err("Expected 'on' in a fragment definition"),
                }
                let (type_condition, _) = self.name()?;
                self.directives()?;
                let selection = self.selection_set()?;
                Ok(Definition::Fragment(FragmentDef {
                    name,
                    type_condition,
                    selection,
                    span: start..self.prev_end,
                }))
            }
            _ => self.err(format!(
                "Expected 'query', 'mutation', 'subscription', 'fragment' or '{{' but found {}",
                self.describe()
            )),
        }
    }

    fn var_defs(&mut self) -> R<Vec<VarDef>> {
        self.expect('(')?;
        let mut out = Vec::new();
        while self.tok != Tok::Punct(')') {
            let start = self.span.start;
            self.expect('$')?;
            let (name, _) = self.name()?;
            self.expect(':')?;
            let ty = self.type_ref()?;
            let default = if self.eat('=')? {
                let s = self.span.start;
                self.value(true)?;
                Some(self.src[s..self.prev_end].to_string())
            } else {
                None
            };
            self.directives()?;
            out.push(VarDef {
                name,
                ty,
                default,
                span: start..self.prev_end,
            });
        }
        self.expect(')')?;
        Ok(out)
    }

    fn type_ref(&mut self) -> R<String> {
        let start = self.span.start;
        if self.eat('[')? {
            self.type_ref()?;
            self.expect(']')?;
        } else {
            self.name()?;
        }
        self.eat('!')?;
        Ok(self.src[start..self.prev_end].to_string())
    }

    fn directives(&mut self) -> R<()> {
        while self.eat('@')? {
            self.name()?;
            if self.tok == Tok::Punct('(') {
                self.args()?;
            }
        }
        Ok(())
    }

    fn selection_set(&mut self) -> R<SelectionSet> {
        let start = self.span.start;
        self.expect('{')?;
        let mut items = Vec::new();
        while self.tok != Tok::Punct('}') {
            if self.tok == Tok::Eof {
                return self.err("Expected '}' to close the selection set");
            }
            items.push(self.selection()?);
        }
        if items.is_empty() {
            return self.err("A selection set cannot be empty");
        }
        self.expect('}')?;
        Ok(SelectionSet {
            items,
            span: start..self.prev_end,
        })
    }

    fn selection(&mut self) -> R<Selection> {
        let start = self.span.start;
        if self.tok == Tok::Spread {
            self.bump()?;
            if let Tok::Name(n) = self.tok.clone()
                && n != "on"
            {
                self.bump()?;
                self.directives()?;
                return Ok(Selection::Spread {
                    name: n,
                    span: start..self.prev_end,
                });
            }
            let type_condition = match &self.tok {
                Tok::Name(on) if on == "on" => {
                    self.bump()?;
                    Some(self.name()?.0)
                }
                _ => None,
            };
            self.directives()?;
            let selection = self.selection_set()?;
            return Ok(Selection::Inline {
                type_condition,
                selection,
                span: start..self.prev_end,
            });
        }
        let (first, first_span) = self.name()?;
        let (alias, name, name_span) = if self.eat(':')? {
            let (n, s) = self.name()?;
            (Some(first), n, s)
        } else {
            (None, first, first_span)
        };
        let args = if self.tok == Tok::Punct('(') {
            self.args()?
        } else {
            Vec::new()
        };
        self.directives()?;
        let selection = if self.tok == Tok::Punct('{') {
            Some(self.selection_set()?)
        } else {
            None
        };
        Ok(Selection::Field(Field {
            alias,
            name,
            args,
            selection,
            span: start..self.prev_end,
            name_span,
        }))
    }

    fn args(&mut self) -> R<Vec<Arg>> {
        self.expect('(')?;
        let mut out = Vec::new();
        while self.tok != Tok::Punct(')') {
            let start = self.span.start;
            let (name, name_span) = self.name()?;
            self.expect(':')?;
            let vs = self.span.start;
            let value = self.value(false)?;
            out.push(Arg {
                name,
                value,
                text: self.src[vs..self.prev_end].to_string(),
                span: start..self.prev_end,
                name_span,
            });
        }
        if out.is_empty() {
            return self.err("Arguments cannot be empty");
        }
        self.expect(')')?;
        Ok(out)
    }

    fn value(&mut self, is_const: bool) -> R<Value> {
        let tok = self.tok.clone();
        match tok {
            Tok::Punct('$') => {
                if is_const {
                    return self.err("A variable is not allowed here");
                }
                self.bump()?;
                Ok(Value::Variable(self.name()?.0))
            }
            Tok::Int => {
                self.bump()?;
                Ok(Value::Int)
            }
            Tok::Float => {
                self.bump()?;
                Ok(Value::Float)
            }
            Tok::Str => {
                self.bump()?;
                Ok(Value::String)
            }
            Tok::Name(n) => {
                self.bump()?;
                Ok(match n.as_str() {
                    "true" | "false" => Value::Bool,
                    "null" => Value::Null,
                    _ => Value::Enum(n),
                })
            }
            Tok::Punct('[') => {
                self.bump()?;
                let mut items = Vec::new();
                while self.tok != Tok::Punct(']') {
                    if self.tok == Tok::Eof {
                        return self.err("Expected ']' to close the list");
                    }
                    items.push(self.value(is_const)?);
                }
                self.expect(']')?;
                Ok(Value::List(items))
            }
            Tok::Punct('{') => {
                self.bump()?;
                let mut fields = Vec::new();
                while self.tok != Tok::Punct('}') {
                    if self.tok == Tok::Eof {
                        return self.err("Expected '}' to close the object");
                    }
                    let (n, _) = self.name()?;
                    self.expect(':')?;
                    fields.push((n, self.value(is_const)?));
                }
                self.expect('}')?;
                Ok(Value::Object(fields))
            }
            _ => self.err(format!("Expected a value but found {}", self.describe())),
        }
    }
}

/// Parse an executable document.
pub fn parse(src: &str) -> Result<Document, ParseError> {
    Parser::new(src)?.document()
}

impl Document {
    pub fn operations(&self) -> impl Iterator<Item = &OperationDef> {
        self.definitions.iter().filter_map(|d| match d {
            Definition::Operation(o) => Some(o),
            _ => None,
        })
    }

    pub fn fragment(&self, name: &str) -> Option<&FragmentDef> {
        self.definitions.iter().find_map(|d| match d {
            Definition::Fragment(f) if f.name == name => Some(f),
            _ => None,
        })
    }
}

/// 1-based line and column of `offset` in `src` (columns count characters).
pub fn line_col(src: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(src.len());
    let before = &src[..offset];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, col)
}

/// Byte offset of a 1-based line and column; clamped to the end of the text.
pub fn offset_of(src: &str, line: usize, col: usize) -> usize {
    let mut off = 0;
    for (i, l) in src.split('\n').enumerate() {
        if i + 1 == line {
            let c = l
                .char_indices()
                .nth(col.saturating_sub(1))
                .map_or(l.len(), |(b, _)| b);
            return off + c;
        }
        off += l.len() + 1;
    }
    src.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_operation_with_variables_args_and_nesting() {
        let src = "query Pay($id: ID!, $n: Int = 3) {\n  payment(id: $id, first: $n) { status amount ...F }\n}\nfragment F on Payment { currency }";
        let d = parse(src).unwrap();
        let op = d.operations().next().unwrap();
        assert_eq!(op.name.as_deref(), Some("Pay"));
        assert_eq!(op.variables.len(), 2);
        assert_eq!(op.variables[1].default.as_deref(), Some("3"));
        let Selection::Field(f) = &op.selection.items[0] else {
            panic!()
        };
        assert_eq!(f.name, "payment");
        assert_eq!(f.args[0].value, Value::Variable("id".into()));
        assert_eq!(f.selection.as_ref().unwrap().items.len(), 3);
        assert!(d.fragment("F").is_some());
    }

    #[test]
    fn shorthand_query_alias_and_values() {
        let d = parse("{ a: user(filter: {tags: [\"x\", \"y\"], on: true, kind: ADMIN}) { id } }")
            .unwrap();
        let Selection::Field(f) = &d.operations().next().unwrap().selection.items[0] else {
            panic!()
        };
        assert_eq!(f.alias.as_deref(), Some("a"));
        assert_eq!(f.response_key(), "a");
        assert!(matches!(f.args[0].value, Value::Object(_)));
    }

    #[test]
    fn errors_carry_offsets() {
        let e = parse("{ a { b }").unwrap_err();
        assert!(e.message.contains("'}'"), "{}", e.message);
        let e = parse("query { }").unwrap_err();
        assert!(e.message.contains("empty"));
        assert!(parse("").is_err());
        assert!(parse("{ a(x: ) }").is_err());
    }

    #[test]
    fn comments_commas_and_block_strings() {
        let src = "# hi\n{ a(s: \"\"\"x \" y\"\"\", n: -1.5e3,) , b }";
        assert!(parse(src).is_ok());
    }

    #[test]
    fn line_and_offset_round_trip() {
        let src = "ab\ncd€e\nf";
        let o = src.find('e').unwrap();
        let (l, c) = line_col(src, o);
        assert_eq!((l, c), (2, 4));
        assert_eq!(offset_of(src, l, c), o);
    }

    #[test]
    fn inline_fragments_and_directives() {
        let d = parse("{ n { ... on User @include(if: true) { id } ... { x } } }").unwrap();
        assert_eq!(d.operations().count(), 1);
    }
}
