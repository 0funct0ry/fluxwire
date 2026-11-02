//! Assembles planned files: unique names, folders, environments, config and `.gitignore`.

use crate::plan::{ImportPlan, ImportReport, PlannedFile, PlannedKind};
use crate::render::{FolderDraft, ReqDraft, render_folder, render_request};
use crate::secrets::{dotenv, looks_secret, var_name};
use fluxwire_format::slug;
use fluxwire_model::FileKind;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub struct Builder {
    pub files: Vec<PlannedFile>,
    pub report: ImportReport,
    used: BTreeSet<PathBuf>,
    secret_names: BTreeSet<String>,
    pub extra_toml: String,
}

impl Builder {
    pub fn new() -> Self {
        Builder {
            files: Vec::new(),
            report: ImportReport::default(),
            used: BTreeSet::new(),
            secret_names: BTreeSet::new(),
            extra_toml: String::new(),
        }
    }

    /// A relative path under `dir` named after `name`, made unique with `-2`, `-3`.
    pub fn unique(&mut self, dir: &Path, name: &str, ext: &str) -> PathBuf {
        let stem = slug(name);
        let mut p = dir.join(format!("{stem}{ext}"));
        let mut n = 2;
        while self.used.contains(&p) {
            p = dir.join(format!("{stem}-{n}{ext}"));
            n += 1;
        }
        self.used.insert(p.clone());
        p
    }

    /// A unique folder name under `dir` (so two groups with one name do not merge).
    pub fn unique_dir(&mut self, dir: &Path, name: &str) -> PathBuf {
        let stem = slug(name);
        let mut p = dir.join(&stem);
        let mut n = 2;
        while self.used.contains(&p) {
            p = dir.join(format!("{stem}-{n}"));
            n += 1;
        }
        self.used.insert(p.clone());
        p
    }

    pub fn push(&mut self, path: PathBuf, contents: String, kind: PlannedKind) {
        self.used.insert(path.clone());
        self.files.push(PlannedFile {
            path,
            contents,
            kind,
        });
    }

    /// Reserves the path a request named `name` will be written to.
    pub fn reserve_request(&mut self, dir: &Path, name: &str) -> PathBuf {
        self.unique(dir, name, ".flux.md")
    }

    pub fn add_request_at(&mut self, path: PathBuf, d: &ReqDraft) {
        self.files.push(PlannedFile {
            path,
            contents: render_request(d),
            kind: PlannedKind::Request,
        });
    }

    pub fn add_folder(&mut self, dir: &Path, d: &FolderDraft) {
        self.push(
            dir.join("folder.flux.md"),
            render_folder(d, FileKind::Folder),
            PlannedKind::Folder,
        );
    }

    pub fn add_collection(&mut self, d: &FolderDraft) {
        self.push(
            PathBuf::from("collection.flux.md"),
            render_folder(d, FileKind::Collection),
            PlannedKind::Collection,
        );
    }

    /// `rows`: (name, value, explicitly secret). Secrets go to `<env>.local.env`.
    pub fn add_env(&mut self, name: &str, rows: &[(String, String, bool)]) {
        let stem = slug(name);
        let mut shared = Vec::new();
        let mut local = Vec::new();
        for (k, v, secret) in rows {
            let k2 = var_name(k);
            if k2 != *k {
                self.report.lossy(
                    &format!("environments/{stem}.env"),
                    format!("variable `{k}`"),
                    format!("Not a valid variable name; renamed to `{k2}`"),
                );
            }
            if *secret || looks_secret(&k2) {
                self.secret_names.insert(k2.clone());
                local.push((k2, v.clone()));
            } else {
                shared.push((k2, v.clone()));
            }
        }
        let env = PathBuf::from("environments").join(format!("{stem}.env"));
        self.push(env, dotenv(&shared), PlannedKind::Env);
        if !local.is_empty() {
            let p = PathBuf::from("environments").join(format!("{stem}.local.env"));
            self.push(p, dotenv(&local), PlannedKind::LocalEnv);
        }
    }

    pub fn finish(mut self, collection_name: &str) -> ImportPlan {
        let mut toml = format!(
            "[collection]\nname = {}\nformat = 1\n",
            toml_string(collection_name)
        );
        if !self.secret_names.is_empty() {
            let names: Vec<String> = self.secret_names.iter().map(|n| toml_string(n)).collect();
            toml.push_str(&format!("\n[secrets]\nnames = [{}]\n", names.join(", ")));
        }
        if !self.extra_toml.is_empty() {
            toml.push('\n');
            toml.push_str(&self.extra_toml);
        }
        self.files.push(PlannedFile {
            path: PathBuf::from("fluxwire.toml"),
            contents: toml,
            kind: PlannedKind::Config,
        });
        self.files.push(PlannedFile {
            path: PathBuf::from(".gitignore"),
            contents: "*.local.env\nfluxwire.local.toml\n.fluxwire/cache/\n".into(),
            kind: PlannedKind::Gitignore,
        });
        ImportPlan {
            files: self.files,
            report: self.report,
        }
    }
}

pub fn toml_string(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', " ")
    )
}

/// Split `a=b&c=d` (no leading `?`) into pairs, keeping `{{vars}}` intact.
pub fn split_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|s| !s.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (k.to_string(), v.to_string()),
            None => (p.to_string(), String::new()),
        })
        .collect()
}

/// Split `url?query` into the base and its query pairs.
pub fn split_url(url: &str) -> (String, Vec<(String, String)>) {
    match url.split_once('?') {
        Some((b, q)) if !q.contains("{{") || q.contains('=') => (b.to_string(), split_query(q)),
        _ => (url.to_string(), Vec::new()),
    }
}
