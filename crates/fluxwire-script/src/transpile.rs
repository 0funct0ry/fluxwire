//! TypeScript stripping (oxc) with a line map back to the `.flux.md` file.

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_codegen::{Codegen, CodegenOptions};
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_transformer::{TransformOptions, Transformer};

use crate::{Lang, ScriptError};

/// A block turned into plain JavaScript plus a way to map generated lines back.
pub struct Transpiled {
    pub js: String,
    /// `map[generated_line0] = source_line0` (0-based, within the block text).
    lines: Vec<u32>,
}

impl Transpiled {
    /// Source line (0-based, within the block) for a 1-based generated line.
    pub fn source_line(&self, generated_line: u32) -> u32 {
        let idx = generated_line.saturating_sub(1) as usize;
        self.lines
            .get(idx)
            .copied()
            .or_else(|| self.lines.last().copied())
            .unwrap_or(0)
    }
}

/// Strips types from TypeScript (no type checking); JavaScript is parsed and re-printed
/// so both languages share one error-mapping path.
pub fn transpile(
    source: &str,
    lang: Lang,
    file: &str,
    first_line: u32,
) -> Result<Transpiled, ScriptError> {
    let allocator = Allocator::default();
    let path = Path::new(match lang {
        Lang::Ts => "block.ts",
        Lang::Js => "block.js",
    });
    let source_type = SourceType::from_path(path)
        .map_err(|_| ScriptError::syntax("unsupported script language", file, first_line))?
        .with_module(true);

    let parsed = Parser::new(&allocator, source, source_type).parse();
    if let Some(err) = parsed.errors.first() {
        let line = err
            .labels
            .as_ref()
            .and_then(|l| l.first())
            .map(|l| line_of(source, l.offset()))
            .unwrap_or(0);
        return Err(ScriptError::syntax(&err.message, file, first_line + line));
    }
    let mut program = parsed.program;

    let scoping = SemanticBuilder::new()
        .build(&program)
        .semantic
        .into_scoping();
    let options = TransformOptions::default();
    let ret =
        Transformer::new(&allocator, path, &options).build_with_scoping(scoping, &mut program);
    if let Some(err) = ret.errors.first() {
        return Err(ScriptError::syntax(&err.message, file, first_line));
    }

    let out = Codegen::new()
        .with_options(CodegenOptions {
            source_map_path: Some(path.to_path_buf()),
            ..CodegenOptions::default()
        })
        .build(&program);

    let mut lines = Vec::new();
    if let Some(map) = &out.map {
        let table = map.generate_lookup_table();
        let total = out.code.lines().count() as u32;
        let mut last = 0;
        for l in 0..total.max(1) {
            if let Some(t) = map.lookup_token(&table, l, 0) {
                last = t.get_src_line();
            } else if let Some(t) = map.get_tokens().find(|t| t.get_dst_line() == l) {
                last = t.get_src_line();
            }
            lines.push(last);
        }
    }
    Ok(Transpiled {
        js: out.code,
        lines,
    })
}

fn line_of(source: &str, offset: usize) -> u32 {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|b| *b == b'\n')
        .count() as u32
}
