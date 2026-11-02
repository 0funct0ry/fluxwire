use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, params, params_from_iter, types::Value as Sql};
use sha2::{Digest, Sha256};

use crate::export;
use crate::model::{
    Counts, Entry, ExportFormat, Filter, NewEntry, SLOW_MS, Scope, SortCol, Stats, Summary,
};
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
    "
ALTER TABLE entries ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
ALTER TABLE entries ADD COLUMN note TEXT;
",
    "
ALTER TABLE entries ADD COLUMN origin TEXT;
",
];

/// Milliseconds in a day (timeline buckets).
pub const DAY_MS: i64 = 86_400_000;

const SUMMARY_COLS: &str =
    "id, ts_ms, collection, request_path, request_name, protocol, method, url,
                    status, total_ms, env, dirty, summary, run_id, pinned, note, origin";

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
                protocol, method, url, status, total_ms, env, dirty, run_id, summary, truncated, payload, origin)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
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
                payload,
                e.origin
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list(&self, workspace: &str, f: &Filter) -> Result<Vec<Summary>> {
        let (mut sql, mut args) = build_where(workspace, f, true);
        let dir = if f.sort.asc { "ASC" } else { "DESC" };
        let col = match f.sort.col {
            SortCol::Time => "id",
            SortCol::Method => "method COLLATE NOCASE",
            SortCol::Name => "request_name COLLATE NOCASE",
            SortCol::Url => "url",
            SortCol::Status => "COALESCE(status, 0)",
            SortCol::Duration => "total_ms",
            SortCol::Env => "COALESCE(env, '') COLLATE NOCASE",
        };
        if f.sort.col == SortCol::Time {
            sql += &format!(" ORDER BY id {dir} LIMIT ?");
        } else {
            sql += &format!(" ORDER BY {col} {dir}, id DESC LIMIT ?");
        }
        args.push((f.limit.unwrap_or(200) as i64).into());
        sql += " OFFSET ?";
        args.push((f.offset.unwrap_or(0) as i64).into());
        let conn = self.lock();
        let mut st = conn.prepare(&sql)?;
        let rows = st.query_map(params_from_iter(args), row_summary)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Count, success rate, and duration percentiles of the filtered set (paging fields ignored).
    pub fn stats(&self, workspace: &str, f: &Filter) -> Result<Stats> {
        let mut f = f.clone();
        f.before_id = None;
        let (mut sql, args) = build_where(workspace, &f, false);
        sql = sql.replacen(SUMMARY_COLS, "status, total_ms", 1);
        sql += " ORDER BY total_ms";
        let conn = self.lock();
        let mut st = conn.prepare(&sql)?;
        let rows: Vec<(Option<i64>, f64)> = st
            .query_map(params_from_iter(args), |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        if rows.is_empty() {
            return Ok(Stats::default());
        }
        let n = rows.len();
        let ok = rows
            .iter()
            .filter(|(s, _)| matches!(s, Some(200..=399)))
            .count();
        // Nearest rank: the smallest value with at least p% of the set at or below it.
        let pct = |p: f64| rows[((p * n as f64).ceil() as usize).clamp(1, n) - 1].1;
        Ok(Stats {
            count: n as i64,
            success_rate: ok as f64 / n as f64,
            p50_ms: pct(0.50),
            p95_ms: pct(0.95),
        })
    }

    /// Navigator counts. `day_start_ms` is the start of the user's current local day.
    pub fn counts(&self, workspace: &str, day_start_ms: i64) -> Result<Counts> {
        let conn = self.lock();
        let n = |cond: &str, args: &[Sql]| -> Result<i64> {
            let sql = format!("SELECT COUNT(*) FROM entries WHERE workspace = ?1 AND {cond}");
            let mut all: Vec<Sql> = vec![workspace.to_string().into()];
            all.extend_from_slice(args);
            Ok(conn.query_row(&sql, params_from_iter(all), |r| r.get(0))?)
        };
        let yday = day_start_ms - DAY_MS;
        let week = day_start_ms - 6 * DAY_MS;
        let mut c = Counts {
            today: n("ts_ms >= ?2", &[day_start_ms.into()])?,
            yesterday: n(
                "ts_ms >= ?2 AND ts_ms < ?3",
                &[yday.into(), day_start_ms.into()],
            )?,
            this_week: n("ts_ms >= ?2 AND ts_ms < ?3", &[week.into(), yday.into()])?,
            older: n("ts_ms < ?2", &[week.into()])?,
            failures: n("(status IS NULL OR status >= 400)", &[])?,
            slow: n("total_ms > ?2", &[SLOW_MS.into()])?,
            unsaved: n("dirty = 1", &[])?,
            pinned: n("pinned = 1", &[])?,
            runs: n("run_id IS NOT NULL", &[])?,
            requests: vec![],
        };
        let mut st = conn.prepare(
            "SELECT collection, request_path, MAX(request_name), COUNT(*) FROM entries
             WHERE workspace = ? GROUP BY collection, request_path ORDER BY collection, request_path",
        )?;
        c.requests = st
            .query_map([workspace], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(c)
    }

    pub fn set_pinned(&self, id: i64, pinned: bool) -> Result<()> {
        self.lock().execute(
            "UPDATE entries SET pinned = ? WHERE id = ?",
            params![pinned, id],
        )?;
        Ok(())
    }

    /// An empty note clears it. Notes are user text, redacted with the same secrets as bodies.
    pub fn set_note(&self, id: i64, note: Option<String>, red: &Redactor) -> Result<()> {
        let note = note
            .map(|n| red.redact_str(n.trim()))
            .filter(|n| !n.is_empty());
        self.lock().execute(
            "UPDATE entries SET note = ? WHERE id = ?",
            params![note, id],
        )?;
        Ok(())
    }

    /// Delete the given entries (pinned or not: this is an explicit action).
    pub fn delete(&self, ids: &[i64]) -> Result<usize> {
        let conn = self.lock();
        let mut n = 0;
        for id in ids {
            n += conn.execute("DELETE FROM entries WHERE id = ?", [id])?;
        }
        Ok(n)
    }

    /// The last earlier entry of the same request (for "Compare to previous").
    pub fn previous_of(&self, id: i64) -> Result<Option<i64>> {
        let conn = self.lock();
        let mut st = conn.prepare(
            "SELECT p.id FROM entries p JOIN entries e
             ON p.workspace = e.workspace AND p.collection = e.collection
                AND p.request_path = e.request_path
             WHERE e.id = ? AND p.id < e.id ORDER BY p.id DESC LIMIT 1",
        )?;
        let mut rows = st.query_map([id], |r| r.get(0))?;
        Ok(rows.next().transpose()?)
    }

    /// The whole filtered set (limit and paging ignored) as JSON or CSV.
    pub fn export(&self, workspace: &str, f: &Filter, format: ExportFormat) -> Result<String> {
        let mut f = f.clone();
        f.before_id = None;
        f.offset = None;
        f.limit = Some(usize::MAX >> 1);
        let rows = self.list(workspace, &f)?;
        Ok(match format {
            ExportFormat::Json => export::to_json(&rows),
            ExportFormat::Csv => export::to_csv(&rows),
        })
    }

    pub fn get(&self, id: i64) -> Result<Option<Entry>> {
        let conn = self.lock();
        let mut st = conn.prepare(&format!(
            "SELECT {SUMMARY_COLS}, truncated, payload FROM entries WHERE id = ?"
        ))?;
        let mut rows = st.query_map([id], |r| {
            let summary = row_summary(r)?;
            let payload: String = r.get(18)?;
            Ok(Entry {
                summary,
                truncated: r.get(17)?,
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

    /// Delete history for a scope in one workspace; returns rows removed. Pinned entries stay
    /// unless `include_pinned`.
    pub fn clear(&self, workspace: &str, scope: &Scope, include_pinned: bool) -> Result<usize> {
        let conn = self.lock();
        let keep = if include_pinned {
            ""
        } else {
            " AND pinned = 0"
        };
        Ok(match scope {
            Scope::All => conn.execute(
                &format!("DELETE FROM entries WHERE workspace = ?{keep}"),
                [workspace],
            )?,
            Scope::Collection(c) => conn.execute(
                &format!("DELETE FROM entries WHERE workspace = ? AND collection = ?{keep}"),
                params![workspace, c],
            )?,
            Scope::Request { collection, path } => conn.execute(
                &format!(
                    "DELETE FROM entries WHERE workspace = ? AND collection = ? AND request_path = ?{keep}"
                ),
                params![workspace, collection, path],
            )?,
        })
    }

    /// Keep at most `max_entries` per workspace and nothing older than `max_age_days`.
    /// Pinned entries are exempt from both.
    pub fn enforce_retention(&self, s: &HistorySettings, now_ms: i64) -> Result<usize> {
        let cutoff = now_ms - (s.max_age_days as i64) * DAY_MS;
        let conn = self.lock();
        let mut n = conn.execute(
            "DELETE FROM entries WHERE ts_ms < ? AND pinned = 0",
            [cutoff],
        )?;
        n += conn.execute(
            "DELETE FROM entries WHERE id IN (
                SELECT id FROM (
                    SELECT id, ROW_NUMBER() OVER (PARTITION BY workspace ORDER BY id DESC) AS rn
                    FROM entries WHERE pinned = 0) WHERE rn > ?)",
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
        run_id: r.get(13)?,
        pinned: r.get(14)?,
        note: r.get(15)?,
        origin: r.get(16)?,
    })
}

/// `SELECT … WHERE` for a filter. `paging` adds the `before_id` bound.
fn build_where(workspace: &str, f: &Filter, paging: bool) -> (String, Vec<Sql>) {
    let mut sql = format!("SELECT {SUMMARY_COLS} FROM entries WHERE workspace = ?");
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
    for (col, v) in [
        ("request_path", &f.request_path),
        ("run_id", &f.run_id),
        ("env", &f.env),
        ("protocol", &f.protocol),
        ("collection", &f.collection),
        ("origin", &f.origin),
    ] {
        if let Some(v) = v {
            sql += &format!(" AND {col} = ?");
            args.push(v.clone().into());
        }
    }
    if f.pinned {
        sql += " AND pinned = 1";
    }
    if f.dirty {
        sql += " AND dirty = 1";
    }
    if f.failed {
        sql += " AND (status IS NULL OR status >= 400)";
    }
    if f.slow {
        sql += " AND total_ms > ?";
        args.push(SLOW_MS.into());
    }
    if f.runs_only {
        sql += " AND run_id IS NOT NULL";
    }
    if let Some(t) = f.since_ms {
        sql += " AND ts_ms >= ?";
        args.push(t.into());
    }
    if let Some(t) = f.until_ms {
        sql += " AND ts_ms < ?";
        args.push(t.into());
    }
    if let (true, Some(b)) = (paging, f.before_id) {
        sql += " AND id < ?";
        args.push(b.into());
    }
    (sql, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Sort;
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
            origin: None,
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
                false,
            )
            .unwrap();
        assert_eq!(rm, 1);
        assert_eq!(
            s.clear("w", &Scope::Collection("c".into()), false).unwrap(),
            3
        );
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

    fn q(s: &Store, f: Filter) -> Vec<i64> {
        s.list("w", &f).unwrap().iter().map(|r| r.id).collect()
    }

    #[test]
    fn migrates_a_v2_database_in_place() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("h.sqlite");
        {
            let c = Connection::open(&p).unwrap();
            c.execute_batch("CREATE TABLE schema_version (version INTEGER NOT NULL);")
                .unwrap();
            for (i, sql) in MIGRATIONS.iter().take(2).enumerate() {
                c.execute_batch(sql).unwrap();
                c.execute("INSERT INTO schema_version VALUES (?)", [i as i64 + 1])
                    .unwrap();
            }
            c.execute(
                "INSERT INTO entries (workspace, ts_ms, collection, request_path, request_name,
                 protocol, method, url, status, total_ms, env, dirty, summary, truncated, payload)
                 VALUES ('w',1,'c','a','a','http','GET','u',200,1.0,NULL,0,'s',0,'{}')",
                [],
            )
            .unwrap();
        }
        let s = Store::open(&p).unwrap();
        assert_eq!(s.schema_version().unwrap(), MIGRATIONS.len() as i64);
        let rows = s.list("w", &Filter::default()).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].pinned && rows[0].note.is_none() && rows[0].origin.is_none());
    }

    #[test]
    fn origin_is_stored_and_filterable() {
        let s = Store::open_in_memory().unwrap();
        let r = Redactor::default();
        let mut a = entry("a", "GET", Some(200), 1);
        a.origin = Some("agent".into());
        let a = s.insert("w", a, &r).unwrap();
        s.insert("w", entry("b", "GET", Some(200), 2), &r).unwrap();
        let rows = s
            .list(
                "w",
                &Filter {
                    origin: Some("agent".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![a]);
        assert_eq!(rows[0].origin.as_deref(), Some("agent"));
    }

    #[test]
    fn new_filters_and_sort() {
        let s = Store::open_in_memory().unwrap();
        let r = Redactor::default();
        let mut a = entry("a", "GET", Some(200), 100);
        a.total_ms = 2000.0;
        a.dirty = true;
        let mut b = entry("b", "POST", Some(500), 200);
        b.env = Some("prod".into());
        b.protocol = "ws".into();
        b.collection = "other".into();
        b.run_id = Some("r1".into());
        let c = entry("c", "GET", None, 300);
        let ids: Vec<i64> = [a, b, c]
            .into_iter()
            .map(|e| s.insert("w", e, &r).unwrap())
            .collect();
        let f = |f: Filter| q(&s, f);
        assert_eq!(
            f(Filter {
                env: Some("prod".into()),
                ..Default::default()
            }),
            vec![ids[1]]
        );
        assert_eq!(
            f(Filter {
                protocol: Some("ws".into()),
                ..Default::default()
            }),
            vec![ids[1]]
        );
        assert_eq!(
            f(Filter {
                collection: Some("other".into()),
                ..Default::default()
            }),
            vec![ids[1]]
        );
        assert_eq!(
            f(Filter {
                failed: true,
                ..Default::default()
            }),
            vec![ids[2], ids[1]]
        );
        assert_eq!(
            f(Filter {
                slow: true,
                ..Default::default()
            }),
            vec![ids[0]]
        );
        assert_eq!(
            f(Filter {
                dirty: true,
                ..Default::default()
            }),
            vec![ids[0]]
        );
        assert_eq!(
            f(Filter {
                runs_only: true,
                ..Default::default()
            }),
            vec![ids[1]]
        );
        assert_eq!(
            f(Filter {
                since_ms: Some(150),
                until_ms: Some(300),
                ..Default::default()
            }),
            vec![ids[1]]
        );
        let sort = |col, asc| Filter {
            sort: Sort { col, asc },
            ..Default::default()
        };
        assert_eq!(f(sort(SortCol::Time, true)), vec![ids[0], ids[1], ids[2]]);
        assert_eq!(f(sort(SortCol::Duration, false))[0], ids[0]);
        assert_eq!(f(sort(SortCol::Method, true))[2], ids[1]);
        assert_eq!(f(sort(SortCol::Status, false))[0], ids[1]);
        let page = Filter {
            sort: Sort {
                col: SortCol::Method,
                asc: true,
            },
            limit: Some(2),
            offset: Some(2),
            ..Default::default()
        };
        assert_eq!(f(page), vec![ids[1]]);
    }

    #[test]
    fn stats_match_a_hand_computed_fixture() {
        let s = Store::open_in_memory().unwrap();
        let r = Redactor::default();
        // durations 10..=100, statuses: 8 success (2xx/3xx), one 404, one failed send.
        for i in 1..=10 {
            let status = match i {
                9 => Some(404),
                10 => None,
                8 => Some(302),
                _ => Some(200),
            };
            let mut e = entry("a", "GET", status, i);
            e.total_ms = (i * 10) as f64;
            s.insert("w", e, &r).unwrap();
        }
        let st = s.stats("w", &Filter::default()).unwrap();
        assert_eq!(st.count, 10);
        assert!((st.success_rate - 0.8).abs() < 1e-9);
        assert_eq!(st.p50_ms, 50.0); // rank ceil(0.5*10) = 5
        assert_eq!(st.p95_ms, 100.0); // rank ceil(9.5) = 10
        let none = s
            .stats(
                "w",
                &Filter {
                    env: Some("x".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(none, Stats::default());
    }

    #[test]
    fn pin_note_delete_and_previous() {
        let s = Store::open_in_memory().unwrap();
        let r = Redactor::with_secrets(vec![("k".into(), "SEKRET".into())]);
        let a1 = s.insert("w", entry("a", "GET", Some(200), 1), &r).unwrap();
        let b1 = s.insert("w", entry("b", "GET", Some(200), 2), &r).unwrap();
        let a2 = s.insert("w", entry("a", "GET", Some(200), 3), &r).unwrap();
        assert_eq!(s.previous_of(a2).unwrap(), Some(a1));
        assert_eq!(s.previous_of(a1).unwrap(), None);
        s.set_pinned(a1, true).unwrap();
        s.set_note(a1, Some("  baseline SEKRET ".into()), &r)
            .unwrap();
        let e = s.get(a1).unwrap().unwrap().summary;
        assert!(e.pinned);
        assert_eq!(e.note.as_deref(), Some("baseline ‹secret:k›"));
        s.set_note(a1, Some("  ".into()), &r).unwrap();
        assert_eq!(s.get(a1).unwrap().unwrap().summary.note, None);
        assert_eq!(s.delete(&[b1, 999]).unwrap(), 1);
        assert_eq!(s.count("w").unwrap(), 2);
    }

    #[test]
    fn pinned_survive_retention_and_clear() {
        let s = Store::open_in_memory().unwrap();
        let r = Redactor::default();
        let now = 100 * DAY_MS;
        let old = s
            .insert("w", entry("old", "GET", Some(200), now - 50 * DAY_MS), &r)
            .unwrap();
        s.set_pinned(old, true).unwrap();
        for i in 0..4 {
            s.insert("w", entry("a", "GET", Some(200), now - i), &r)
                .unwrap();
        }
        let cfg = HistorySettings {
            max_entries: 2,
            max_age_days: 30,
        };
        s.enforce_retention(&cfg, now).unwrap();
        assert_eq!(s.count("w").unwrap(), 3); // pinned + 2 newest
        assert_eq!(s.clear("w", &Scope::All, false).unwrap(), 2);
        assert_eq!(s.count("w").unwrap(), 1);
        assert_eq!(s.clear("w", &Scope::All, true).unwrap(), 1);
    }

    #[test]
    fn counts_buckets_views_and_requests() {
        let s = Store::open_in_memory().unwrap();
        let r = Redactor::default();
        let day = 10 * DAY_MS;
        let mut slow = entry("a", "GET", Some(200), day + 5);
        slow.total_ms = 5000.0;
        s.insert("w", slow, &r).unwrap();
        s.insert("w", entry("b", "GET", Some(500), day - 1), &r)
            .unwrap();
        let mut run = entry("a", "GET", None, day - 3 * DAY_MS);
        run.run_id = Some("r".into());
        run.dirty = true;
        s.insert("w", run, &r).unwrap();
        let id = s
            .insert("w", entry("a", "GET", Some(200), day - 9 * DAY_MS), &r)
            .unwrap();
        s.set_pinned(id, true).unwrap();
        s.insert("other", entry("a", "GET", Some(200), day), &r)
            .unwrap();
        let c = s.counts("w", day).unwrap();
        assert_eq!((c.today, c.yesterday, c.this_week, c.older), (1, 1, 1, 1));
        assert_eq!(
            (c.failures, c.slow, c.unsaved, c.pinned, c.runs),
            (2, 1, 1, 1, 1)
        );
        assert_eq!(c.requests.len(), 2);
        assert_eq!((c.requests[0].1.as_str(), c.requests[0].3), ("a", 3));
    }

    #[test]
    fn export_covers_the_whole_filtered_set_without_secrets() {
        let s = Store::open_in_memory().unwrap();
        let r = Redactor::with_secrets(vec![("k".into(), "TOP-SECRET".into())]);
        for i in 0..3 {
            let mut e = entry("a", "GET", Some(200), i);
            e.url = "https://api/a?key=TOP-SECRET".into();
            s.insert("w", e, &r).unwrap();
        }
        s.insert("w", entry("b", "POST", Some(500), 9), &r).unwrap();
        let f = Filter {
            limit: Some(1),
            method: Some("GET".into()),
            ..Default::default()
        };
        let json = s.export("w", &f, ExportFormat::Json).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["entries"].as_array().unwrap().len(), 3);
        assert!(!json.contains("TOP-SECRET"));
        let csv = s.export("w", &f, ExportFormat::Csv).unwrap();
        assert_eq!(csv.trim_end().split("\r\n").count(), 4);
        assert!(!csv.contains("TOP-SECRET"));
    }
}
