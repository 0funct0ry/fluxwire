//! The environment as a document (M9.02, ADR-011): rows live in memory, edits mark the document
//! dirty, undo steps are whole row sets, and only an explicit save writes the `.env` files.
//! Pure logic, no GPUI.

use std::{io, path::PathBuf};

use fluxwire_format::{
    EnvDiagnostic, EnvRow, Environment, diagnose_rows, load_environment, local_env_path,
    save_environment_with,
};
use fluxwire_vars::{parse_dotenv, write_dotenv_value};

use crate::editor::EditorState;

const UNDO_LIMIT: usize = 200;

/// Which cell of a row an edit touches; consecutive edits of one cell are one undo step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    Name,
    Value,
}

/// Bulk-edit buffers: the `.env` text and the `.local.env` text, each in the code editor.
#[derive(Clone, Debug, Default)]
pub struct Bulk {
    pub public: String,
    pub local: String,
    pub public_ed: EditorState,
    pub local_ed: EditorState,
}

#[derive(Clone, Debug)]
pub struct EnvDoc {
    pub root: PathBuf,
    pub name: String,
    pub env: Environment,
    /// Stable id per row (parallel to `env.rows`) so inputs survive reordering.
    pub ids: Vec<u64>,
    next_id: u64,
    disk: Vec<(String, String, bool)>,
    undo: Vec<Vec<EnvRow>>,
    redo: Vec<Vec<EnvRow>>,
    last_cell: Option<(u64, Cell)>,
    /// The text a cell had when its current edit run began (Escape reverts to it).
    run_start: Option<(u64, Cell, String)>,
    /// One of the files changed on disk while this document had unsaved edits.
    pub changed_on_disk: bool,
    pub bulk: Option<Bulk>,
}

fn key_of(rows: &[EnvRow]) -> Vec<(String, String, bool)> {
    rows.iter()
        .map(|r| (r.name.clone(), r.value.clone(), r.local))
        .collect()
}

impl EnvDoc {
    pub fn load(root: PathBuf, name: &str, secret_names: &[String]) -> Self {
        let env = load_environment(&root, name, secret_names);
        let n = env.rows.len() as u64;
        Self {
            disk: key_of(&env.rows),
            ids: (0..n).collect(),
            next_id: n,
            env,
            root,
            name: name.to_string(),
            undo: Vec::new(),
            redo: Vec::new(),
            last_cell: None,
            run_start: None,
            changed_on_disk: false,
            bulk: None,
        }
    }

    /// Replace the rows with what is on disk and forget history (Reload, or a clean tab whose files
    /// changed).
    pub fn reload(&mut self, secret_names: &[String]) {
        let fresh = Self::load(self.root.clone(), &self.name, secret_names);
        let bulk = self.bulk.is_some();
        *self = fresh;
        if bulk {
            self.enter_bulk();
        }
    }

    pub fn rows(&self) -> &[EnvRow] {
        &self.env.rows
    }

    pub fn is_dirty(&self) -> bool {
        key_of(&self.env.rows) != self.disk
            || self.bulk.as_ref().is_some_and(|b| {
                b.public != self.dotenv_text(false) || b.local != self.dotenv_text(true)
            })
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn diagnostics(&self, secret_names: &[String]) -> Vec<EnvDiagnostic> {
        diagnose_rows(&self.env, secret_names)
    }

    fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id - 1
    }

    fn checkpoint(&mut self) {
        self.undo.push(self.env.rows.clone());
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Row index of the stable `id`.
    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.ids.iter().position(|i| *i == id)
    }

    /// Set a cell. A run of edits to the same cell is one undo step.
    pub fn set_cell(&mut self, i: usize, cell: Cell, value: &str) {
        let Some(id) = self.ids.get(i).copied() else {
            return;
        };
        let r = &self.env.rows[i];
        let cur = match cell {
            Cell::Name => r.name.clone(),
            Cell::Value => r.value.clone(),
        };
        if cur == *value {
            return;
        }
        if self.last_cell != Some((id, cell)) {
            self.checkpoint();
            self.run_start = Some((id, cell, cur));
        }
        self.last_cell = Some((id, cell));
        let r = &mut self.env.rows[i];
        match cell {
            Cell::Name => r.name = value.to_string(),
            Cell::Value => r.value = value.to_string(),
        }
    }

    /// Escape in a cell: the text it had when typing began, if it was edited since.
    pub fn revert_cell(&mut self, i: usize, cell: Cell) -> Option<String> {
        let id = *self.ids.get(i)?;
        let (rid, rcell, text) = self.run_start.clone()?;
        if rid != id || rcell != cell || self.last_cell != Some((id, cell)) {
            return None;
        }
        self.undo();
        Some(text)
    }

    /// True when the files on disk no longer match what this document last read or wrote.
    pub fn disk_differs(&self, secret_names: &[String]) -> bool {
        let fresh = load_environment(&self.root, &self.name, secret_names);
        key_of(&fresh.rows) != self.disk
    }

    /// Ends the current cell's undo run (focus moved).
    pub fn end_cell_run(&mut self) {
        self.last_cell = None;
    }

    fn blank(local: bool) -> EnvRow {
        EnvRow {
            name: String::new(),
            value: String::new(),
            secret: local,
            local,
            line: 0,
        }
    }

    /// Append an empty row; returns its index.
    pub fn add_row(&mut self) -> usize {
        self.add_row_with(String::new(), String::new(), false)
    }

    /// Append an empty row that is part of the clean state (a fresh environment's first row).
    pub fn add_blank_clean(&mut self) -> usize {
        self.env.rows.push(Self::blank(false));
        let id = self.new_id();
        self.ids.push(id);
        self.disk = key_of(&self.env.rows);
        self.env.rows.len() - 1
    }

    pub fn add_row_with(&mut self, name: String, value: String, local: bool) -> usize {
        self.checkpoint();
        self.last_cell = None;
        let mut r = Self::blank(local);
        r.name = name;
        r.value = value;
        self.env.rows.push(r);
        let id = self.new_id();
        self.ids.push(id);
        self.env.rows.len() - 1
    }

    pub fn delete_row(&mut self, i: usize) {
        if i >= self.env.rows.len() {
            return;
        }
        self.checkpoint();
        self.last_cell = None;
        self.env.rows.remove(i);
        self.ids.remove(i);
    }

    pub fn duplicate_row(&mut self, i: usize) -> Option<usize> {
        let r = self.env.rows.get(i)?.clone();
        self.checkpoint();
        self.last_cell = None;
        let mut copy = r;
        copy.line = 0;
        copy.name = format!("{}_copy", copy.name);
        self.env.rows.insert(i + 1, copy);
        let id = self.new_id();
        self.ids.insert(i + 1, id);
        Some(i + 1)
    }

    /// Whether the row's secret flag can be flipped (not when the collection marks it secret).
    pub fn can_toggle_secret(&self, i: usize, secret_names: &[String]) -> bool {
        self.env
            .rows
            .get(i)
            .is_some_and(|r| !secret_names.contains(&r.name))
    }

    /// Flip Secret: the row moves to `.local.env` (on) or `.env` (off) on save.
    pub fn toggle_secret(&mut self, i: usize, secret_names: &[String]) {
        if !self.can_toggle_secret(i, secret_names) {
            return;
        }
        self.checkpoint();
        self.last_cell = None;
        let r = &mut self.env.rows[i];
        r.secret = !r.secret;
        r.local = r.secret;
    }

    /// Move a row to the other file without changing its secret flag.
    pub fn move_to_file(&mut self, i: usize, local: bool) {
        if self.env.rows.get(i).is_none_or(|r| r.local == local) {
            return;
        }
        self.checkpoint();
        self.last_cell = None;
        self.env.rows[i].local = local;
    }

    /// Reorder: row `from` is placed at index `to`.
    pub fn move_row(&mut self, from: usize, to: usize) {
        let n = self.env.rows.len();
        if from >= n || to >= n || from == to {
            return;
        }
        self.checkpoint();
        self.last_cell = None;
        let r = self.env.rows.remove(from);
        let id = self.ids.remove(from);
        self.env.rows.insert(to, r);
        self.ids.insert(to, id);
    }

    fn restore(&mut self, rows: Vec<EnvRow>) {
        self.env.rows = rows;
        // Inputs are rebuilt from the restored rows, so every row gets a new id.
        let n = self.env.rows.len();
        self.ids = (0..n).map(|_| self.new_id()).collect();
        self.last_cell = None;
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.env.rows.clone());
        self.restore(prev);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.env.rows.clone());
        self.restore(next);
        true
    }

    /// Write both files. Returns how many rows were skipped (invalid names) and resets the dirty
    /// state to what is now on disk.
    pub fn save(&mut self, secret_names: &[String]) -> io::Result<usize> {
        if self.bulk.is_some() {
            self.leave_bulk();
        }
        let skipped = save_environment_with(&self.root, &self.env, secret_names)?;
        // Rows the files cannot hold (invalid names) stay in the tab; the rest now match disk.
        self.disk = key_of(&self.env.rows);
        self.last_cell = None;
        self.changed_on_disk = false;
        Ok(skipped)
    }

    pub fn local_path(&self) -> PathBuf {
        local_env_path(&self.root, &self.name)
    }

    // ---- bulk edit ------------------------------------------------------------------------------

    fn dotenv_text(&self, local: bool) -> String {
        self.env
            .rows
            .iter()
            .filter(|r| r.local == local && !r.name.trim().is_empty())
            .map(|r| format!("{}={}\n", r.name.trim(), write_dotenv_value(&r.value)))
            .collect()
    }

    pub fn enter_bulk(&mut self) {
        self.bulk = Some(Bulk {
            public: self.dotenv_text(false),
            local: self.dotenv_text(true),
            ..Bulk::default()
        });
    }

    /// Parse the bulk buffers back into rows (one undo step). Rows that are not assignments are
    /// dropped; the caller shows parse problems before calling this if it wants to.
    pub fn leave_bulk(&mut self) {
        let Some(b) = self.bulk.take() else { return };
        let mut rows: Vec<EnvRow> = Vec::new();
        for (text, local) in [(&b.public, false), (&b.local, true)] {
            for e in parse_dotenv(text) {
                rows.push(EnvRow {
                    secret: local,
                    local,
                    name: e.name,
                    value: e.value,
                    line: 0,
                });
            }
        }
        // Collection-secret rows keep their flag when the name is unchanged.
        for r in &mut rows {
            if let Some(old) = self
                .env
                .rows
                .iter()
                .find(|o| o.name == r.name && o.local == r.local)
            {
                r.secret = old.secret;
                r.line = old.line;
            }
        }
        if key_of(&rows) != key_of(&self.env.rows) {
            self.checkpoint();
            self.env.rows = rows;
            self.ids = (0..self.env.rows.len()).map(|_| self.new_id()).collect();
        }
        self.last_cell = None;
    }

    /// Lines of a bulk buffer that are neither blank, a comment nor `NAME=value`.
    pub fn bulk_problems(text: &str) -> Vec<(usize, String)> {
        text.lines()
            .enumerate()
            .filter_map(|(i, l)| {
                let t = l.trim();
                if t.is_empty() || t.starts_with('#') {
                    return None;
                }
                let body = t.strip_prefix("export ").unwrap_or(t);
                match body.split_once('=') {
                    Some((n, _)) if fluxwire_format::is_valid_var_name(n.trim()) => None,
                    Some((n, _)) => Some((
                        i + 1,
                        format!("`{}` is not a valid variable name.", n.trim()),
                    )),
                    None => Some((i + 1, "Expected NAME=value.".into())),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluxwire_format::env_path;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fw-envdoc-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("environments")).unwrap();
        d
    }

    fn doc(tag: &str, public: &str, local: &str) -> EnvDoc {
        let d = scratch(tag);
        std::fs::write(env_path(&d, "qa"), public).unwrap();
        if !local.is_empty() {
            std::fs::write(local_env_path(&d, "qa"), local).unwrap();
        }
        EnvDoc::load(d, "qa", &[])
    }

    #[test]
    fn edits_are_dirty_and_do_not_touch_disk_until_save() {
        let mut d = doc("dirty", "# c\nA=1\n", "");
        assert!(!d.is_dirty());
        d.set_cell(0, Cell::Value, "2");
        assert!(d.is_dirty());
        assert_eq!(
            std::fs::read_to_string(env_path(&d.root, "qa")).unwrap(),
            "# c\nA=1\n"
        );
        assert_eq!(d.save(&[]).unwrap(), 0);
        assert!(!d.is_dirty());
        assert_eq!(
            std::fs::read_to_string(env_path(&d.root, "qa")).unwrap(),
            "# c\nA=2\n"
        );
    }

    #[test]
    fn secret_toggle_moves_the_row_to_the_local_file_on_save() {
        let mut d = doc("secret", "url=u\npw=p\n", "");
        d.toggle_secret(1, &[]);
        d.save(&[]).unwrap();
        assert_eq!(
            std::fs::read_to_string(env_path(&d.root, "qa")).unwrap(),
            "url=u\n"
        );
        assert_eq!(
            std::fs::read_to_string(local_env_path(&d.root, "qa")).unwrap(),
            "pw=p\n"
        );
        // Toggling back removes it from the local file.
        d.toggle_secret(1, &[]);
        d.save(&[]).unwrap();
        assert_eq!(
            std::fs::read_to_string(env_path(&d.root, "qa")).unwrap(),
            "url=u\npw=p\n"
        );
    }

    #[test]
    fn collection_secrets_cannot_be_toggled() {
        let mut d = doc("coll", "k=v\n", "");
        let names = vec!["k".to_string()];
        assert!(!d.can_toggle_secret(0, &names));
        d.toggle_secret(0, &names);
        assert!(!d.is_dirty());
    }

    #[test]
    fn typing_runs_are_one_undo_step_and_structure_changes_each_one() {
        let mut d = doc("undo", "A=1\n", "");
        for v in ["2", "23", "234"] {
            d.set_cell(0, Cell::Value, v);
        }
        d.add_row();
        assert!(d.undo());
        assert_eq!(d.rows().len(), 1);
        assert!(d.undo());
        assert_eq!(d.rows()[0].value, "1");
        assert!(!d.is_dirty());
        assert!(d.redo());
        assert_eq!(d.rows()[0].value, "234");
    }

    #[test]
    fn reorder_delete_duplicate_and_ids() {
        let mut d = doc("reorder", "A=1\nB=2\nC=3\n", "");
        let ids = d.ids.clone();
        d.move_row(2, 0);
        assert_eq!(
            d.rows().iter().map(|r| &*r.name).collect::<Vec<_>>(),
            ["C", "A", "B"]
        );
        assert_eq!(d.ids, vec![ids[2], ids[0], ids[1]]);
        d.delete_row(1);
        assert_eq!(d.rows().len(), 2);
        assert_eq!(d.duplicate_row(0), Some(1));
        assert_eq!(d.rows()[1].name, "C_copy");
        d.save(&[]).unwrap();
        assert_eq!(
            std::fs::read_to_string(env_path(&d.root, "qa")).unwrap(),
            "C=3\nB=2\nC_copy=3\n"
        );
    }

    #[test]
    fn invalid_rows_are_skipped_on_save_and_stay_in_the_tab() {
        let mut d = doc("invalid", "A=1\n", "");
        let i = d.add_row_with("1bad".into(), "x".into(), false);
        assert_eq!(i, 1);
        assert_eq!(d.save(&[]).unwrap(), 1);
        assert_eq!(
            std::fs::read_to_string(env_path(&d.root, "qa")).unwrap(),
            "A=1\n"
        );
        assert_eq!(d.rows().len(), 2, "the invalid row is still shown");
        assert_eq!(d.diagnostics(&[])[0].code, "E101");
    }

    #[test]
    fn bulk_edit_round_trips_and_is_one_undo_step() {
        let mut d = doc("bulk", "A=1\n", "pw=s\n");
        d.enter_bulk();
        let b = d.bulk.as_mut().unwrap();
        assert_eq!((b.public.as_str(), b.local.as_str()), ("A=1\n", "pw=s\n"));
        b.public.push_str("B=\"two words\"\n");
        d.leave_bulk();
        assert_eq!(d.rows().len(), 3);
        assert_eq!(d.rows()[1].value, "two words");
        assert!(d.rows()[2].local);
        assert!(d.undo());
        assert_eq!(d.rows().len(), 2);
        assert_eq!(EnvDoc::bulk_problems("A=1\nnope\n9x=1\n# c\n").len(), 2);
    }

    #[test]
    fn reload_discards_edits() {
        let mut d = doc("reload", "A=1\n", "");
        d.set_cell(0, Cell::Value, "9");
        std::fs::write(env_path(&d.root, "qa"), "A=5\n").unwrap();
        d.reload(&[]);
        assert_eq!(d.rows()[0].value, "5");
        assert!(!d.is_dirty() && !d.can_undo());
    }
}
