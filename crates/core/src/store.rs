use crate::model::UsageEvent;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
  id INTEGER PRIMARY KEY,
  slug TEXT UNIQUE NOT NULL,
  cwd TEXT NOT NULL DEFAULT '',
  display_name TEXT NOT NULL DEFAULT '',
  first_seen TEXT,
  last_seen TEXT,
  loc INTEGER, files INTEGER, commits INTEGER, language TEXT
);
CREATE TABLE IF NOT EXISTS sessions (
  id INTEGER PRIMARY KEY,
  session_id TEXT UNIQUE NOT NULL,
  project_id INTEGER NOT NULL REFERENCES projects(id),
  started_at TEXT,
  ended_at TEXT,
  billing_mode TEXT
);
CREATE TABLE IF NOT EXISTS usage_events (
  id INTEGER PRIMARY KEY,
  dedup_key TEXT UNIQUE NOT NULL,
  ts TEXT NOT NULL,
  project_id INTEGER NOT NULL REFERENCES projects(id),
  session_id INTEGER NOT NULL REFERENCES sessions(id),
  model TEXT NOT NULL,
  is_sidechain INTEGER NOT NULL DEFAULT 0,
  input_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0,
  thinking_tokens INTEGER NOT NULL DEFAULT 0,
  cache_write_5m INTEGER NOT NULL DEFAULT 0,
  cache_write_1h INTEGER NOT NULL DEFAULT 0,
  cache_read INTEGER NOT NULL DEFAULT 0,
  cost_usd REAL
);
CREATE INDEX IF NOT EXISTS idx_events_ts ON usage_events(ts);
CREATE INDEX IF NOT EXISTS idx_events_project ON usage_events(project_id);
CREATE INDEX IF NOT EXISTS idx_events_session ON usage_events(session_id);
CREATE TABLE IF NOT EXISTS model_prices (
  id INTEGER PRIMARY KEY,
  model TEXT NOT NULL,
  input_cost REAL NOT NULL,
  output_cost REAL NOT NULL,
  cache_read_cost REAL NOT NULL,
  cache_write_5m_cost REAL NOT NULL,
  cache_write_1h_cost REAL NOT NULL,
  effective_from TEXT NOT NULL,
  source TEXT NOT NULL,
  UNIQUE(model, effective_from)
);
CREATE TABLE IF NOT EXISTS file_offsets (
  path TEXT PRIMARY KEY,
  offset INTEGER NOT NULL,
  mtime TEXT
);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
"#;

pub fn open_db(path: &Path) -> rusqlite::Result<Connection> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

pub fn open_memory() -> rusqlite::Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

pub fn default_db_path() -> PathBuf {
    if let Ok(p) = std::env::var("BOOKHOLDER_DB") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("Library/Application Support/bookholder/bookholder.db")
}

pub fn claude_projects_dir() -> PathBuf {
    if let Ok(p) = std::env::var("BOOKHOLDER_CLAUDE_DIR") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".claude/projects")
}

pub fn normalize_ts(iso: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(iso)
        .map(|dt| dt.with_timezone(&chrono::Utc).format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|_| iso.to_string())
}

fn ensure_project(conn: &Connection, slug: &str, cwd: &str, ts: &str) -> rusqlite::Result<i64> {
    let display = cwd.rsplit('/').next().unwrap_or(slug).to_string();
    conn.execute(
        "INSERT INTO projects (slug, cwd, display_name, first_seen, last_seen)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(slug) DO UPDATE SET
           cwd = CASE WHEN excluded.cwd != '' THEN excluded.cwd ELSE projects.cwd END,
           display_name = CASE WHEN excluded.display_name != '' THEN excluded.display_name ELSE projects.display_name END,
           last_seen = MAX(projects.last_seen, excluded.last_seen)",
        params![slug, cwd, display, ts],
    )?;
    conn.query_row("SELECT id FROM projects WHERE slug = ?1", [slug], |r| r.get(0))
}

fn ensure_session(conn: &Connection, session_id: &str, project_id: i64, ts: &str, billing: &str) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO sessions (session_id, project_id, started_at, ended_at, billing_mode)
         VALUES (?1, ?2, ?3, ?3, ?4)
         ON CONFLICT(session_id) DO UPDATE SET
           started_at = MIN(sessions.started_at, excluded.started_at),
           ended_at = MAX(sessions.ended_at, excluded.ended_at)",
        params![session_id, project_id, ts, billing],
    )?;
    conn.query_row("SELECT id FROM sessions WHERE session_id = ?1", [session_id], |r| r.get(0))
}

/// 返回 true 表示新插入（false = 去重覆盖或忽略）
pub fn record_event(
    conn: &Connection,
    slug: &str,
    e: &UsageEvent,
    cost_usd: Option<f64>,
    billing_mode: &str,
) -> rusqlite::Result<bool> {
    let ts = normalize_ts(&e.ts);
    let project_id = ensure_project(conn, slug, &e.cwd, &ts)?;
    let session_key = if e.session_id.is_empty() { format!("unknown-{slug}") } else { e.session_id.clone() };
    let session_id = ensure_session(conn, &session_key, project_id, &ts, billing_mode)?;
    let existed: bool = conn
        .query_row("SELECT 1 FROM usage_events WHERE dedup_key = ?1", [&e.dedup_key], |_| Ok(true))
        .unwrap_or(false);
    conn.execute(
        "INSERT INTO usage_events
           (dedup_key, ts, project_id, session_id, model, is_sidechain,
            input_tokens, output_tokens, thinking_tokens, cache_write_5m, cache_write_1h, cache_read, cost_usd)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
         ON CONFLICT(dedup_key) DO UPDATE SET
           ts=excluded.ts, model=excluded.model, is_sidechain=excluded.is_sidechain,
           input_tokens=excluded.input_tokens, output_tokens=excluded.output_tokens,
           thinking_tokens=excluded.thinking_tokens, cache_write_5m=excluded.cache_write_5m,
           cache_write_1h=excluded.cache_write_1h, cache_read=excluded.cache_read, cost_usd=excluded.cost_usd
         WHERE excluded.output_tokens > usage_events.output_tokens",
        params![
            e.dedup_key, ts, project_id, session_id, e.model, e.is_sidechain as i64,
            e.input_tokens, e.output_tokens, e.thinking_tokens,
            e.cache_write_5m, e.cache_write_1h, e.cache_read, cost_usd
        ],
    )?;
    Ok(!existed)
}

pub fn get_offset(conn: &Connection, path: &str) -> i64 {
    conn.query_row("SELECT offset FROM file_offsets WHERE path = ?1", [path], |r| r.get(0))
        .unwrap_or(0)
}

pub fn set_offset(conn: &Connection, path: &str, offset: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO file_offsets (path, offset) VALUES (?1, ?2)
         ON CONFLICT(path) DO UPDATE SET offset = excluded.offset",
        params![path, offset],
    )?;
    Ok(())
}

pub fn meta_get(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).ok()
}

pub fn meta_set(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn bump_counter(conn: &Connection, key: &str, by: i64) -> rusqlite::Result<()> {
    let cur: i64 = meta_get(conn, key).and_then(|v| v.parse().ok()).unwrap_or(0);
    meta_set(conn, key, &(cur + by).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::UsageEvent;

    fn ev(key: &str, out: i64) -> UsageEvent {
        UsageEvent {
            dedup_key: key.into(),
            ts: "2026-08-27T07:15:00.123Z".into(),
            session_id: "s1".into(),
            cwd: "/Users/me/proj".into(),
            model: "claude-fable-5".into(),
            is_sidechain: false,
            input_tokens: 2,
            output_tokens: out,
            thinking_tokens: 0,
            cache_write_5m: 100,
            cache_write_1h: 0,
            cache_read: 0,
        }
    }

    #[test]
    fn normalizes_timestamp_to_utc_sqlite_format() {
        assert_eq!(normalize_ts("2026-08-27T07:15:00.123Z"), "2026-08-27 07:15:00");
    }

    #[test]
    fn record_event_inserts_and_creates_dimensions() {
        let conn = open_memory().unwrap();
        assert!(record_event(&conn, "-Users-me-proj", &ev("k1", 10), Some(0.5), "subscription").unwrap());
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM usage_events", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        let name: String = conn
            .query_row("SELECT display_name FROM projects WHERE slug='-Users-me-proj'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "proj");
        let bm: String = conn
            .query_row("SELECT billing_mode FROM sessions WHERE session_id='s1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(bm, "subscription");
    }

    #[test]
    fn dedup_keeps_larger_output() {
        let conn = open_memory().unwrap();
        record_event(&conn, "sl", &ev("k1", 10), Some(0.1), "api").unwrap();
        // 流式重复：同 key 更大的 output 覆盖
        assert!(!record_event(&conn, "sl", &ev("k1", 99), Some(0.9), "api").unwrap());
        // 更小的不覆盖
        record_event(&conn, "sl", &ev("k1", 5), Some(0.05), "api").unwrap();
        let (n, out, cost): (i64, i64, f64) = conn
            .query_row("SELECT COUNT(*), MAX(output_tokens), MAX(cost_usd) FROM usage_events", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_eq!((n, out), (1, 99));
        assert!((cost - 0.9).abs() < 1e-9);
    }

    #[test]
    fn offsets_and_meta_roundtrip() {
        let conn = open_memory().unwrap();
        assert_eq!(get_offset(&conn, "/a.jsonl"), 0);
        set_offset(&conn, "/a.jsonl", 1234).unwrap();
        assert_eq!(get_offset(&conn, "/a.jsonl"), 1234);
        assert_eq!(meta_get(&conn, "x"), None);
        meta_set(&conn, "x", "1").unwrap();
        assert_eq!(meta_get(&conn, "x").as_deref(), Some("1"));
        bump_counter(&conn, "bad_lines", 3).unwrap();
        bump_counter(&conn, "bad_lines", 2).unwrap();
        assert_eq!(meta_get(&conn, "bad_lines").as_deref(), Some("5"));
    }

    #[test]
    fn wal_mode_enabled_on_file_db() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let db_path = temp_dir.path().join("subdir").join("test.db");

        let conn = open_db(&db_path).unwrap();

        // Verify WAL mode is active
        let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(mode.to_lowercase(), "wal");

        // Verify directory was created
        assert!(db_path.parent().unwrap().exists());
    }

    #[test]
    fn session_time_aggregation_min_max() {
        let conn = open_memory().unwrap();

        // Insert event 1 at 02:00
        let mut e1 = ev("k1", 10);
        e1.ts = "2026-08-27T02:00:00Z".into();
        record_event(&conn, "proj", &e1, Some(0.1), "subscription").unwrap();

        // Insert event 2 at 01:00 (earlier)
        let mut e2 = ev("k2", 20);
        e2.ts = "2026-08-27T01:00:00Z".into();
        record_event(&conn, "proj", &e2, Some(0.2), "subscription").unwrap();

        // Insert event 3 at 03:00 (later)
        let mut e3 = ev("k3", 30);
        e3.ts = "2026-08-27T03:00:00Z".into();
        record_event(&conn, "proj", &e3, Some(0.3), "subscription").unwrap();

        // Verify session has min(started_at) and max(ended_at)
        let (started, ended): (String, String) = conn
            .query_row("SELECT started_at, ended_at FROM sessions WHERE session_id='s1'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();

        assert_eq!(started, "2026-08-27 01:00:00");
        assert_eq!(ended, "2026-08-27 03:00:00");
    }

    #[test]
    fn billing_mode_first_write_wins() {
        let conn = open_memory().unwrap();

        // Event 1: session "s1" with billing "subscription"
        record_event(&conn, "proj", &ev("k1", 10), Some(0.1), "subscription").unwrap();

        // Event 2: same session "s1" with billing "api"
        record_event(&conn, "proj", &ev("k2", 20), Some(0.2), "api").unwrap();

        // Verify billing_mode is still "subscription" (first write wins)
        let billing: String = conn
            .query_row("SELECT billing_mode FROM sessions WHERE session_id='s1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(billing, "subscription");
    }
}
