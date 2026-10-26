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

/// Write `env` to its two files. Secret rows go to `.local.env`, others to `.env`.
/// A file that would end up empty and did not exist is not created.
pub fn save_environment(root: &Path, env: &Environment) -> io::Result<()> {
    std::fs::create_dir_all(root.join(ENV_DIR))?;
    for local in [false, true] {
        let path = if local {
            local_env_path(root, &env.name)
        } else {
            env_path(root, &env.name)
        };
        let entries: Vec<(String, String)> = env
            .rows
            .iter()
            .filter(|r| (r.secret || r.local) == local && !r.name.trim().is_empty())
            .map(|r| (r.name.trim().to_string(), r.value.clone()))
            .collect();
        let old = std::fs::read_to_string(&path).ok();
        if old.is_none() && entries.is_empty() && local {
            continue;
        }
        let text = rewrite_dotenv(old.as_deref().unwrap_or(""), &entries);
        if old.as_deref() != Some(text.as_str()) {
            std::fs::write(&path, text)?;
        }
    }
    Ok(())
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
}
