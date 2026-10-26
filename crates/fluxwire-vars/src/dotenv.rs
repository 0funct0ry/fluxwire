//! Dotenv files with line numbers for provenance (SPEC §6, §8.3).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DotEnvEntry {
    pub name: String,
    pub value: String,
    /// 1-based line number.
    pub line: usize,
}

/// Parse `NAME=value` lines. Blank lines and `#` comments are skipped; values may
/// be single- or double-quoted; an `export ` prefix is tolerated.
pub fn parse_dotenv(text: &str) -> Vec<DotEnvEntry> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        out.push(DotEnvEntry {
            name: name.to_string(),
            value: unquote(value.trim()),
            line: i + 1,
        });
    }
    out
}

fn unquote(v: &str) -> String {
    let b = v.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' && b[b.len() - 1] == b'"') {
        return v[1..v.len() - 1]
            .replace("\\n", "\n")
            .replace("\\\"", "\"")
            .replace("\\\\", "\\");
    }
    if b.len() >= 2 && b[0] == b'\'' && b[b.len() - 1] == b'\'' {
        return v[1..v.len() - 1].to_string();
    }
    // Unquoted: strip a trailing ` # comment`.
    match v.find(" #") {
        Some(i) => v[..i].trim_end().to_string(),
        None => v.to_string(),
    }
}

/// Render a value so `parse_dotenv` reads it back unchanged.
pub fn write_dotenv_value(v: &str) -> String {
    let plain = !v.is_empty() && !v.contains(['\n', '"', '\'', '#', '\\']) && v.trim() == v;
    if plain || v.is_empty() {
        v.to_string()
    } else {
        format!(
            "\"{}\"",
            v.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lines_comments_and_quotes() {
        let e = parse_dotenv("# c\n\nA=1\nexport B = \"x y\"\nC='q#'\nD=v # note\nbad\n");
        let got: Vec<_> = e.iter().map(|e| (&*e.name, &*e.value, e.line)).collect();
        assert_eq!(
            got,
            vec![
                ("A", "1", 3),
                ("B", "x y", 4),
                ("C", "q#", 5),
                ("D", "v", 6)
            ]
        );
    }

    #[test]
    fn written_values_round_trip() {
        for v in [
            "plain",
            "",
            "a b",
            "has \"q\"",
            "hash # x",
            "line\nbreak",
            " pad ",
        ] {
            let t = format!("K={}", write_dotenv_value(v));
            let e = parse_dotenv(&t);
            assert_eq!(e[0].value, v, "{t}");
        }
    }
}
