use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, params, params_from_iter, types::Value as Sql};
use sha2::{Digest, Sha256};

use crate::model::{Entry, Filter, NewEntry, Scope, Summary};
use crate::redact::{Redactor, cap_bodies};
use crate::settings::HistorySettings;

#[derive(Debug)]
pub struct StoreError(pub String);

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "history store: {}", self.0)
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError(e.to_string())
    }
}

type Result<T> = std::result::Result<T, StoreError>;

/// Ordered migrations; index + 1 is the schema version.
const MIGRATIONS: &[&str] = &[
    "
CREATE TABLE entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace TEXT NOT NULL,
    ts_ms INTEGER NOT NULL,
    collection TEXT NOT NULL,
    request_path TEXT NOT NULL,
    request_name TEXT NOT NULL,
    protocol TEXT NOT NULL,
    method TEXT NOT NULL,
    url TEXT NOT NULL,
    status INTEGER,
    total_ms REAL NOT NULL,
    env TEXT,
    dirty INTEGER NOT NULL,
    run_id TEXT,
    summary TEXT NOT NULL,
    truncated INTEGER NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX entries_ws_ts ON entries (workspace, ts_ms DESC);
CREATE INDEX entries_ws_req ON entries (workspace, request_path, id DESC);
",
    "
CREATE INDEX entries_ws_run ON entries (workspace, run_id);
",
];

/// The user data dir (`FLUXWIRE_DATA_DIR` overrides it, for tests and demos).
pub fn data_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("FLUXWIRE_DATA_DIR") {
        return Some(PathBuf::from(d));
    }
    directories::ProjectDirs::from("", "", "fluxwire").map(|d| d.data_dir().to_path_buf())
}

/// Stable workspace key: sha256 of the root path.
pub fn workspace_key(root: &Path) -> String {
    let p = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let h = Sha256::digest(p.to_string_lossy().as_bytes());
    h.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// Cheap to clone; every call locks one connection. Call from a blocking thread, not the UI.
#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| StoreError(e.to_string()))?;
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);",
        )?;
        let cur: i64 = conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |r| r.get(0),
        )?;
        if cur as usize > MIGRATIONS.len() {
            return Err(StoreError(format!(
                "database schema {cur} is newer than this version supports"
            )));
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(cur as usize) {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                [i as i64 + 1],
            )?;
            tx.commit()?;
        }
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .lock()
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))?)
    }

    pub fn journal_mode(&self) -> Result<String> {
        Ok(self
            .lock()
            .pragma_query_value(None, "journal_mode", |r| r.get(0))?)
    }

    /// Redact, cap, and write one entry. Returns its id.
    pub fn insert(&self, workspace: &str, mut e: NewEntry, red: &Redactor) -> Result<i64> {
        red.redact_value(&mut e.payload);
        let truncated = cap_bodies(&mut e.payload);
        let payload = serde_json::to_string(&e.payload).map_err(|x| StoreError(x.to_string()))?;
        let conn = self.lock();
        conn.execute(
            "INSERT INTO entries (workspace, ts_ms, collection, request_path, request_name,
                protocol, method, url, status, total_ms, env, dirty, run_id, summary, truncated, payload)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
            params![
                workspace,
                e.ts_ms,
                e.collection,
                e.request_path,
                red.redact_str(&e.request_name),
                e.protocol,
                e.method,
                red.redact_str(&e.url),
                e.status,
                e.total_ms,
                e.env,
                e.dirty,
                e.run_id,
                red.redact_str(&e.summary),
                truncated,
                payload
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list(&self, workspace: &str, f: &Filter) -> Result<Vec<Summary>> {
        let mut sql = String::from(
            "SELECT id, ts_ms, collection, request_path, request_name, protocol, method, url,
                    status, total_ms, env, dirty, summary, run_id FROM entries WHERE workspace = ?",
        );
        let mut args: Vec<Sql> = vec![workspace.to_string().into()];
        let like = |s: &str| {
            format!(
                "%{}%",
                s.replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            )
        };
        if let Some(m) = &f.method {
            sql += " AND method = ? COLLATE NOCASE";
            args.push(m.clone().into());
        }
        match f.status_class {
            Some(0) => sql += " AND status IS NULL",
            Some(c) => {
                sql += " AND status >= ? AND status < ?";
                args.push((i64::from(c) * 100).into());
                args.push((i64::from(c) * 100 + 100).into());
            }
            None => {}
        }
        if let Some(n) = f.name.as_ref().filter(|s| !s.is_empty()) {
            sql += " AND request_name LIKE ? ESCAPE '\\'";
            args.push(like(n).into());
        }
        if let Some(x) = f.text.as_ref().filter(|s| !s.is_empty()) {
            sql += " AND (request_name LIKE ? ESCAPE '\\' OR url LIKE ? ESCAPE '\\')";
            args.push(like(x).into());
            args.push(like(x).into());
        }
        if let Some(u) = f.url_text.as_ref().filter(|s| !s.is_empty()) {
            sql += " AND url LIKE ? ESCAPE '\\'";
            args.push(like(u).into());
        }
        if let Some(p) = &f.request_path {
            sql += " AND request_path = ?";
            args.push(p.clone().into());
        }
        if let Some(r) = &f.run_id {
            sql += " AND run_id = ?";
            args.push(r.clone().into());
        }
        if let Some(b) = f.before_id {
            sql += " AND id < ?";
            args.push(b.into());
        }
        sql += " ORDER BY id DESC LIMIT ?";
        args.push((f.limit.unwrap_or(200) as i64).into());
        let conn = self.lock();
        let mut st = conn.prepare(&sql)?;
        let rows = st.query_map(params_from_iter(args), row_summary)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn get(&self, id: i64) -> Result<Option<Entry>> {
        let conn = self.lock();
        let mut st = conn.prepare(
            "SELECT id, ts_ms, collection, request_path, request_name, protocol, method, url,
                    status, total_ms, env, dirty, summary, truncated, payload, run_id
             FROM entries WHERE id = ?",
        )?;
        let mut rows = st.query_map([id], |r| {
            let mut summary = row_summary(r)?;
            summary.run_id = r.get(15)?;
            let payload: String = r.get(14)?;
            Ok(Entry {
                summary,
                truncated: r.get(13)?,
                payload: serde_json::from_str(&payload).unwrap_or_default(),
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    pub fn count(&self, workspace: &str) -> Result<i64> {
        Ok(self.lock().query_row(
            "SELECT COUNT(*) FROM entries WHERE workspace = ?",
            [workspace],
            |r| r.get(0),
        )?)
    }

    /// Delete history for a scope in one workspace; returns rows removed.
    pub fn clear(&self, workspace: &str, scope: &Scope) -> Result<usize> {
        let conn = self.lock();
        Ok(match scope {
            Scope::All => conn.execute("DELETE FROM entries WHERE workspace = ?", [workspace])?,
            Scope::Collection(c) => conn.execute(
                "DELETE FROM entries WHERE workspace = ? AND collection = ?",
                params![workspace, c],
            )?,
            Scope::Request { collection, path } => conn.execute(
                "DELETE FROM entries WHERE workspace = ? AND collection = ? AND request_path = ?",
                params![workspace, collection, path],
            )?,
        })
    }

    /// Keep at most `max_entries` per workspace and nothing older than `max_age_days`.
    pub fn enforce_retention(&self, s: &HistorySettings, now_ms: i64) -> Result<usize> {
        let cutoff = now_ms - (s.max_age_days as i64) * 86_400_000;
        let conn = self.lock();
        let mut n = conn.execute("DELETE FROM entries WHERE ts_ms < ?", [cutoff])?;
        n += conn.execute(
            "DELETE FROM entries WHERE id IN (
                SELECT id FROM (
                    SELECT id, ROW_NUMBER() OVER (PARTITION BY workspace ORDER BY id DESC) AS rn
                    FROM entries) WHERE rn > ?)",
            [s.max_entries as i64],
        )?;
        Ok(n)
    }

    /// Flush the WAL into the main file (also lets tests scan one file).
    pub fn checkpoint(&self) -> Result<()> {
        self.lock()
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        Ok(())
    }
}

fn row_summary(r: &rusqlite::Row<'_>) -> rusqlite::Result<Summary> {
    Ok(Summary {
        id: r.get(0)?,
        ts_ms: r.get(1)?,
        collection: r.get(2)?,
        request_path: r.get(3)?,
        request_name: r.get(4)?,
        protocol: r.get(5)?,
        method: r.get(6)?,
        url: r.get(7)?,
        status: r.get(8)?,
        total_ms: r.get(9)?,
        env: r.get(10)?,
        dirty: r.get(11)?,
        summary: r.get(12)?,
        run_id: r.get(13).unwrap_or(None),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(path: &str, method: &str, status: Option<i64>, ts: i64) -> NewEntry {
        NewEntry {
            ts_ms: ts,
            collection: "c".into(),
            request_path: path.into(),
            request_name: path.into(),
            protocol: "http".into(),
            method: method.into(),
            url: format!("https://api/{path}"),
            status,
            total_ms: 5.0,
            env: Some("staging".into()),
            dirty: false,
            run_id: None,
            summary: "s".into(),
            payload: json!({"request": {"headers": []}}),
        }
    }

    #[test]
    fn opens_wal_migrates_and_is_idempotent() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("h.sqlite");
        let s = Store::open(&p).unwrap();
        assert_eq!(s.journal_mode().unwrap(), "wal");
        assert_eq!(s.schema_version().unwrap(), MIGRATIONS.len() as i64);
        drop(s);
        let s = Store::open(&p).unwrap();
        assert_eq!(s.schema_version().unwrap(), MIGRATIONS.len() as i64);
    }

    #[test]
    fn run_id_is_stored_listed_and_filterable() {
        let s = Store::open_in_memory().unwrap();
        let r = Redactor::default();
        let mut e = entry("a", "GET", Some(200), 1);
        e.run_id = Some("run1".into());
        s.insert("w", e, &r).unwrap();
        s.insert("w", entry("b", "GET", Some(200), 2), &r).unwrap();
        let f = Filter {
            run_id: Some("run1".into()),
            ..Default::default()
        };
        let rows = s.list("w", &f).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].run_id.as_deref(), Some("run1"));
        assert_eq!(
            s.get(rows[0].id)
                .unwrap()
                .unwrap()
                .summary
                .run_id
                .as_deref(),
            Some("run1")
        );
    }

    #[test]
    fn insert_list_filter_page() {
        let s = Store::open_in_memory().unwrap();
        let r = Redactor::default();
        for i in 0..5 {
            s.insert("w", entry("a", "GET", Some(200), i), &r).unwrap();
        }
        s.insert("w", entry("b", "POST", Some(404), 9), &r).unwrap();
        s.insert("w", entry("e", "GET", None, 10), &r).unwrap();
        s.insert("other", entry("a", "GET", Some(200), 11), &r)
            .unwrap();
        let all = s.list("w", &Filter::default()).unwrap();
        assert_eq!(all.len(), 7);
        assert!(all[0].id > all[1].id);
        let q = |f: Filter| s.list("w", &f).unwrap().len();
        assert_eq!(
            q(Filter {
                method: Some("post".into()),
                ..Default::default()
            }),
            1
        );
        assert_eq!(
            q(Filter {
                status_class: Some(2),
                ..Default::default()
            }),
            5
        );
        assert_eq!(
            q(Filter {
                status_class: Some(4),
                ..Default::default()
            }),
            1
        );
        assert_eq!(
            q(Filter {
                status_class: Some(0),
                ..Default::default()
            }),
            1
        );
        assert_eq!(
            q(Filter {
                url_text: Some("api/b".into()),
                ..Default::default()
            }),
            1
        );
        assert_eq!(
            q(Filter {
                name: Some("a".into()),
                ..Default::default()
            }),
            5
        );
        let page = s
            .list(
                "w",
                &Filter {
                    limit: Some(3),
                    ..Default::default()
                },
            )
            .unwrap();
        let next = s
            .list(
                "w",
                &Filter {
                    before_id: Some(page[2].id),
                    limit: Some(10),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!((page.len(), next.len()), (3, 4));
        let e = s.get(all[0].id).unwrap().unwrap();
        assert_eq!(e.summary.method, "GET");
        assert!(!e.truncated);
    }

    #[test]
    fn retention_by_count_and_age_and_clear() {
        let s = Store::open_in_memory().unwrap();
        let r = Redactor::default();
        let day = 86_400_000;
        let now = 100 * day;
        s.insert("w", entry("old", "GET", Some(200), now - 40 * day), &r)
            .unwrap();
        for i in 0..5 {
            s.insert("w", entry("a", "GET", Some(200), now - i), &r)
                .unwrap();
        }
        let cfg = HistorySettings {
            max_entries: 3,
            max_age_days: 30,
        };
        let n = s.enforce_retention(&cfg, now).unwrap();
        assert_eq!((n, s.count("w").unwrap()), (3, 3));
        s.insert("w", entry("b", "GET", Some(200), now), &r)
            .unwrap();
        let rm = s
            .clear(
                "w",
                &Scope::Request {
                    collection: "c".into(),
                    path: "b".into(),
                },
            )
            .unwrap();
        assert_eq!(rm, 1);
        assert_eq!(s.clear("w", &Scope::Collection("c".into())).unwrap(), 3);
        assert_eq!(s.count("w").unwrap(), 0);
    }

    #[test]
    fn secrets_never_reach_the_database_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("h.sqlite");
        let s = Store::open(&p).unwrap();
        let r = Redactor::with_secrets(vec![("apiKey".into(), "TOP-SECRET-12345".into())]);
        let mut e = entry("a", "GET", Some(200), 1);
        e.url = "https://api/a?key=TOP-SECRET-12345".into();
        e.payload = json!({
            "request": {"headers": [["Authorization", "Bearer BEARER-TOKEN-999"]],
                        "body": {"text": "{\"k\":\"TOP-SECRET-12345\"}"}},
            "response": {"headers": [["Set-Cookie", "sid=COOKIE-VALUE-777"]]}
        });
        s.insert("w", e, &r).unwrap();
        s.checkpoint().unwrap();
        drop(s);
        let mut bytes = std::fs::read(&p).unwrap();
        if let Ok(wal) = std::fs::read(d.path().join("h.sqlite-wal")) {
            bytes.extend(wal);
        }
        for needle in ["TOP-SECRET-12345", "BEARER-TOKEN-999", "COOKIE-VALUE-777"] {
            assert!(
                !bytes.windows(needle.len()).any(|w| w == needle.as_bytes()),
                "{needle} leaked"
            );
        }
    }

    #[test]
    fn workspace_key_is_stable() {
        let a = workspace_key(Path::new("/tmp/does-not-exist-x"));
        assert_eq!(a, workspace_key(Path::new("/tmp/does-not-exist-x")));
        assert_ne!(a, workspace_key(Path::new("/tmp/does-not-exist-y")));
    }
}
