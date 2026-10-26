# ADR-005: History store

Status: accepted (M5)

## Context
SPEC §13 requires a local, redacted, searchable history of every send, with retention, restore, copy as cURL and diff. The runner, history and UI must stay protocol-agnostic (SPEC §5.5).

## Decision
- **`fluxwire-store`** owns SQLite (`rusqlite`, `bundled`, WAL, a `schema_version` table with ordered migrations). It depends on neither the protocol crates nor GPUI. A row holds indexed columns (workspace key, collection, request path, method, URL, status, timing, environment, dirty flag, `run_id` reserved for M9) plus the protocol's JSON payload.
- **Workspace key** is the SHA-256 of the canonical workspace path; the database lives in the user data dir (`FLUXWIRE_DATA_DIR` overrides it for tests and demos).
- **Redaction happens inside `Store::insert`**, before serialization: secret values become `‹secret:name›`, and `Authorization`, `Proxy-Authorization`, `Cookie`, `Set-Cookie` and `X-Api-Key` header values are replaced by `‹redacted›`. HTTP also never snapshots auth-derived headers or query values, only the auth type. A test scans the database and WAL bytes for known secrets.
- **Bodies** are capped at 1 MiB by the protocol (`truncated: true`) and again, generically, by the store.
- **Protocol trait** gains four defaulted methods so history stays protocol-agnostic: `history_request(&Plan)`, `history_secrets(&Plan)`, `history_to_curl(&HistoryRecord)` and `history_to_outcome(&HistoryRecord)`. The runner merges the request snapshot into `payload.request` and records failed sends too (`payload.error`).
- **Threading**: UI calls run on the shared Tokio runtime via `spawn_blocking`; results return through `cx.spawn`. Retention runs on a plain thread at startup and hourly.
- **Settings**: `[history] max_entries = 10000, max_age_days = 30` in `settings.toml`; the smaller limit wins.
- **Restore** applies the stored method, URL, headers and body to the request's own file as span edits, leaving an unsaved (dirty) tab; headers whose value was redacted are skipped.
- **Diff** compares sorted headers line by line and pretty-printed JSON bodies (non-JSON falls back to text). Rows past 2,000 per section are summarised.

## Consequences
Restored URLs are the resolved URLs (variables substituted). Secrets from environments reach the redactor through `RunResult.secrets` (M4). History is never synced or stored in the collection.
