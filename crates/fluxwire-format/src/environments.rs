//! Environment files: `environments/<name>.env` (committed) and `<name>.local.env`
//! (git-ignored, always secret) per SPEC §6 and §8.3.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use fluxwire_vars::{DotEnvEntry, parse_dotenv, write_dotenv_value};

pub const ENV_DIR: &str = "environments";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvRow {
    pub name: String,
    pub value: String,
    pub secret: bool,
    /// Stored in the `.local.env` file.
    pub local: bool,
    /// 1-based line in its file; 0 for rows not yet saved.
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Environment {
    pub name: String,
    pub rows: Vec<EnvRow>,
}

pub fn env_path(root: &Path, name: &str) -> PathBuf {
    root.join(ENV_DIR).join(format!("{name}.env"))
}

pub fn local_env_path(root: &Path, name: &str) -> PathBuf {
    root.join(ENV_DIR).join(format!("{name}.local.env"))
}

/// Environment names found on disk, sorted. `x.env` and `x.local.env` both yield `x`.
pub fn list_environments(root: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(root.join(ENV_DIR)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = rd
        .flatten()
        .filter_map(|e| {
            let f = e.file_name().to_string_lossy().into_owned();
            let stem = f.strip_suffix(".env")?;
            let stem = stem.strip_suffix(".local").unwrap_or(stem);
            (!stem.is_empty()).then(|| stem.to_string())
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

pub fn load_environment(root: &Path, name: &str, secret_names: &[String]) -> Environment {
    let read = |p: PathBuf| std::fs::read_to_string(p).unwrap_or_default();
    let mut rows = Vec::new();
    for (entries, local) in [
        (parse_dotenv(&read(env_path(root, name))), false),
        (parse_dotenv(&read(local_env_path(root, name))), true),
    ] {
        for DotEnvEntry { name, value, line } in entries {
            rows.push(EnvRow {
                secret: local || secret_names.contains(&name),
                name,
                value,
                local,
                line,
            });
        }
    }
    Environment {
        name: name.to_string(),
        rows,
    }
}

/// Rewrite `old` so that it holds exactly `entries`, leaving comments, blank lines and
/// untouched assignments byte for byte. New names are appended.
pub fn rewrite_dotenv(old: &str, entries: &[(String, String)]) -> String {
    let reordered = reorder_dotenv(old, entries);
    let old = reordered.as_deref().unwrap_or(old);
    let mut out = String::new();
    let mut seen: Vec<&str> = Vec::new();
    for raw in old.split_inclusive('\n') {
        let line = raw.trim();
        let assign = (!line.is_empty() && !line.starts_with('#'))
            .then(|| {
                line.strip_prefix("export ")
                    .unwrap_or(line)
                    .split_once('=')
                    .map(|(n, _)| n.trim())
            })
            .flatten();
        let Some(n) = assign else {
            out.push_str(raw);
            continue;
        };
        // A name that is no longer in `entries` was deleted: drop its line. Unchanged
        // values keep their original text (quoting, comments).
        if let Some((k, v)) = entries.iter().find(|(k, _)| k == n) {
            seen.push(k);
            let cur = parse_dotenv(raw).into_iter().next();
            if cur.is_some_and(|c| &c.value == v) {
                out.push_str(raw);
            } else {
                out.push_str(&format!("{k}={}\n", write_dotenv_value(v)));
            }
        }
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    for (k, v) in entries {
        if !seen.contains(&k.as_str()) {
            out.push_str(&format!("{k}={}\n", write_dotenv_value(v)));
        }
    }
    out
}

/// True when `name` is a legal variable name (SPEC §8.1).
pub fn is_valid_var_name(name: &str) -> bool {
    let mut cs = name.chars();
    cs.next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && cs.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Why an environment name is refused, as a message for the New/Rename dialogs. `existing` are the
/// collection's other environment names; matching is case-insensitive when `case_insensitive`
/// (macOS and Windows file systems).
pub fn validate_environment_name(
    name: &str,
    existing: &[String],
    case_insensitive: bool,
) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Enter a name.".into());
    }
    if name != name.trim() {
        return Err("The name cannot start or end with a space.".into());
    }
    if let Some(c) = name
        .chars()
        .find(|c| c.is_control() || "<>:\"/\\|?*".contains(*c))
    {
        return Err(format!("The name cannot contain `{}`.", c.escape_default()));
    }
    if name.starts_with('.') {
        return Err("The name cannot start with a dot.".into());
    }
    if name.ends_with('.') {
        return Err("The name cannot end with a dot.".into());
    }
    if name.ends_with(".local") {
        return Err("The name cannot end with `.local`.".into());
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || ["COM", "LPT"].iter().any(|p| {
            stem.strip_prefix(p)
                .is_some_and(|d| d.len() == 1 && d.as_bytes()[0].is_ascii_digit() && d != "0")
        });
    if reserved {
        return Err(format!("`{stem}` is a reserved name on Windows."));
    }
    let clash = existing.iter().any(|e| {
        if case_insensitive {
            e.eq_ignore_ascii_case(name)
        } else {
            e == name
        }
    });
    if clash {
        return Err(format!("An environment named `{name}` already exists."));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvSeverity {
    Error,
    Warning,
    Info,
}

/// One validation finding on a row (SPEC §8.6). `row` indexes `Environment::rows`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvDiagnostic {
    pub code: &'static str,
    pub severity: EnvSeverity,
    pub row: usize,
    pub message: String,
}

/// E101–E104 for the rows of one environment. `secret_names` decides which file a row lives in.
pub fn diagnose_rows(env: &Environment, secret_names: &[String]) -> Vec<EnvDiagnostic> {
    let mut out = Vec::new();
    for (i, r) in env.rows.iter().enumerate() {
        let name = r.name.trim();
        let mut push = |code, severity, message: String| {
            out.push(EnvDiagnostic {
                code,
                severity,
                row: i,
                message,
            });
        };
        if name.is_empty() {
            if !r.value.is_empty() {
                push(
                    "E103",
                    EnvSeverity::Warning,
                    "This value has no name and will not be saved.".into(),
                );
            }
            continue;
        }
        if !is_valid_var_name(name) {
            push(
                "E101",
                EnvSeverity::Error,
                format!("`{name}` is not a valid variable name; the row will not be saved."),
            );
        }
        let file = file_is_local(r, secret_names);
        let later = env.rows[i + 1..]
            .iter()
            .any(|o| o.name.trim() == name && file_is_local(o, secret_names) == file);
        if later {
            push(
                "E102",
                EnvSeverity::Error,
                format!("`{name}` is defined again further down; the later row wins."),
            );
        }
        if r.value.contains('\n') {
            push(
                "E104",
                EnvSeverity::Info,
                "Multi-line value; edit it in a text editor.".into(),
            );
        }
    }
    out
}

fn file_is_local(r: &EnvRow, secret_names: &[String]) -> bool {
    r.local || (r.secret && !secret_names.contains(&r.name))
}

/// Minimal glob: `*` matches any run of characters, everything else is literal.
fn glob_match(pat: &str, text: &str) -> bool {
    match pat.split_once('*') {
        None => pat == text,
        Some((head, rest)) => {
            let Some(after) = text.strip_prefix(head) else {
                return false;
            };
            (0..=after.len())
                .filter(|i| after.is_char_boundary(*i))
                .any(|i| glob_match(rest, &after[i..]))
        }
    }
}

/// True when `*.local.env` files under `root` are git-ignored (E106 otherwise). Asks
/// `git check-ignore` when `root` is inside a repository (so global ignores, parent
/// `.gitignore` files and `*.env` all count); otherwise reads `root/.gitignore` and matches its
/// patterns against `environments/x.local.env`.
pub fn gitignore_ignores_local_env(root: &Path) -> bool {
    if let Ok(o) = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["check-ignore", "-q", "--", "environments/x.local.env"])
        .output()
    {
        match o.status.code() {
            Some(0) => return true,
            Some(1) => return false,
            _ => {}
        }
    }
    std::fs::read_to_string(root.join(".gitignore")).is_ok_and(|t| {
        t.lines().map(str::trim).any(|l| {
            if l.is_empty() || l.starts_with('#') || l.starts_with('!') {
                return false;
            }
            let raw = l.trim_start_matches('/').trim_start_matches("**/");
            if raw.trim_end_matches('/') == "environments" {
                return true;
            }
            let p = raw.strip_prefix("environments/").unwrap_or(raw);
            glob_match(p, "x.local.env")
        })
    })
}

fn assignment_name(raw: &str) -> Option<&str> {
    let line = raw.trim();
    (!line.is_empty() && !line.starts_with('#'))
        .then(|| {
            line.strip_prefix("export ")
                .unwrap_or(line)
                .split_once('=')
                .map(|(n, _)| n.trim())
        })
        .flatten()
}

/// Move assignments (each with the comment and blank lines directly above it) so the keys that
/// survive appear in the order of `entries`. `None` when the order already matches.
fn reorder_dotenv(old: &str, entries: &[(String, String)]) -> Option<String> {
    let mut segs: Vec<(Option<&str>, String)> = Vec::new();
    let mut cur = String::new();
    for raw in old.split_inclusive('\n') {
        cur.push_str(raw);
        if let Some(n) = assignment_name(raw) {
            segs.push((Some(n), std::mem::take(&mut cur)));
        }
    }
    let pos = |k: &str| entries.iter().position(|(e, _)| e == k);
    let slots: Vec<usize> = (0..segs.len())
        .filter(|i| segs[*i].0.is_some_and(|k| pos(k).is_some()))
        .collect();
    let mut order: Vec<usize> = slots.clone();
    order.sort_by_key(|i| pos(segs[*i].0.unwrap_or_default()));
    if order == slots {
        return None;
    }
    let mut out_segs: Vec<String> = segs.iter().map(|(_, t)| t.clone()).collect();
    for (slot, from) in slots.iter().zip(&order) {
        let mut t = segs[*from].1.clone();
        if !t.ends_with('\n') {
            t.push('\n');
        }
        out_segs[*slot] = t;
    }
    out_segs.push(cur);
    Some(out_segs.concat())
}

/// Rename the assignment `from` to `to` on its own line, leaving everything else (including the
/// value's quoting and the line number) untouched.
pub fn rename_key_in_place(old: &str, from: &str, to: &str) -> String {
    let mut out = String::new();
    for raw in old.split_inclusive('\n') {
        if assignment_name(raw) == Some(from)
            && let Some(at) = raw.find(from)
        {
            out.push_str(&raw[..at]);
            out.push_str(to);
            out.push_str(&raw[at + from.len()..]);
            continue;
        }
        out.push_str(raw);
    }
    out
}

/// Write `env` to its two files; see [`save_environment_with`].
pub fn save_environment(root: &Path, env: &Environment) -> io::Result<()> {
    save_environment_with(root, env, &[]).map(|_| ())
}

/// Write `env` to its two files. A row is written to `.local.env` when it lives there or is secret
/// without being secret by the collection's `[secrets]` list (`secret_names`); other rows go to
/// `.env`. Rows with an invalid or empty name are skipped; the count is returned. Of two rows with
/// the same name in one file the later wins. A `.local.env` that would be empty and did not exist
/// is not created, and a file whose text is unchanged is not touched.
pub fn save_environment_with(
    root: &Path,
    env: &Environment,
    secret_names: &[String],
) -> io::Result<usize> {
    std::fs::create_dir_all(root.join(ENV_DIR))?;
    let skipped = env
        .rows
        .iter()
        .filter(|r| !is_valid_var_name(r.name.trim()))
        .filter(|r| !(r.name.trim().is_empty() && r.value.is_empty()))
        .count();
    for local in [false, true] {
        let path = if local {
            local_env_path(root, &env.name)
        } else {
            env_path(root, &env.name)
        };
        let mut entries: Vec<(String, String)> = Vec::new();
        for r in env
            .rows
            .iter()
            .filter(|r| file_is_local(r, secret_names) == local && is_valid_var_name(r.name.trim()))
        {
            let n = r.name.trim().to_string();
            entries.retain(|(k, _)| *k != n);
            entries.push((n, r.value.clone()));
        }
        let old = std::fs::read_to_string(&path).ok();
        if old.is_none() && entries.is_empty() && local {
            continue;
        }
        let text = rewrite_dotenv(old.as_deref().unwrap_or(""), &entries);
        if old.as_deref() != Some(text.as_str()) {
            std::fs::write(&path, text)?;
        }
    }
    Ok(skipped)
}

pub fn create_environment(root: &Path, name: &str) -> io::Result<()> {
    let path = env_path(root, name);
    if path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("Environment `{name}` already exists."),
        ));
    }
    std::fs::create_dir_all(root.join(ENV_DIR))?;
    std::fs::write(path, "")
}

pub fn rename_environment(root: &Path, from: &str, to: &str) -> io::Result<()> {
    if list_environments(root).iter().any(|n| n == to) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("Environment `{to}` already exists."),
        ));
    }
    for (a, b) in [
        (env_path(root, from), env_path(root, to)),
        (local_env_path(root, from), local_env_path(root, to)),
    ] {
        if a.exists() {
            std::fs::rename(a, b)?;
        }
    }
    Ok(())
}

pub fn duplicate_environment(root: &Path, from: &str) -> io::Result<String> {
    let names = list_environments(root);
    let mut n = 1;
    let to = loop {
        let c = if n == 1 {
            format!("{from}-copy")
        } else {
            format!("{from}-copy-{n}")
        };
        if !names.contains(&c) {
            break c;
        }
        n += 1;
    };
    for (a, b) in [
        (env_path(root, from), env_path(root, &to)),
        (local_env_path(root, from), local_env_path(root, &to)),
    ] {
        if a.exists() {
            std::fs::copy(a, b)?;
        }
    }
    Ok(to)
}

/// Move both files to the OS trash (never a hard delete).
pub fn delete_environment(root: &Path, name: &str) -> io::Result<()> {
    delete_environment_with(root, name, |p| trash::delete(p).map_err(io::Error::other))
}

/// `delete_environment` with the removal strategy injected (tests cannot use the trash).
pub fn delete_environment_with(
    root: &Path,
    name: &str,
    remove: impl Fn(&Path) -> io::Result<()>,
) -> io::Result<()> {
    for p in [env_path(root, name), local_env_path(root, name)] {
        if p.exists() {
            remove(&p)?;
        }
    }
    Ok(())
}

/// True when git tracks `<name>.local.env`, which would publish secrets. Silent
/// (false) when `git` is missing or the directory is not a repository.
pub fn local_env_is_tracked(root: &Path, name: &str) -> bool {
    let rel = format!("{ENV_DIR}/{name}.local.env");
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "--error-unmatch", &rel])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fw-env-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join(ENV_DIR)).unwrap();
        d
    }

    #[test]
    fn loads_public_and_local_rows_with_lines() {
        let d = scratch("load");
        std::fs::write(
            env_path(&d, "staging"),
            "# hi\nbaseUrl=https://s\napiKey=k\n",
        )
        .unwrap();
        std::fs::write(local_env_path(&d, "staging"), "token=t\n").unwrap();
        let e = load_environment(&d, "staging", &["apiKey".into()]);
        let got: Vec<_> = e
            .rows
            .iter()
            .map(|r| (&*r.name, r.secret, r.local, r.line))
            .collect();
        assert_eq!(
            got,
            vec![
                ("baseUrl", false, false, 2),
                ("apiKey", true, false, 3),
                ("token", true, true, 1)
            ]
        );
        assert_eq!(list_environments(&d), vec!["staging"]);
    }

    #[test]
    fn rewrite_preserves_comments_and_untouched_lines() {
        let old = "# note\nA=1\n\nB=\"two words\"\nC=3\n";
        let new = rewrite_dotenv(
            old,
            &[
                ("A".into(), "1".into()),
                ("B".into(), "two words".into()),
                ("D".into(), "4".into()),
            ],
        );
        assert_eq!(new, "# note\nA=1\n\nB=\"two words\"\nD=4\n");
        let changed = rewrite_dotenv(
            old,
            &[
                ("A".into(), "9".into()),
                ("B".into(), "two words".into()),
                ("C".into(), "3".into()),
            ],
        );
        assert_eq!(changed, "# note\nA=9\n\nB=\"two words\"\nC=3\n");
    }

    #[test]
    fn save_routes_secrets_to_local_file() {
        let d = scratch("save");
        let env = Environment {
            name: "dev".into(),
            rows: vec![
                EnvRow {
                    name: "url".into(),
                    value: "http://x".into(),
                    secret: false,
                    local: false,
                    line: 0,
                },
                EnvRow {
                    name: "pw".into(),
                    value: "s3".into(),
                    secret: true,
                    local: false,
                    line: 0,
                },
            ],
        };
        save_environment(&d, &env).unwrap();
        assert_eq!(
            std::fs::read_to_string(env_path(&d, "dev")).unwrap(),
            "url=http://x\n"
        );
        assert_eq!(
            std::fs::read_to_string(local_env_path(&d, "dev")).unwrap(),
            "pw=s3\n"
        );
    }

    #[test]
    fn create_rename_duplicate_delete() {
        let d = scratch("ops");
        create_environment(&d, "a").unwrap();
        assert!(create_environment(&d, "a").is_err());
        std::fs::write(local_env_path(&d, "a"), "k=v\n").unwrap();
        rename_environment(&d, "a", "b").unwrap();
        assert_eq!(list_environments(&d), vec!["b"]);
        assert_eq!(duplicate_environment(&d, "b").unwrap(), "b-copy");
        assert!(local_env_path(&d, "b-copy").exists());
        assert!(rename_environment(&d, "b", "b-copy").is_err());
        delete_environment_with(&d, "b-copy", |p| std::fs::remove_file(p)).unwrap();
        assert_eq!(list_environments(&d), vec!["b"]);
    }

    #[test]
    fn untracked_outside_repo_is_false() {
        let d = scratch("git");
        assert!(!local_env_is_tracked(&d, "x"));
    }

    fn row(name: &str, value: &str, local: bool) -> EnvRow {
        EnvRow {
            name: name.into(),
            value: value.into(),
            secret: local,
            local,
            line: 0,
        }
    }

    #[test]
    fn name_validation_rules() {
        let ex = vec!["staging".to_string()];
        let ok = |n: &str, ci| validate_environment_name(n, &ex, ci);
        assert!(ok("qa", true).is_ok());
        for bad in [
            "", " x", "a/b", "a\\b", "a:b", ".hidden", "x.local", "con", "NUL", "com1", "x.",
        ] {
            assert!(ok(bad, true).is_err(), "{bad:?}");
        }
        assert!(ok("com0", true).is_ok());
        assert!(ok("staging", false).is_err());
        assert!(ok("Staging", true).is_err());
        assert!(ok("Staging", false).is_ok());
    }

    #[test]
    fn diagnostics_cover_e101_to_e104() {
        let env = Environment {
            name: "x".into(),
            rows: vec![
                row("a", "1", false),
                row("a", "2", false),
                row("a", "3", true),
                row("1bad", "v", false),
                row("", "orphan", false),
                row("m", "l1\nl2", false),
            ],
        };
        let d: Vec<_> = diagnose_rows(&env, &[])
            .iter()
            .map(|d| (d.code, d.row))
            .collect();
        assert_eq!(d, vec![("E102", 0), ("E101", 3), ("E103", 4), ("E104", 5)]);
    }

    #[test]
    fn save_skips_invalid_rows_and_counts_them() {
        let d = scratch("skip");
        let env = Environment {
            name: "dev".into(),
            rows: vec![
                row("ok", "1", false),
                row("1bad", "2", false),
                row("", "3", false),
                row("", "", false),
            ],
        };
        assert_eq!(save_environment_with(&d, &env, &[]).unwrap(), 2);
        assert_eq!(
            std::fs::read_to_string(env_path(&d, "dev")).unwrap(),
            "ok=1\n"
        );
        assert!(!local_env_path(&d, "dev").exists());
    }

    #[test]
    fn reorder_moves_lines_with_their_comment_and_keeps_others() {
        let old = "# a\nA=1\n# b\nB=2\nC=3\n";
        let ent = |ks: &[(&str, &str)]| -> Vec<(String, String)> {
            ks.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        assert_eq!(
            rewrite_dotenv(old, &ent(&[("B", "2"), ("A", "1"), ("C", "3")])),
            "# b\nB=2\n# a\nA=1\nC=3\n"
        );
        assert_eq!(
            rewrite_dotenv(old, &ent(&[("A", "1"), ("B", "2"), ("C", "3")])),
            old
        );
    }

    #[test]
    fn rename_in_place_keeps_line_numbers() {
        let old = "# c\nA=1\nB=2\n";
        let new = rewrite_dotenv(old, &[("A2".into(), "1".into()), ("B".into(), "2".into())]);
        // A renamed key is a delete plus an append; a rename that keeps position is the UI's job
        // via `rename_key_in_place`.
        assert!(new.contains("B=2"));
        assert_eq!(rename_key_in_place(old, "A", "Z"), "# c\nZ=1\nB=2\n");
    }

    #[test]
    fn gitignore_check() {
        let d = scratch("ign");
        assert!(!gitignore_ignores_local_env(&d));
        std::fs::write(d.join(".gitignore"), "target\n*.local.env\n").unwrap();
        assert!(gitignore_ignores_local_env(&d));
        for ok in [
            "*.env",
            "environments/*.local.env",
            "/environments/",
            "**/*.local.env",
            "*.local.*",
        ] {
            std::fs::write(d.join(".gitignore"), format!("target\n{ok}\n")).unwrap();
            assert!(gitignore_ignores_local_env(&d), "{ok}");
        }
        for no in ["target", "*.md", "# *.env", "!*.local.env"] {
            std::fs::write(d.join(".gitignore"), format!("{no}\n")).unwrap();
            assert!(!gitignore_ignores_local_env(&d), "{no}");
        }
    }

    mod props {
        use super::*;
        use proptest::prelude::*;

        fn dotenv() -> impl Strategy<Value = String> {
            let line = prop_oneof![
                Just(String::new()),
                "# [a-z ]{0,8}".prop_map(|s| s),
                ("[A-Za-z_][A-Za-z0-9_]{0,5}", "[a-z0-9:/.]{0,8}")
                    .prop_map(|(k, v)| format!("{k}={v}")),
                ("[A-Za-z_][A-Za-z0-9_]{0,5}", "[a-z ]{1,8}")
                    .prop_map(|(k, v)| format!("export {k}=\"{v}\"")),
            ];
            proptest::collection::vec(line, 0..8).prop_map(|ls| {
                let mut t = ls.join("\n");
                if !t.is_empty() {
                    t.push('\n');
                }
                t
            })
        }

        fn entries_of(text: &str) -> Vec<(String, String)> {
            let mut out: Vec<(String, String)> = Vec::new();
            for e in parse_dotenv(text) {
                out.retain(|(k, _)| *k != e.name);
                out.push((e.name, e.value));
            }
            out
        }

        proptest! {
            #[test]
            fn load_then_save_without_edits_is_byte_exact(t in dotenv()) {
                // Duplicate keys collapse (last wins), so only unique-key files are exact.
                let es = entries_of(&t);
                prop_assume!(es.len() == parse_dotenv(&t).len());
                prop_assert_eq!(rewrite_dotenv(&t, &es), t);
            }

            #[test]
            fn save_is_idempotent(t in dotenv(), edit in "[a-z0-9]{0,6}") {
                let mut es = entries_of(&t);
                if let Some(e) = es.first_mut() { e.1 = edit; }
                es.push(("ZZ".into(), "new".into()));
                let once = rewrite_dotenv(&t, &es);
                prop_assert_eq!(rewrite_dotenv(&once, &es), once);
            }
        }
    }
}
