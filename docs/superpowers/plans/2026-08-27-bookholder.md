# Bookholder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 构建 Bookholder——解析 Claude Code 本地转录、按项目/会话/模型/主对话 vs subagent 精确统计 token 成本的 macOS 桌面工具（悬浮窗 + 详细面板 + 可选 CLI）。

**Architecture:** Cargo workspace：`crates/core`（纯逻辑：解析/去重/计价/SQLite/监视）、`crates/cli`（可选命令行）、`app/src-tauri` + `app/ui`（Tauri 2 双窗口，前端 Vite + vanilla TS + ECharts）。所有业务逻辑在 core，UI 与 CLI 都是薄壳。

**Tech Stack:** Rust stable、rusqlite(bundled)、serde_json、chrono、notify + notify-debouncer-mini、ureq、clap、Tauri 2（plugins: dialog, autostart）、Vite、TypeScript、ECharts 5（本地打包）。

**Spec:** `docs/superpowers/specs/2026-08-27-bookholder-design.md`

## Global Constraints

- 平台：macOS（Apple Silicon），Rust stable ≥ 1.80，Node ≥ 20。
- 数据库路径：`~/Library/Application Support/bookholder/bookholder.db`（env `BOOKHOLDER_DB` 可覆盖，测试用）。
- 转录源：`~/.claude/projects/<slug>/*.jsonl`（env `BOOKHOLDER_CLAUDE_DIR` 可覆盖，测试用）。
- 价格单位一律 USD / 单 token（LiteLLM 格式），显示时换算。
- ECharts 等一切前端资源本地打包，禁止 CDN。
- ingest 路径绝不 panic：坏行计数跳过；未知模型 cost 存 NULL，绝不写错误数字。
- GUI 优先：CLI 能做的每件事在设置页/面板上都有按钮。
- 去重键：`message.id + ":" + requestId`，冲突时保留 output_tokens 更大的记录。
- 时间戳入库前统一规范化为 UTC `YYYY-MM-DD HH:MM:SS`（SQLite datetime 可比较格式）。
- 每个任务结束必须 `cargo test`（或指定验证命令）通过后 git commit。

---

### Task 1: Cargo workspace 脚手架

**Files:**
- Create: `Cargo.toml`（workspace 根）
- Create: `crates/core/Cargo.toml`, `crates/core/src/lib.rs`
- Create: `.gitignore`

**Interfaces:**
- Produces: workspace 结构；`bookholder-core` crate，后续任务在其中加模块。

- [ ] **Step 1: 创建 workspace 根 Cargo.toml**

```toml
[workspace]
resolver = "2"
members = ["crates/core", "crates/cli"]

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = "0.4"
rusqlite = { version = "0.32", features = ["bundled"] }
```

（`app/src-tauri` 在 Task 11 加入 members。）

- [ ] **Step 2: 创建 core crate**

`crates/core/Cargo.toml`:

```toml
[package]
name = "bookholder-core"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
chrono = { workspace = true }
rusqlite = { workspace = true }

[dev-dependencies]
tempfile = "3"
```

`crates/core/src/lib.rs`:

```rust
pub mod model;
pub mod parse;
```

先创建两个空模块文件 `src/model.rs`、`src/parse.rs`（内容在 Task 2 填充，此处各放一行 `// see Task 2`——为了让 Step 4 编译通过，本任务先只声明 `pub mod model;`，`parse` 留到 Task 2 再声明。lib.rs 本任务内容：）

```rust
pub mod model;
```

`crates/core/src/model.rs`（本任务仅占位一个空测试确认工程能跑）:

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn workspace_builds() {
        assert_eq!(1 + 1, 2);
    }
}
```

- [ ] **Step 3: 创建 cli crate 占位**（Task 10 才实现，但 workspace members 引用了它）

`crates/cli/Cargo.toml`:

```toml
[package]
name = "bookholder"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "bookholder"
path = "src/main.rs"

[dependencies]
bookholder-core = { path = "../core" }
```

`crates/cli/src/main.rs`:

```rust
fn main() {
    println!("bookholder");
}
```

- [ ] **Step 4: .gitignore + 验证构建**

`.gitignore`:

```
/target
node_modules
dist
.DS_Store
```

Run: `cargo test`
Expected: PASS（1 个测试通过）

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "chore: cargo workspace scaffold (core + cli)"
```

---

### Task 2: 数据模型 + JSONL 解析

**Files:**
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/src/model.rs`
- Create: `crates/core/src/parse.rs`

**Interfaces:**
- Produces:
  - `model::UsageEvent { dedup_key, ts, session_id, cwd, model, is_sidechain, input_tokens, output_tokens, thinking_tokens, cache_write_5m, cache_write_1h, cache_read }`（全部 pub 字段；token 类型 `i64`，其余 `String`/`bool`；derive `Debug, Clone, PartialEq, serde::Serialize`）
  - `parse::ParseOutcome { Event(UsageEvent), Skipped, Bad }`
  - `parse::parse_line(line: &str) -> ParseOutcome`

- [ ] **Step 1: 写失败测试**（`crates/core/src/parse.rs` 底部 `#[cfg(test)]`）

真实样本（与本机验证过的 Claude Code 转录字段一致）：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"{"type":"assistant","uuid":"u1","requestId":"req_1","sessionId":"s1","cwd":"/Users/me/proj","isSidechain":false,"timestamp":"2026-08-27T07:15:00.123Z","message":{"id":"msg_1","model":"claude-fable-5","usage":{"input_tokens":2,"cache_creation_input_tokens":42322,"cache_read_input_tokens":100,"output_tokens":627,"output_tokens_details":{"thinking_tokens":439},"cache_creation":{"ephemeral_5m_input_tokens":40000,"ephemeral_1h_input_tokens":2322}}}}"#;
    const SIDECHAIN: &str = r#"{"type":"assistant","uuid":"u2","requestId":"req_2","sessionId":"s1","cwd":"/Users/me/proj","isSidechain":true,"timestamp":"2026-08-27T07:16:00.000Z","message":{"id":"msg_2","model":"claude-haiku-4-5-20251001","usage":{"input_tokens":10,"cache_creation_input_tokens":500,"cache_read_input_tokens":0,"output_tokens":50}}}"#;

    #[test]
    fn parses_full_assistant_record() {
        let ParseOutcome::Event(e) = parse_line(FULL) else { panic!("expected Event") };
        assert_eq!(e.dedup_key, "msg_1:req_1");
        assert_eq!(e.model, "claude-fable-5");
        assert_eq!(e.session_id, "s1");
        assert_eq!(e.cwd, "/Users/me/proj");
        assert!(!e.is_sidechain);
        assert_eq!(e.input_tokens, 2);
        assert_eq!(e.output_tokens, 627);
        assert_eq!(e.thinking_tokens, 439);
        assert_eq!(e.cache_write_5m, 40000);
        assert_eq!(e.cache_write_1h, 2322);
        assert_eq!(e.cache_read, 100);
        assert_eq!(e.ts, "2026-08-27T07:15:00.123Z");
    }

    #[test]
    fn sidechain_without_cache_breakdown_goes_to_5m() {
        let ParseOutcome::Event(e) = parse_line(SIDECHAIN) else { panic!() };
        assert!(e.is_sidechain);
        assert_eq!(e.model, "claude-haiku-4-5-20251001");
        assert_eq!(e.cache_write_5m, 500); // 无细分时全部按 5 分钟档
        assert_eq!(e.cache_write_1h, 0);
        assert_eq!(e.thinking_tokens, 0);
    }

    #[test]
    fn non_assistant_is_skipped() {
        assert!(matches!(parse_line(r#"{"type":"user","message":{}}"#), ParseOutcome::Skipped));
        assert!(matches!(parse_line(r#"{"type":"file-history-snapshot"}"#), ParseOutcome::Skipped));
    }

    #[test]
    fn assistant_without_usage_is_skipped() {
        assert!(matches!(
            parse_line(r#"{"type":"assistant","message":{"model":"claude-fable-5"}}"#),
            ParseOutcome::Skipped
        ));
    }

    #[test]
    fn synthetic_model_is_skipped() {
        assert!(matches!(
            parse_line(r#"{"type":"assistant","message":{"model":"<synthetic>","usage":{"input_tokens":1}}}"#),
            ParseOutcome::Skipped
        ));
    }

    #[test]
    fn malformed_json_is_bad() {
        assert!(matches!(parse_line("not json {"), ParseOutcome::Bad));
    }

    #[test]
    fn missing_ids_falls_back_to_uuid() {
        let line = r#"{"type":"assistant","uuid":"u9","sessionId":"s1","cwd":"/p","timestamp":"2026-08-27T00:00:00Z","message":{"model":"claude-fable-5","usage":{"input_tokens":1,"output_tokens":1}}}"#;
        let ParseOutcome::Event(e) = parse_line(line) else { panic!() };
        assert_eq!(e.dedup_key, "uuid:u9");
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p bookholder-core`
Expected: 编译失败（`UsageEvent`/`parse_line` 未定义）

- [ ] **Step 3: 实现**

`crates/core/src/model.rs`:

```rust
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UsageEvent {
    pub dedup_key: String,
    pub ts: String, // 原始 ISO8601，入库时规范化
    pub session_id: String,
    pub cwd: String,
    pub model: String,
    pub is_sidechain: bool,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub thinking_tokens: i64,
    pub cache_write_5m: i64,
    pub cache_write_1h: i64,
    pub cache_read: i64,
}
```

`crates/core/src/parse.rs`:

```rust
use crate::model::UsageEvent;
use serde_json::Value;

pub enum ParseOutcome {
    Event(UsageEvent),
    Skipped, // 非 assistant / 无 usage / synthetic
    Bad,     // JSON 解析失败
}

pub fn parse_line(line: &str) -> ParseOutcome {
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return ParseOutcome::Bad,
    };
    if v.get("type").and_then(Value::as_str) != Some("assistant") {
        return ParseOutcome::Skipped;
    }
    let Some(msg) = v.get("message") else { return ParseOutcome::Skipped };
    let Some(usage) = msg.get("usage") else { return ParseOutcome::Skipped };
    let model = msg.get("model").and_then(Value::as_str).unwrap_or("unknown").to_string();
    if model == "<synthetic>" {
        return ParseOutcome::Skipped;
    }
    let s = |obj: &Value, k: &str| obj.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let msg_id = s(msg, "id");
    let req_id = s(&v, "requestId");
    let dedup_key = if msg_id.is_empty() && req_id.is_empty() {
        format!("uuid:{}", s(&v, "uuid"))
    } else {
        format!("{msg_id}:{req_id}")
    };
    let g = |k: &str| usage.get(k).and_then(Value::as_i64).unwrap_or(0);
    let total_cc = g("cache_creation_input_tokens");
    let cc = |k: &str| usage.get("cache_creation").and_then(|c| c.get(k)).and_then(Value::as_i64);
    let (w5, w1) = match (cc("ephemeral_5m_input_tokens"), cc("ephemeral_1h_input_tokens")) {
        (None, None) => (total_cc, 0),
        (a, b) => (a.unwrap_or(0), b.unwrap_or(0)),
    };
    ParseOutcome::Event(UsageEvent {
        dedup_key,
        ts: s(&v, "timestamp"),
        session_id: s(&v, "sessionId"),
        cwd: s(&v, "cwd"),
        model,
        is_sidechain: v.get("isSidechain").and_then(Value::as_bool).unwrap_or(false),
        input_tokens: g("input_tokens"),
        output_tokens: g("output_tokens"),
        thinking_tokens: usage
            .get("output_tokens_details")
            .and_then(|d| d.get("thinking_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
        cache_write_5m: w5,
        cache_write_1h: w1,
        cache_read: g("cache_read_input_tokens"),
    })
}
```

`crates/core/src/lib.rs`:

```rust
pub mod model;
pub mod parse;
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p bookholder-core`
Expected: PASS（7 个测试）

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): UsageEvent model and JSONL line parser"
```

---

### Task 3: SQLite store —— schema、事件写入（含去重）、偏移量、meta

**Files:**
- Create: `crates/core/src/store.rs`
- Modify: `crates/core/src/lib.rs`（加 `pub mod store;`）

**Interfaces:**
- Consumes: `model::UsageEvent`
- Produces（全部在 `store::`）:
  - `open_db(path: &Path) -> rusqlite::Result<Connection>`（建目录、WAL、建表）
  - `open_memory() -> rusqlite::Result<Connection>`（测试用）
  - `default_db_path() -> PathBuf`（env `BOOKHOLDER_DB` 覆盖，否则 `~/Library/Application Support/bookholder/bookholder.db`）
  - `claude_projects_dir() -> PathBuf`（env `BOOKHOLDER_CLAUDE_DIR` 覆盖，否则 `~/.claude/projects`）
  - `record_event(conn, slug: &str, e: &UsageEvent, cost_usd: Option<f64>, billing_mode: &str) -> rusqlite::Result<bool>`（true = 新插入；内部 ensure project/session、时间戳规范化、upsert 保大）
  - `get_offset(conn, path: &str) -> i64` / `set_offset(conn, path: &str, offset: i64)`
  - `meta_get(conn, key: &str) -> Option<String>` / `meta_set(conn, key: &str, value: &str)`
  - `bump_counter(conn, key: &str, by: i64)`（skip_lines / bad_lines 计数）
  - `normalize_ts(iso: &str) -> String`

- [ ] **Step 1: 写失败测试**（`store.rs` 底部）

```rust
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
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p bookholder-core store`
Expected: 编译失败（store 未实现）

- [ ] **Step 3: 实现 `store.rs`**

```rust
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
```

lib.rs 加 `pub mod store;`。

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p bookholder-core`
Expected: PASS（Task 2 的 7 个 + 本任务 4 个）

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): sqlite store with dedup upsert, offsets, meta"
```

---

### Task 4: 价格表 —— 快照、LiteLLM 解析、历史版本、计价

**Files:**
- Create: `crates/core/data/model_prices_snapshot.json`（真实数据，下载生成）
- Create: `crates/core/src/pricing.rs`
- Modify: `crates/core/src/lib.rs`（加 `pub mod pricing;`）
- Modify: `crates/core/Cargo.toml`（加 `ureq = { version = "2", features = ["tls"] }`）

**Interfaces:**
- Consumes: `store`（`model_prices` 表、`meta_set`）、`model::UsageEvent`
- Produces（`pricing::`）:
  - `struct ModelPrice { model: String, input_cost: f64, output_cost: f64, cache_read_cost: f64, cache_write_5m_cost: f64, cache_write_1h_cost: f64 }`（derive `Debug, Clone, PartialEq, Serialize`）
  - `parse_litellm(json: &str) -> Vec<ModelPrice>`
  - `lookup<'a>(prices: &'a [ModelPrice], model: &str) -> Option<&'a ModelPrice>`
  - `cost_usd(e: &UsageEvent, p: &ModelPrice) -> f64`
  - `seed_snapshot(conn) -> rusqlite::Result<usize>`（内置快照入库，effective_from = '1970-01-01 00:00:00'，source = 'snapshot'）
  - `upsert_prices(conn, prices: &[ModelPrice], source: &str, now: &str) -> rusqlite::Result<usize>`（仅当与该模型最新价不同才插新版本行）
  - `latest_price(conn, model: &str) -> Option<ModelPrice>`（含 lookup 的模糊匹配：先精确后剥日期前缀匹配）
  - `refresh_from_network(conn, now: &str) -> Result<usize, String>`（拉 LiteLLM URL → parse → upsert → meta `prices_last_fetch`/`prices_last_status`）
  - `reprice_null_costs(conn) -> rusqlite::Result<usize>`（为 cost_usd IS NULL 的事件按最新价补算）
  - `PRICES_URL: &str`

- [ ] **Step 1: 下载真实快照（只保留 claude 模型，缩小体积）**

```bash
curl -fsSL https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json \
| python3 -c "import json,sys; d=json.load(sys.stdin); json.dump({k:v for k,v in d.items() if 'claude' in k.lower()}, open('crates/core/data/model_prices_snapshot.json','w'), indent=1)"
```

Run: `python3 -c "import json; d=json.load(open('crates/core/data/model_prices_snapshot.json')); print(len(d))"`
Expected: 输出一个 >10 的数字（claude 模型条目数）

- [ ] **Step 2: 写失败测试**（`pricing.rs` 底部）

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::UsageEvent;

    const SNAPSHOT: &str = include_str!("../data/model_prices_snapshot.json");

    fn ev() -> UsageEvent {
        UsageEvent {
            dedup_key: "k".into(), ts: "2026-08-27T00:00:00Z".into(), session_id: "s".into(),
            cwd: "/p".into(), model: "m".into(), is_sidechain: false,
            input_tokens: 1_000_000, output_tokens: 1_000_000, thinking_tokens: 0,
            cache_write_5m: 1_000_000, cache_write_1h: 1_000_000, cache_read: 1_000_000,
        }
    }

    #[test]
    fn parses_snapshot_with_positive_costs() {
        let prices = parse_litellm(SNAPSHOT);
        assert!(prices.len() > 5);
        for p in &prices {
            assert!(p.input_cost > 0.0, "{} input", p.model);
            assert!(p.output_cost > 0.0, "{} output", p.model);
            assert!(p.cache_write_5m_cost > p.input_cost, "{} 5m write > input", p.model);
            assert!(p.cache_write_1h_cost > p.cache_write_5m_cost, "{} 1h > 5m", p.model);
            assert!(!p.model.contains('/'), "键应剥掉 provider 前缀: {}", p.model);
        }
    }

    #[test]
    fn cost_formula_sums_all_buckets() {
        let p = ModelPrice {
            model: "m".into(), input_cost: 3e-6, output_cost: 15e-6,
            cache_read_cost: 0.3e-6, cache_write_5m_cost: 3.75e-6, cache_write_1h_cost: 6e-6,
        };
        // 每类各 1M token：3 + 15 + 0.3 + 3.75 + 6 = 28.05 USD
        assert!((cost_usd(&ev(), &p) - 28.05).abs() < 1e-6);
    }

    #[test]
    fn lookup_matches_exact_then_date_stripped() {
        let prices = vec![ModelPrice {
            model: "claude-opus-4-1".into(), input_cost: 1e-6, output_cost: 2e-6,
            cache_read_cost: 1e-7, cache_write_5m_cost: 1.25e-6, cache_write_1h_cost: 2e-6,
        }];
        assert!(lookup(&prices, "claude-opus-4-1").is_some());
        assert!(lookup(&prices, "claude-opus-4-1-20250805").is_some()); // 剥日期后缀
        assert!(lookup(&prices, "gpt-4o").is_none());
    }

    #[test]
    fn upsert_only_on_change_and_latest_wins() {
        let conn = crate::store::open_memory().unwrap();
        let p1 = vec![ModelPrice { model: "claude-x".into(), input_cost: 1e-6, output_cost: 2e-6,
            cache_read_cost: 1e-7, cache_write_5m_cost: 1.25e-6, cache_write_1h_cost: 2e-6 }];
        assert_eq!(upsert_prices(&conn, &p1, "test", "2026-01-01 00:00:00").unwrap(), 1);
        assert_eq!(upsert_prices(&conn, &p1, "test", "2026-01-02 00:00:00").unwrap(), 0); // 没变不插
        let p2 = vec![ModelPrice { input_cost: 9e-6, ..p1[0].clone() }];
        assert_eq!(upsert_prices(&conn, &p2, "test", "2026-01-03 00:00:00").unwrap(), 1);
        let latest = latest_price(&conn, "claude-x").unwrap();
        assert!((latest.input_cost - 9e-6).abs() < 1e-12);
    }

    #[test]
    fn seed_and_reprice_null_costs() {
        let conn = crate::store::open_memory().unwrap();
        seed_snapshot(&conn).unwrap();
        let mut e = ev();
        e.model = parse_litellm(SNAPSHOT)[0].model.clone();
        crate::store::record_event(&conn, "sl", &e, None, "api").unwrap();
        assert_eq!(reprice_null_costs(&conn).unwrap(), 1);
        let c: f64 = conn.query_row("SELECT cost_usd FROM usage_events", [], |r| r.get(0)).unwrap();
        assert!(c > 0.0);
    }
}
```

- [ ] **Step 3: 运行确认失败**

Run: `cargo test -p bookholder-core pricing`
Expected: 编译失败

- [ ] **Step 4: 实现 `pricing.rs`**

```rust
use crate::model::UsageEvent;
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::Value;

pub const PRICES_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
const SNAPSHOT_JSON: &str = include_str!("../data/model_prices_snapshot.json");

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelPrice {
    pub model: String,
    pub input_cost: f64,
    pub output_cost: f64,
    pub cache_read_cost: f64,
    pub cache_write_5m_cost: f64,
    pub cache_write_1h_cost: f64,
}

pub fn parse_litellm(json: &str) -> Vec<ModelPrice> {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return vec![] };
    let Some(obj) = v.as_object() else { return vec![] };
    let mut out: Vec<ModelPrice> = vec![];
    for (key, m) in obj {
        let name = key.rsplit('/').next().unwrap_or(key).to_string();
        if !name.starts_with("claude") { continue; }
        let f = |k: &str| m.get(k).and_then(Value::as_f64);
        let (Some(input), Some(output)) = (f("input_cost_per_token"), f("output_cost_per_token")) else { continue };
        if input <= 0.0 || output <= 0.0 { continue; }
        let w5 = f("cache_creation_input_token_cost").unwrap_or(input * 1.25);
        let w1 = f("cache_creation_input_token_cost_above_1hr").unwrap_or(input * 2.0);
        let read = f("cache_read_input_token_cost").unwrap_or(input * 0.1);
        if out.iter().any(|p: &ModelPrice| p.model == name) { continue; } // 去掉前缀后的重复键
        out.push(ModelPrice {
            model: name, input_cost: input, output_cost: output,
            cache_read_cost: read, cache_write_5m_cost: w5, cache_write_1h_cost: w1,
        });
    }
    out
}

pub fn cost_usd(e: &UsageEvent, p: &ModelPrice) -> f64 {
    e.input_tokens as f64 * p.input_cost
        + e.output_tokens as f64 * p.output_cost
        + e.cache_read as f64 * p.cache_read_cost
        + e.cache_write_5m as f64 * p.cache_write_5m_cost
        + e.cache_write_1h as f64 * p.cache_write_1h_cost
}

/// 事件模型名尾部常带 -YYYYMMDD 日期；价格键有的带有的不带。双向剥日期比较。
fn strip_date(name: &str) -> &str {
    if name.len() > 9 {
        let (head, tail) = name.split_at(name.len() - 9);
        if tail.starts_with('-') && tail[1..].chars().all(|c| c.is_ascii_digit()) {
            return head;
        }
    }
    name
}

pub fn lookup<'a>(prices: &'a [ModelPrice], model: &str) -> Option<&'a ModelPrice> {
    prices.iter().find(|p| p.model == model)
        .or_else(|| prices.iter().find(|p| strip_date(&p.model) == strip_date(model)))
}

pub fn upsert_prices(conn: &Connection, prices: &[ModelPrice], source: &str, now: &str) -> rusqlite::Result<usize> {
    let mut inserted = 0;
    for p in prices {
        let changed = match latest_price_exact(conn, &p.model) {
            Some(cur) => cur != *p,
            None => true,
        };
        if changed {
            conn.execute(
                "INSERT OR IGNORE INTO model_prices
                   (model, input_cost, output_cost, cache_read_cost, cache_write_5m_cost, cache_write_1h_cost, effective_from, source)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![p.model, p.input_cost, p.output_cost, p.cache_read_cost,
                        p.cache_write_5m_cost, p.cache_write_1h_cost, now, source],
            )?;
            inserted += 1;
        }
    }
    Ok(inserted)
}

fn latest_price_exact(conn: &Connection, model: &str) -> Option<ModelPrice> {
    conn.query_row(
        "SELECT model, input_cost, output_cost, cache_read_cost, cache_write_5m_cost, cache_write_1h_cost
         FROM model_prices WHERE model = ?1 ORDER BY effective_from DESC LIMIT 1",
        [model],
        |r| Ok(ModelPrice {
            model: r.get(0)?, input_cost: r.get(1)?, output_cost: r.get(2)?,
            cache_read_cost: r.get(3)?, cache_write_5m_cost: r.get(4)?, cache_write_1h_cost: r.get(5)?,
        }),
    ).ok()
}

pub fn latest_price(conn: &Connection, model: &str) -> Option<ModelPrice> {
    if let Some(p) = latest_price_exact(conn, model) { return Some(p); }
    // 剥日期匹配：取全部最新价再 lookup
    let mut stmt = conn.prepare(
        "SELECT model, input_cost, output_cost, cache_read_cost, cache_write_5m_cost, cache_write_1h_cost
         FROM model_prices p WHERE effective_from = (
           SELECT MAX(effective_from) FROM model_prices WHERE model = p.model)"
    ).ok()?;
    let all: Vec<ModelPrice> = stmt.query_map([], |r| Ok(ModelPrice {
        model: r.get(0)?, input_cost: r.get(1)?, output_cost: r.get(2)?,
        cache_read_cost: r.get(3)?, cache_write_5m_cost: r.get(4)?, cache_write_1h_cost: r.get(5)?,
    })).ok()?.flatten().collect();
    lookup(&all, model).cloned()
}

pub fn seed_snapshot(conn: &Connection) -> rusqlite::Result<usize> {
    upsert_prices(conn, &parse_litellm(SNAPSHOT_JSON), "snapshot", "1970-01-01 00:00:00")
}

pub fn refresh_from_network(conn: &Connection, now: &str) -> Result<usize, String> {
    let body = ureq::get(PRICES_URL).call().map_err(|e| e.to_string())?
        .into_string().map_err(|e| e.to_string())?;
    let prices = parse_litellm(&body);
    if prices.is_empty() { return Err("parsed 0 claude models".into()); }
    let n = upsert_prices(conn, &prices, "litellm", now).map_err(|e| e.to_string())?;
    let _ = crate::store::meta_set(conn, "prices_last_fetch", now);
    let _ = crate::store::meta_set(conn, "prices_last_status", "ok");
    Ok(n)
}

pub fn reprice_null_costs(conn: &Connection) -> rusqlite::Result<usize> {
    let mut stmt = conn.prepare(
        "SELECT id, model, input_tokens, output_tokens, cache_write_5m, cache_write_1h, cache_read
         FROM usage_events WHERE cost_usd IS NULL")?;
    let rows: Vec<(i64, String, i64, i64, i64, i64, i64)> = stmt.query_map([], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
    })?.flatten().collect();
    let mut n = 0;
    for (id, model, input, output, w5, w1, read) in rows {
        if let Some(p) = latest_price(conn, &model) {
            let cost = input as f64 * p.input_cost + output as f64 * p.output_cost
                + read as f64 * p.cache_read_cost
                + w5 as f64 * p.cache_write_5m_cost + w1 as f64 * p.cache_write_1h_cost;
            conn.execute("UPDATE usage_events SET cost_usd = ?1 WHERE id = ?2", params![cost, id])?;
            n += 1;
        }
    }
    Ok(n)
}
```

Cargo.toml 加 `ureq = { version = "2", features = ["tls"] }`；lib.rs 加 `pub mod pricing;`。

注意：若快照数据导致 `cache_write_1h_cost > cache_write_5m_cost` 断言失败（LiteLLM 缺 1h 字段时 fallback 是 input×2.0 > input×1.25，正常应通过；若个别模型自带字段违反此关系，放宽该条断言为 `> 0.0` 并在测试注释说明）。

- [ ] **Step 5: 运行确认通过**

Run: `cargo test -p bookholder-core`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat(core): pricing with litellm snapshot, versioned prices, repricing"
```

---

### Task 5: 计费模式检测（订阅 vs API）

**Files:**
- Create: `crates/core/src/billing.rs`
- Modify: `crates/core/src/lib.rs`（加 `pub mod billing;`）

**Interfaces:**
- Produces（`billing::`）:
  - `enum BillingMode { Subscription, Api, Unknown }`，实现 `as_str() -> &'static str`（"subscription"/"api"/"unknown"）
  - `detect(home: &Path) -> BillingMode`
  - `effective_mode(conn: &Connection, home: &Path) -> String`（meta `billing_override` 优先，否则 detect）

判定规则：`<home>/.claude.json` 存在且含非空 `oauthAccount` 对象 → Subscription；否则若环境变量 `ANTHROPIC_API_KEY` 非空或 `<home>/.claude/settings.json` 含 `apiKeyHelper` → Api；否则 Unknown。

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn detects_subscription_from_oauth_account() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"a@b.c"}}"#).unwrap();
        assert!(matches!(detect(dir.path()), BillingMode::Subscription));
    }

    #[test]
    fn detects_api_from_api_key_helper() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".claude.json"), r#"{}"#).unwrap();
        fs::create_dir_all(dir.path().join(".claude")).unwrap();
        fs::write(dir.path().join(".claude/settings.json"), r#"{"apiKeyHelper":"/bin/key.sh"}"#).unwrap();
        assert!(matches!(detect(dir.path()), BillingMode::Api));
    }

    #[test]
    fn unknown_when_nothing_present() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(detect(dir.path()), BillingMode::Unknown));
    }

    #[test]
    fn override_wins() {
        let conn = crate::store::open_memory().unwrap();
        crate::store::meta_set(&conn, "billing_override", "api").unwrap();
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(effective_mode(&conn, dir.path()), "api");
    }
}
```

（注意：`detect` 不读 `ANTHROPIC_API_KEY` 之外的环境变量；测试中该变量可能存在于开发者 shell，`unknown_when_nothing_present` 若因此不稳定，在 detect 增加参数 `env_api_key: Option<&str>`，生产调用处传 `std::env::var("ANTHROPIC_API_KEY").ok().as_deref()`，测试传 `None`。按此签名实现：`detect(home: &Path, env_api_key: Option<&str>) -> BillingMode`。）

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p bookholder-core billing`
Expected: 编译失败

- [ ] **Step 3: 实现 `billing.rs`**

```rust
use rusqlite::Connection;
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BillingMode { Subscription, Api, Unknown }

impl BillingMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            BillingMode::Subscription => "subscription",
            BillingMode::Api => "api",
            BillingMode::Unknown => "unknown",
        }
    }
}

pub fn detect(home: &Path, env_api_key: Option<&str>) -> BillingMode {
    let claude_json = home.join(".claude.json");
    if let Ok(txt) = std::fs::read_to_string(&claude_json) {
        if let Ok(v) = serde_json::from_str::<Value>(&txt) {
            if v.get("oauthAccount").map(|o| o.is_object() && !o.as_object().unwrap().is_empty()).unwrap_or(false) {
                return BillingMode::Subscription;
            }
        }
    }
    if env_api_key.map(|k| !k.is_empty()).unwrap_or(false) {
        return BillingMode::Api;
    }
    if let Ok(txt) = std::fs::read_to_string(home.join(".claude/settings.json")) {
        if txt.contains("apiKeyHelper") {
            return BillingMode::Api;
        }
    }
    BillingMode::Unknown
}

pub fn effective_mode(conn: &Connection, home: &Path) -> String {
    if let Some(o) = crate::store::meta_get(conn, "billing_override") {
        if o == "subscription" || o == "api" { return o; }
    }
    detect(home, std::env::var("ANTHROPIC_API_KEY").ok().as_deref()).as_str().to_string()
}
```

测试相应用 `detect(dir.path(), None)` 调用。

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p bookholder-core`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): billing mode detection with manual override"
```

---

### Task 6: ingest —— 全量扫描 + 偏移量增量读取

**Files:**
- Create: `crates/core/src/ingest.rs`
- Modify: `crates/core/src/lib.rs`（加 `pub mod ingest;`）

**Interfaces:**
- Consumes: `parse::parse_line`、`store::*`、`pricing::latest_price`/`cost_usd`
- Produces（`ingest::`）:
  - `struct IngestStats { pub added: u64, pub skipped: u64, pub bad: u64 }`（derive `Debug, Default, Clone, Serialize`，实现 `merge(&mut self, other: &IngestStats)`）
  - `ingest_file(conn: &Connection, path: &Path, slug: &str, billing: &str) -> IngestStats`（从记录偏移读到 EOF；只处理完整行——最后不带 `\n` 的半行不消费、偏移停在行首，下次再读）
  - `scan_all(conn: &Connection, projects_dir: &Path, billing: &str) -> IngestStats`（遍历 `<projects_dir>/*/*.jsonl`）

计价内联：每个事件入库前 `pricing::latest_price(conn, &e.model)` 算 cost，找不到价则 `None`。文件缩水（被清理重建，len < offset）时偏移归零重读。skipped/bad 计数累加进 meta（`skip_lines`/`bad_lines`）。

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, OpenOptions};
    use std::io::Write;

    const L1: &str = r#"{"type":"assistant","uuid":"u1","requestId":"r1","sessionId":"s1","cwd":"/Users/me/alpha","isSidechain":false,"timestamp":"2026-08-27T01:00:00Z","message":{"id":"m1","model":"claude-fable-5","usage":{"input_tokens":10,"output_tokens":20}}}"#;
    const L2: &str = r#"{"type":"user","message":{}}"#;
    const L3: &str = r#"{"type":"assistant","uuid":"u2","requestId":"r2","sessionId":"s1","cwd":"/Users/me/alpha","isSidechain":true,"timestamp":"2026-08-27T01:01:00Z","message":{"id":"m2","model":"claude-haiku-4-5-20251001","usage":{"input_tokens":5,"output_tokens":7}}}"#;

    fn setup() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("-Users-me-alpha");
        fs::create_dir_all(&proj).unwrap();
        (dir, proj.join("sess1.jsonl"))
    }

    #[test]
    fn scan_ingests_and_counts() {
        let (dir, file) = setup();
        fs::write(&file, format!("{L1}\n{L2}\nnot-json\n")).unwrap();
        let conn = crate::store::open_memory().unwrap();
        let st = scan_all(&conn, dir.path(), "api");
        assert_eq!((st.added, st.skipped, st.bad), (1, 1, 1));
        let slug: String = conn.query_row("SELECT slug FROM projects", [], |r| r.get(0)).unwrap();
        assert_eq!(slug, "-Users-me-alpha");
    }

    #[test]
    fn incremental_reads_only_new_bytes() {
        let (dir, file) = setup();
        fs::write(&file, format!("{L1}\n")).unwrap();
        let conn = crate::store::open_memory().unwrap();
        assert_eq!(scan_all(&conn, dir.path(), "api").added, 1);
        assert_eq!(scan_all(&conn, dir.path(), "api").added, 0); // 无新内容
        let mut f = OpenOptions::new().append(true).open(&file).unwrap();
        writeln!(f, "{L3}").unwrap();
        let st = scan_all(&conn, dir.path(), "api");
        assert_eq!(st.added, 1);
        let side: i64 = conn.query_row(
            "SELECT COUNT(*) FROM usage_events WHERE is_sidechain=1", [], |r| r.get(0)).unwrap();
        assert_eq!(side, 1);
    }

    #[test]
    fn partial_last_line_is_not_consumed() {
        let (dir, file) = setup();
        fs::write(&file, format!("{L1}\n{}", &L3[..40])).unwrap(); // 半行
        let conn = crate::store::open_memory().unwrap();
        assert_eq!(scan_all(&conn, dir.path(), "api").added, 1);
        // 半行补全后能读到
        let mut f = OpenOptions::new().append(true).open(&file).unwrap();
        write!(f, "{}\n", &L3[40..]).unwrap();
        assert_eq!(scan_all(&conn, dir.path(), "api").added, 1);
    }

    #[test]
    fn shrunk_file_rereads_from_zero() {
        let (dir, file) = setup();
        fs::write(&file, format!("{L1}\n{L3}\n")).unwrap();
        let conn = crate::store::open_memory().unwrap();
        assert_eq!(scan_all(&conn, dir.path(), "api").added, 2);
        fs::write(&file, format!("{L1}\n")).unwrap(); // 缩水
        let st = scan_all(&conn, dir.path(), "api");
        assert_eq!(st.added, 0); // 重读但 dedup 挡住
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p bookholder-core ingest`
Expected: 编译失败

- [ ] **Step 3: 实现 `ingest.rs`**

```rust
use crate::parse::{parse_line, ParseOutcome};
use rusqlite::Connection;
use serde::Serialize;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Default, Clone, Serialize)]
pub struct IngestStats {
    pub added: u64,
    pub skipped: u64,
    pub bad: u64,
}

impl IngestStats {
    pub fn merge(&mut self, o: &IngestStats) {
        self.added += o.added;
        self.skipped += o.skipped;
        self.bad += o.bad;
    }
}

pub fn ingest_file(conn: &Connection, path: &Path, slug: &str, billing: &str) -> IngestStats {
    let mut st = IngestStats::default();
    let key = path.to_string_lossy().to_string();
    let mut offset = crate::store::get_offset(conn, &key);
    let Ok(mut f) = std::fs::File::open(path) else { return st };
    let len = f.metadata().map(|m| m.len() as i64).unwrap_or(0);
    if len < offset {
        offset = 0; // 文件被重建，重读（dedup 保证不重复计数）
    }
    if len == offset { return st; }
    if f.seek(SeekFrom::Start(offset as u64)).is_err() { return st; }
    let mut buf = String::new();
    if f.read_to_string(&mut buf).is_err() {
        // 非 UTF-8 边界等：按字节读再有损转换
        let mut bytes = vec![];
        let _ = f.seek(SeekFrom::Start(offset as u64));
        if f.read_to_end(&mut bytes).is_err() { return st; }
        buf = String::from_utf8_lossy(&bytes).into_owned();
    }
    // 只消费完整行
    let consumed_end = match buf.rfind('\n') {
        Some(i) => i + 1,
        None => { return st; } // 整段都是半行
    };
    for line in buf[..consumed_end].lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        match parse_line(line) {
            ParseOutcome::Event(e) => {
                let cost = crate::pricing::latest_price(conn, &e.model)
                    .map(|p| crate::pricing::cost_usd(&e, &p));
                match crate::store::record_event(conn, slug, &e, cost, billing) {
                    Ok(true) => st.added += 1,
                    Ok(false) => {} // 去重
                    Err(_) => st.bad += 1,
                }
            }
            ParseOutcome::Skipped => st.skipped += 1,
            ParseOutcome::Bad => st.bad += 1,
        }
    }
    let _ = crate::store::set_offset(conn, &key, offset + consumed_end as i64);
    let _ = crate::store::bump_counter(conn, "skip_lines", st.skipped as i64);
    let _ = crate::store::bump_counter(conn, "bad_lines", st.bad as i64);
    st
}

pub fn scan_all(conn: &Connection, projects_dir: &Path, billing: &str) -> IngestStats {
    let mut total = IngestStats::default();
    let Ok(entries) = std::fs::read_dir(projects_dir) else { return total };
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_dir() { continue; }
        let slug = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let Ok(files) = std::fs::read_dir(&p) else { continue };
        for f in files.flatten() {
            let fp = f.path();
            if fp.extension().map(|e| e == "jsonl").unwrap_or(false) {
                total.merge(&ingest_file(conn, &fp, &slug, billing));
            }
        }
    }
    total
}
```

注意 `buf[..consumed_end]` 的字节/字符边界：`read_to_string` 后 `rfind` 返回的是字节索引，`&buf[..consumed_end]` 要求 UTF-8 边界——`\n` 是单字节 ASCII，一定在边界上，安全。但 offset 累加必须用**字节数**：`consumed_end` 已是字节数，正确。

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p bookholder-core`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): incremental jsonl ingest with offsets and inline pricing"
```

---

### Task 7: 文件监视（fsevents 实时）

**Files:**
- Create: `crates/core/src/watcher.rs`
- Modify: `crates/core/src/lib.rs`（加 `pub mod watcher;`）
- Modify: `crates/core/Cargo.toml`（加 `notify = "6"`、`notify-debouncer-mini = "0.4"`）

**Interfaces:**
- Consumes: 无（只封装 notify）
- Produces（`watcher::`）:
  - `watch_projects(dir: &Path, on_change: impl FnMut() + Send + 'static) -> Result<Box<dyn std::any::Any + Send>, String>`——启动递归监视，任何 .jsonl 变化经 500ms 去抖后调用 `on_change`；返回值是需要持有的 watcher 句柄（drop 即停止）。调用方在 `on_change` 里自己跑 `scan_all`（增量，代价极小）。

- [ ] **Step 1: 写失败测试**（integration 风格，仍放在模块内）

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn fires_on_change_after_debounce() {
        let dir = tempfile::tempdir().unwrap();
        let counter = Arc::new(AtomicU32::new(0));
        let c2 = counter.clone();
        let _guard = watch_projects(dir.path(), move || {
            c2.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        std::thread::sleep(Duration::from_millis(300)); // 等 watcher 就绪
        std::fs::write(dir.path().join("a.jsonl"), "x\n").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while counter.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(counter.load(Ordering::SeqCst) >= 1, "watcher 未触发");
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p bookholder-core watcher`
Expected: 编译失败

- [ ] **Step 3: 实现 `watcher.rs`**

```rust
use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};
use std::path::Path;
use std::time::Duration;

pub fn watch_projects(
    dir: &Path,
    mut on_change: impl FnMut() + Send + 'static,
) -> Result<Box<dyn std::any::Any + Send>, String> {
    let mut debouncer = new_debouncer(Duration::from_millis(500), move |res| {
        if let Ok(events) = res {
            let hit = {
                let evs: &Vec<notify_debouncer_mini::DebouncedEvent> = &events;
                evs.iter().any(|e| {
                    e.path.extension().map(|x| x == "jsonl").unwrap_or(false)
                        || e.path.is_dir()
                })
            };
            if hit {
                on_change();
            }
        }
    })
    .map_err(|e| e.to_string())?;
    debouncer
        .watcher()
        .watch(dir, RecursiveMode::Recursive)
        .map_err(|e| e.to_string())?;
    Ok(Box::new(debouncer))
}
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p bookholder-core`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): debounced fsevents watcher for transcript changes"
```

---

### Task 8: 聚合查询

**Files:**
- Create: `crates/core/src/queries.rs`
- Modify: `crates/core/src/lib.rs`（加 `pub mod queries;`）

**Interfaces:**
- Consumes: store 的表结构
- Produces（`queries::`，全部 struct derive `Debug, Clone, Serialize`）:
  - `struct Totals { cost_usd: f64, input: i64, output: i64, thinking: i64, cache_read: i64, cache_write: i64, events: i64, unpriced: i64 }`
  - `totals(conn, from: Option<&str>, to: Option<&str>, project_id: Option<i64>) -> Totals`（from/to 为规范化 UTC 时间串，闭开区间）
  - `struct DailyModelRow { date: String, model: String, cost_usd: f64 }` / `daily_by_model(conn, days: i64) -> Vec<DailyModelRow>`（本地时区日期）
  - `struct HourRow { hour: String, main_cost: f64, side_cost: f64 }` / `hourly_last24(conn) -> Vec<HourRow>`
  - `burn_rate_per_hour(conn) -> f64`（最近 30 分钟 cost×2）
  - `struct ModelSplitRow { model: String, cost_usd: f64, input: i64, output: i64, events: i64 }` / `model_split(conn, from, to) -> Vec<ModelSplitRow>`
  - `sidechain_split(conn, from: Option<&str>, to: Option<&str>) -> (f64, f64)`（(主对话 cost, subagent cost)）
  - `struct ProjectRow { id: i64, display_name: String, cwd: String, cost_usd: f64, tokens: i64, sessions: i64, active_days: i64, last_seen: String }` / `project_rows(conn) -> Vec<ProjectRow>`（按 cost 降序）
  - `struct SessionRow { id: i64, session_id: String, started_at: String, ended_at: String, billing_mode: String, cost_usd: f64, events: i64, side_cost: f64 }` / `session_rows(conn, project_id: i64) -> Vec<SessionRow>`
  - `struct EventRow { ts: String, model: String, is_sidechain: bool, input: i64, output: i64, thinking: i64, cache_write_5m: i64, cache_write_1h: i64, cache_read: i64, cost_usd: Option<f64> }` / `event_rows(conn, session_pk: i64) -> Vec<EventRow>`
  - `struct CurrentCtx { project_id: i64, project_name: String, model: String, last_ts: String }` / `current_context(conn) -> Option<CurrentCtx>`（最近一条事件）
  - `today_range() -> (String, String)`（本地今日的 UTC 起止串，给 totals 用）

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::UsageEvent;

    fn seed(conn: &rusqlite::Connection) {
        let mk = |key: &str, ts: &str, model: &str, side: bool, out: i64, sess: &str, cwd: &str| UsageEvent {
            dedup_key: key.into(), ts: ts.into(), session_id: sess.into(), cwd: cwd.into(),
            model: model.into(), is_sidechain: side, input_tokens: 100, output_tokens: out,
            thinking_tokens: 10, cache_write_5m: 50, cache_write_1h: 0, cache_read: 200,
        };
        crate::store::record_event(conn, "-a", &mk("k1", "2026-08-26T02:00:00Z", "claude-fable-5", false, 1000, "s1", "/u/alpha"), Some(1.0), "api").unwrap();
        crate::store::record_event(conn, "-a", &mk("k2", "2026-08-26T03:00:00Z", "claude-haiku-4-5", true, 500, "s1", "/u/alpha"), Some(0.2), "api").unwrap();
        crate::store::record_event(conn, "-b", &mk("k3", "2026-08-27T04:00:00Z", "claude-fable-5", false, 2000, "s2", "/u/beta"), None, "api").unwrap();
    }

    #[test]
    fn totals_and_unpriced() {
        let conn = crate::store::open_memory().unwrap();
        seed(&conn);
        let t = totals(&conn, None, None, None);
        assert_eq!(t.events, 3);
        assert_eq!(t.unpriced, 1);
        assert!((t.cost_usd - 1.2).abs() < 1e-9);
        assert_eq!(t.input, 300);
        assert_eq!(t.cache_write, 150);
    }

    #[test]
    fn totals_filters_by_time_and_project() {
        let conn = crate::store::open_memory().unwrap();
        seed(&conn);
        let t = totals(&conn, Some("2026-08-27 00:00:00"), None, None);
        assert_eq!(t.events, 1);
        let pid: i64 = conn.query_row("SELECT id FROM projects WHERE slug='-a'", [], |r| r.get(0)).unwrap();
        let t2 = totals(&conn, None, None, Some(pid));
        assert_eq!(t2.events, 2);
    }

    #[test]
    fn model_and_sidechain_split() {
        let conn = crate::store::open_memory().unwrap();
        seed(&conn);
        let ms = model_split(&conn, None, None);
        assert_eq!(ms.len(), 2);
        let (main, side) = sidechain_split(&conn, None, None);
        assert!((main - 1.0).abs() < 1e-9);
        assert!((side - 0.2).abs() < 1e-9);
    }

    #[test]
    fn project_session_event_drilldown() {
        let conn = crate::store::open_memory().unwrap();
        seed(&conn);
        let ps = project_rows(&conn);
        assert_eq!(ps.len(), 2);
        assert_eq!(ps[0].display_name, "alpha"); // cost 降序，alpha 1.2 > beta 0
        let ss = session_rows(&conn, ps[0].id);
        assert_eq!(ss.len(), 1);
        assert_eq!(ss[0].events, 2);
        assert!((ss[0].side_cost - 0.2).abs() < 1e-9);
        let evs = event_rows(&conn, ss[0].id);
        assert_eq!(evs.len(), 2);
    }

    #[test]
    fn current_context_is_latest() {
        let conn = crate::store::open_memory().unwrap();
        seed(&conn);
        let c = current_context(&conn).unwrap();
        assert_eq!(c.project_name, "beta");
        assert_eq!(c.model, "claude-fable-5");
    }

    #[test]
    fn burn_rate_zero_on_old_data() {
        let conn = crate::store::open_memory().unwrap();
        seed(&conn);
        assert_eq!(burn_rate_per_hour(&conn), 0.0); // 种子数据都在过去
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p bookholder-core queries`
Expected: 编译失败

- [ ] **Step 3: 实现 `queries.rs`**（SQL 全部给出）

```rust
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, Default)]
pub struct Totals {
    pub cost_usd: f64, pub input: i64, pub output: i64, pub thinking: i64,
    pub cache_read: i64, pub cache_write: i64, pub events: i64, pub unpriced: i64,
}

pub fn totals(conn: &Connection, from: Option<&str>, to: Option<&str>, project_id: Option<i64>) -> Totals {
    conn.query_row(
        "SELECT COALESCE(SUM(cost_usd),0), COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                COALESCE(SUM(thinking_tokens),0), COALESCE(SUM(cache_read),0),
                COALESCE(SUM(cache_write_5m + cache_write_1h),0), COUNT(*),
                COALESCE(SUM(CASE WHEN cost_usd IS NULL THEN 1 ELSE 0 END),0)
         FROM usage_events
         WHERE (?1 IS NULL OR ts >= ?1) AND (?2 IS NULL OR ts < ?2) AND (?3 IS NULL OR project_id = ?3)",
        params![from, to, project_id],
        |r| Ok(Totals {
            cost_usd: r.get(0)?, input: r.get(1)?, output: r.get(2)?, thinking: r.get(3)?,
            cache_read: r.get(4)?, cache_write: r.get(5)?, events: r.get(6)?, unpriced: r.get(7)?,
        }),
    ).unwrap_or_default()
}

#[derive(Debug, Clone, Serialize)]
pub struct DailyModelRow { pub date: String, pub model: String, pub cost_usd: f64 }

pub fn daily_by_model(conn: &Connection, days: i64) -> Vec<DailyModelRow> {
    let mut stmt = conn.prepare(
        "SELECT date(ts, 'localtime') d, model, COALESCE(SUM(cost_usd),0)
         FROM usage_events
         WHERE ts >= datetime('now', ?1)
         GROUP BY d, model ORDER BY d",
    ).unwrap();
    stmt.query_map([format!("-{days} days")], |r| Ok(DailyModelRow {
        date: r.get(0)?, model: r.get(1)?, cost_usd: r.get(2)?,
    })).map(|it| it.flatten().collect()).unwrap_or_default()
}

#[derive(Debug, Clone, Serialize)]
pub struct HourRow { pub hour: String, pub main_cost: f64, pub side_cost: f64 }

pub fn hourly_last24(conn: &Connection) -> Vec<HourRow> {
    let mut stmt = conn.prepare(
        "SELECT strftime('%Y-%m-%d %H:00', ts, 'localtime') h,
                COALESCE(SUM(CASE WHEN is_sidechain=0 THEN cost_usd ELSE 0 END),0),
                COALESCE(SUM(CASE WHEN is_sidechain=1 THEN cost_usd ELSE 0 END),0)
         FROM usage_events WHERE ts >= datetime('now', '-24 hours')
         GROUP BY h ORDER BY h",
    ).unwrap();
    stmt.query_map([], |r| Ok(HourRow { hour: r.get(0)?, main_cost: r.get(1)?, side_cost: r.get(2)? }))
        .map(|it| it.flatten().collect()).unwrap_or_default()
}

pub fn burn_rate_per_hour(conn: &Connection) -> f64 {
    let half_hour: f64 = conn.query_row(
        "SELECT COALESCE(SUM(cost_usd),0) FROM usage_events WHERE ts >= datetime('now','-30 minutes')",
        [], |r| r.get(0),
    ).unwrap_or(0.0);
    half_hour * 2.0
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelSplitRow { pub model: String, pub cost_usd: f64, pub input: i64, pub output: i64, pub events: i64 }

pub fn model_split(conn: &Connection, from: Option<&str>, to: Option<&str>) -> Vec<ModelSplitRow> {
    let mut stmt = conn.prepare(
        "SELECT model, COALESCE(SUM(cost_usd),0), COALESCE(SUM(input_tokens),0),
                COALESCE(SUM(output_tokens),0), COUNT(*)
         FROM usage_events WHERE (?1 IS NULL OR ts >= ?1) AND (?2 IS NULL OR ts < ?2)
         GROUP BY model ORDER BY 2 DESC",
    ).unwrap();
    stmt.query_map(params![from, to], |r| Ok(ModelSplitRow {
        model: r.get(0)?, cost_usd: r.get(1)?, input: r.get(2)?, output: r.get(3)?, events: r.get(4)?,
    })).map(|it| it.flatten().collect()).unwrap_or_default()
}

pub fn sidechain_split(conn: &Connection, from: Option<&str>, to: Option<&str>) -> (f64, f64) {
    conn.query_row(
        "SELECT COALESCE(SUM(CASE WHEN is_sidechain=0 THEN cost_usd ELSE 0 END),0),
                COALESCE(SUM(CASE WHEN is_sidechain=1 THEN cost_usd ELSE 0 END),0)
         FROM usage_events WHERE (?1 IS NULL OR ts >= ?1) AND (?2 IS NULL OR ts < ?2)",
        params![from, to], |r| Ok((r.get(0)?, r.get(1)?)),
    ).unwrap_or((0.0, 0.0))
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectRow {
    pub id: i64, pub display_name: String, pub cwd: String, pub cost_usd: f64,
    pub tokens: i64, pub sessions: i64, pub active_days: i64, pub last_seen: String,
}

pub fn project_rows(conn: &Connection) -> Vec<ProjectRow> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.display_name, p.cwd, COALESCE(SUM(e.cost_usd),0),
                COALESCE(SUM(e.input_tokens + e.output_tokens + e.cache_read + e.cache_write_5m + e.cache_write_1h),0),
                COUNT(DISTINCT e.session_id), COUNT(DISTINCT date(e.ts,'localtime')), COALESCE(p.last_seen,'')
         FROM projects p LEFT JOIN usage_events e ON e.project_id = p.id
         GROUP BY p.id ORDER BY 4 DESC",
    ).unwrap();
    stmt.query_map([], |r| Ok(ProjectRow {
        id: r.get(0)?, display_name: r.get(1)?, cwd: r.get(2)?, cost_usd: r.get(3)?,
        tokens: r.get(4)?, sessions: r.get(5)?, active_days: r.get(6)?, last_seen: r.get(7)?,
    })).map(|it| it.flatten().collect()).unwrap_or_default()
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionRow {
    pub id: i64, pub session_id: String, pub started_at: String, pub ended_at: String,
    pub billing_mode: String, pub cost_usd: f64, pub events: i64, pub side_cost: f64,
}

pub fn session_rows(conn: &Connection, project_id: i64) -> Vec<SessionRow> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.session_id, COALESCE(s.started_at,''), COALESCE(s.ended_at,''),
                COALESCE(s.billing_mode,'unknown'), COALESCE(SUM(e.cost_usd),0), COUNT(e.id),
                COALESCE(SUM(CASE WHEN e.is_sidechain=1 THEN e.cost_usd ELSE 0 END),0)
         FROM sessions s LEFT JOIN usage_events e ON e.session_id = s.id
         WHERE s.project_id = ?1 GROUP BY s.id ORDER BY s.started_at DESC",
    ).unwrap();
    stmt.query_map([project_id], |r| Ok(SessionRow {
        id: r.get(0)?, session_id: r.get(1)?, started_at: r.get(2)?, ended_at: r.get(3)?,
        billing_mode: r.get(4)?, cost_usd: r.get(5)?, events: r.get(6)?, side_cost: r.get(7)?,
    })).map(|it| it.flatten().collect()).unwrap_or_default()
}

#[derive(Debug, Clone, Serialize)]
pub struct EventRow {
    pub ts: String, pub model: String, pub is_sidechain: bool, pub input: i64, pub output: i64,
    pub thinking: i64, pub cache_write_5m: i64, pub cache_write_1h: i64, pub cache_read: i64,
    pub cost_usd: Option<f64>,
}

pub fn event_rows(conn: &Connection, session_pk: i64) -> Vec<EventRow> {
    let mut stmt = conn.prepare(
        "SELECT ts, model, is_sidechain, input_tokens, output_tokens, thinking_tokens,
                cache_write_5m, cache_write_1h, cache_read, cost_usd
         FROM usage_events WHERE session_id = ?1 ORDER BY ts",
    ).unwrap();
    stmt.query_map([session_pk], |r| Ok(EventRow {
        ts: r.get(0)?, model: r.get(1)?, is_sidechain: r.get::<_, i64>(2)? != 0,
        input: r.get(3)?, output: r.get(4)?, thinking: r.get(5)?,
        cache_write_5m: r.get(6)?, cache_write_1h: r.get(7)?, cache_read: r.get(8)?,
        cost_usd: r.get(9)?,
    })).map(|it| it.flatten().collect()).unwrap_or_default()
}

#[derive(Debug, Clone, Serialize)]
pub struct CurrentCtx { pub project_id: i64, pub project_name: String, pub model: String, pub last_ts: String }

pub fn current_context(conn: &Connection) -> Option<CurrentCtx> {
    conn.query_row(
        "SELECT e.project_id, p.display_name, e.model, e.ts
         FROM usage_events e JOIN projects p ON p.id = e.project_id
         ORDER BY e.ts DESC LIMIT 1",
        [], |r| Ok(CurrentCtx { project_id: r.get(0)?, project_name: r.get(1)?, model: r.get(2)?, last_ts: r.get(3)? }),
    ).ok()
}

pub fn today_range() -> (String, String) {
    use chrono::{Local, Utc, TimeZone};
    let today = Local::now().date_naive();
    let start = Local.from_local_datetime(&today.and_hms_opt(0, 0, 0).unwrap()).unwrap();
    let end = start + chrono::Duration::days(1);
    (start.with_timezone(&Utc).format("%Y-%m-%d %H:%M:%S").to_string(),
     end.with_timezone(&Utc).format("%Y-%m-%d %H:%M:%S").to_string())
}
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p bookholder-core`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): aggregation queries (totals, splits, drilldown, burn rate)"
```

---

### Task 9: 报告导出（Markdown / CSV / JSON）

**Files:**
- Create: `crates/core/src/report.rs`
- Modify: `crates/core/src/lib.rs`（加 `pub mod report;`）

**Interfaces:**
- Consumes: `queries::*`
- Produces（`report::`）:
  - `markdown_report(conn) -> String`（总计 + 项目表 + 模型表 + 主/subagent 对比 + 口径与未计价说明）
  - `csv_events(conn, project_id: Option<i64>) -> String`（表头 + 全事件行）
  - `json_report(conn) -> String`（`{"totals":…,"projects":…,"models":…}` serde_json 序列化）

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::UsageEvent;

    fn seed(conn: &rusqlite::Connection) {
        let e = UsageEvent {
            dedup_key: "k1".into(), ts: "2026-08-26T02:00:00Z".into(), session_id: "s1".into(),
            cwd: "/u/alpha".into(), model: "claude-fable-5".into(), is_sidechain: false,
            input_tokens: 100, output_tokens: 200, thinking_tokens: 0,
            cache_write_5m: 0, cache_write_1h: 0, cache_read: 0,
        };
        crate::store::record_event(conn, "-a", &e, Some(1.5), "subscription").unwrap();
    }

    #[test]
    fn markdown_contains_key_sections() {
        let conn = crate::store::open_memory().unwrap();
        seed(&conn);
        let md = markdown_report(&conn);
        assert!(md.contains("# Bookholder"));
        assert!(md.contains("alpha"));
        assert!(md.contains("claude-fable-5"));
        assert!(md.contains("$1.5"));
    }

    #[test]
    fn csv_has_header_and_rows() {
        let conn = crate::store::open_memory().unwrap();
        seed(&conn);
        let csv = csv_events(&conn, None);
        let lines: Vec<&str> = csv.trim().lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("ts,project,session,model,is_sidechain,input"));
        assert!(lines[1].contains("claude-fable-5"));
    }

    #[test]
    fn json_parses_back() {
        let conn = crate::store::open_memory().unwrap();
        seed(&conn);
        let v: serde_json::Value = serde_json::from_str(&json_report(&conn)).unwrap();
        assert!(v["totals"]["events"].as_i64().unwrap() == 1);
        assert!(v["projects"].as_array().unwrap().len() == 1);
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p bookholder-core report`
Expected: 编译失败

- [ ] **Step 3: 实现 `report.rs`**

```rust
use crate::queries;
use rusqlite::Connection;
use std::fmt::Write;

pub fn markdown_report(conn: &Connection) -> String {
    let t = queries::totals(conn, None, None, None);
    let projects = queries::project_rows(conn);
    let models = queries::model_split(conn, None, None);
    let (main, side) = queries::sidechain_split(conn, None, None);
    let mut md = String::new();
    let _ = writeln!(md, "# Bookholder 成本报告\n");
    let _ = writeln!(md, "总成本 **${:.4}** ｜ 请求 {} 次 ｜ input {} / output {} / cache 读 {} / cache 写 {}",
        t.cost_usd, t.events, t.input, t.output, t.cache_read, t.cache_write);
    if t.unpriced > 0 {
        let _ = writeln!(md, "\n> ⚠️ {} 条事件因模型价格未知未计价，未计入总成本。", t.unpriced);
    }
    let _ = writeln!(md, "\n主对话 ${main:.4} ｜ subagent ${side:.4}\n");
    let _ = writeln!(md, "## 项目\n\n| 项目 | 成本 | tokens | 会话 | 活跃天数 |\n|---|---|---|---|---|");
    for p in &projects {
        let _ = writeln!(md, "| {} | ${:.4} | {} | {} | {} |", p.display_name, p.cost_usd, p.tokens, p.sessions, p.active_days);
    }
    let _ = writeln!(md, "\n## 模型\n\n| 模型 | 成本 | input | output | 请求数 |\n|---|---|---|---|---|");
    for m in &models {
        let _ = writeln!(md, "| {} | ${:.4} | {} | {} | {} |", m.model, m.cost_usd, m.input, m.output, m.events);
    }
    md
}

pub fn csv_events(conn: &Connection, project_id: Option<i64>) -> String {
    let mut out = String::from("ts,project,session,model,is_sidechain,input,output,thinking,cache_write_5m,cache_write_1h,cache_read,cost_usd\n");
    let mut stmt = conn.prepare(
        "SELECT e.ts, p.display_name, s.session_id, e.model, e.is_sidechain,
                e.input_tokens, e.output_tokens, e.thinking_tokens,
                e.cache_write_5m, e.cache_write_1h, e.cache_read, e.cost_usd
         FROM usage_events e
         JOIN projects p ON p.id = e.project_id
         JOIN sessions s ON s.id = e.session_id
         WHERE (?1 IS NULL OR e.project_id = ?1) ORDER BY e.ts",
    ).unwrap();
    let rows = stmt.query_map(rusqlite::params![project_id], |r| {
        let cost: Option<f64> = r.get(11)?;
        Ok(format!(
            "{},{},{},{},{},{},{},{},{},{},{},{}",
            r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?,
            r.get::<_, i64>(4)?, r.get::<_, i64>(5)?, r.get::<_, i64>(6)?, r.get::<_, i64>(7)?,
            r.get::<_, i64>(8)?, r.get::<_, i64>(9)?, r.get::<_, i64>(10)?,
            cost.map(|c| format!("{c:.6}")).unwrap_or_default()
        ))
    });
    if let Ok(rows) = rows {
        for row in rows.flatten() {
            out.push_str(&row);
            out.push('\n');
        }
    }
    out
}

pub fn json_report(conn: &Connection) -> String {
    serde_json::json!({
        "totals": queries::totals(conn, None, None, None),
        "projects": queries::project_rows(conn),
        "models": queries::model_split(conn, None, None),
    }).to_string()
}
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p bookholder-core`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): markdown/csv/json report generation"
```

---

### Task 10: CLI（可选附加层）

**Files:**
- Modify: `crates/cli/Cargo.toml`
- Modify: `crates/cli/src/main.rs`

**Interfaces:**
- Consumes: core 全部公开 API
- Produces: `bookholder report [--json|--csv|--md] [--project <name>]`、`bookholder backfill`、`bookholder live`

- [ ] **Step 1: 写失败测试**（`crates/cli/tests/cli.rs`；Cargo.toml dev-dependencies 加 `assert_cmd = "2"`、`predicates = "3"`、`tempfile = "3"`）

```rust
use assert_cmd::Command;

#[test]
fn report_json_on_empty_db() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("t.db");
    let claude = dir.path().join("projects");
    std::fs::create_dir_all(&claude).unwrap();
    Command::cargo_bin("bookholder").unwrap()
        .env("BOOKHOLDER_DB", &db)
        .env("BOOKHOLDER_CLAUDE_DIR", &claude)
        .args(["report", "--json"])
        .assert()
        .success()
        .stdout(predicates::str::contains("\"totals\""));
}

#[test]
fn backfill_ingests_fixture() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("t.db");
    let proj = dir.path().join("projects/-u-alpha");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(proj.join("s.jsonl"),
        r#"{"type":"assistant","uuid":"u1","requestId":"r1","sessionId":"s1","cwd":"/u/alpha","isSidechain":false,"timestamp":"2026-08-27T01:00:00Z","message":{"id":"m1","model":"claude-fable-5","usage":{"input_tokens":10,"output_tokens":20}}}
"#).unwrap();
    Command::cargo_bin("bookholder").unwrap()
        .env("BOOKHOLDER_DB", &db)
        .env("BOOKHOLDER_CLAUDE_DIR", dir.path().join("projects"))
        .arg("backfill")
        .assert()
        .success()
        .stdout(predicates::str::contains("added: 1"));
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p bookholder`
Expected: FAIL（子命令未实现）

- [ ] **Step 3: 实现**

`crates/cli/Cargo.toml` dependencies 加 `clap = { version = "4", features = ["derive"] }`、`serde_json = { workspace = true }`。

`crates/cli/src/main.rs`:

```rust
use bookholder_core::{billing, ingest, pricing, queries, report, store, watcher};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "bookholder", about = "Claude Code token 成本统计（GUI 的可选命令行入口）")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 输出统计报告
    Report {
        #[arg(long)] json: bool,
        #[arg(long)] csv: bool,
        #[arg(long)] md: bool,
        /// 按项目显示名过滤（仅 csv 生效）
        #[arg(long)] project: Option<String>,
    },
    /// 全量重扫转录目录
    Backfill,
    /// 实时输出新事件（JSON 行）
    Live,
}

fn open() -> rusqlite::Connection {
    let conn = store::open_db(&store::default_db_path()).expect("open db");
    let _ = pricing::seed_snapshot(&conn);
    conn
}

fn billing_mode(conn: &rusqlite::Connection) -> String {
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
    billing::effective_mode(conn, &home)
}

fn now_utc() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn main() {
    let cli = Cli::parse();
    let conn = open();
    let mode = billing_mode(&conn);
    match cli.cmd {
        Cmd::Report { json, csv, project, .. } => {
            // 报告前先增量扫一遍，保证数字最新
            ingest::scan_all(&conn, &store::claude_projects_dir(), &mode);
            let _ = pricing::reprice_null_costs(&conn);
            if json {
                println!("{}", report::json_report(&conn));
            } else if csv {
                let pid = project.and_then(|name| {
                    queries::project_rows(&conn).into_iter().find(|p| p.display_name == name).map(|p| p.id)
                });
                print!("{}", report::csv_events(&conn, pid));
            } else {
                print!("{}", report::markdown_report(&conn));
            }
        }
        Cmd::Backfill => {
            let st = ingest::scan_all(&conn, &store::claude_projects_dir(), &mode);
            let _ = pricing::reprice_null_costs(&conn);
            println!("added: {} skipped: {} bad: {}", st.added, st.skipped, st.bad);
        }
        Cmd::Live => {
            let dir = store::claude_projects_dir();
            ingest::scan_all(&conn, &dir, &mode);
            let mut last_id: i64 = conn
                .query_row("SELECT COALESCE(MAX(id),0) FROM usage_events", [], |r| r.get(0))
                .unwrap_or(0);
            let db_path = store::default_db_path();
            let mode2 = mode.clone();
            let _guard = watcher::watch_projects(&dir, move || {
                let Ok(c) = store::open_db(&db_path) else { return };
                ingest::scan_all(&c, &store::claude_projects_dir(), &mode2);
                let mut stmt = c.prepare(
                    "SELECT id, ts, model, is_sidechain, input_tokens, output_tokens, cost_usd
                     FROM usage_events WHERE id > ?1 ORDER BY id").unwrap();
                let rows: Vec<(i64, String, String, i64, i64, i64, Option<f64>)> = stmt
                    .query_map([last_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))
                    .unwrap().flatten().collect();
                for (id, ts, model, side, input, output, cost) in rows {
                    last_id = id;
                    println!("{}", serde_json::json!({
                        "ts": ts, "model": model, "is_sidechain": side != 0,
                        "input": input, "output": output, "cost_usd": cost
                    }));
                }
            }).expect("watch");
            eprintln!("watching {} … Ctrl-C 退出", dir.display());
            loop { std::thread::sleep(std::time::Duration::from_secs(3600)); }
        }
    }
}
```

（`now_utc` 供后续价格刷新用；若编译器警告未使用，在 Report 分支价格刷新逻辑并入 Task 12 的 GUI，CLI 保持不刷价格——删掉该函数即可。）

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p bookholder`
Expected: PASS（2 个测试）

- [ ] **Step 5: 手动验证真实数据**

Run: `cargo run -p bookholder -- backfill && cargo run -p bookholder -- report | head -30`
Expected: 输出真实的项目成本表（本机 ~/.claude/projects 有数据）

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat(cli): report/backfill/live subcommands"
```

---

### Task 11: Tauri 应用脚手架（双窗口 + 前端工程 + 图标）

**Files:**
- Create: `app/ui/package.json`, `app/ui/vite.config.ts`, `app/ui/tsconfig.json`
- Create: `app/ui/index.html`, `app/ui/float.html`, `app/ui/src/main.ts`, `app/ui/src/float.ts`, `app/ui/src/style.css`
- Create: `app/src-tauri/Cargo.toml`, `app/src-tauri/tauri.conf.json`, `app/src-tauri/build.rs`, `app/src-tauri/src/main.rs`, `app/src-tauri/capabilities/default.json`
- Create: `app/src-tauri/icons/*`（生成）
- Modify: 根 `Cargo.toml`（members 加 `"app/src-tauri"`）

**Interfaces:**
- Produces: 可运行的 Tauri 应用，启动时出现悬浮窗（260×150 无边框置顶）和隐藏的主窗口；`npm --prefix app/ui run tauri:dev` 可开发调试。

- [ ] **Step 1: 前端工程**

`app/ui/package.json`:

```json
{
  "name": "bookholder-ui",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "tauri:dev": "tauri dev",
    "tauri:build": "tauri build"
  },
  "dependencies": {
    "@tauri-apps/api": "^2",
    "@tauri-apps/plugin-dialog": "^2",
    "@tauri-apps/plugin-autostart": "^2",
    "echarts": "^5"
  },
  "devDependencies": {
    "@tauri-apps/cli": "^2",
    "typescript": "^5",
    "vite": "^6"
  }
}
```

`app/ui/vite.config.ts`:

```ts
import { defineConfig } from "vite";
import { resolve } from "path";

export default defineConfig({
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: {
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
        float: resolve(__dirname, "float.html"),
      },
    },
  },
});
```

`app/ui/tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "skipLibCheck": true
  },
  "include": ["src"]
}
```

`app/ui/index.html`:

```html
<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="UTF-8" />
  <title>Bookholder</title>
  <link rel="stylesheet" href="/src/style.css" />
</head>
<body>
  <div id="app">Bookholder 面板加载中…</div>
  <script type="module" src="/src/main.ts"></script>
</body>
</html>
```

`app/ui/float.html`:

```html
<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="UTF-8" />
  <title>Bookholder Float</title>
  <link rel="stylesheet" href="/src/style.css" />
</head>
<body class="float-body" data-tauri-drag-region>
  <div id="float">…</div>
  <script type="module" src="/src/float.ts"></script>
</body>
</html>
```

`app/ui/src/main.ts` 与 `app/ui/src/float.ts` 本任务先占位（Task 13/14 实现）：

```ts
document.getElementById("app")!.textContent = "Bookholder";
```

```ts
document.getElementById("float")!.textContent = "Bookholder float";
```

`app/ui/src/style.css`（基础）:

```css
* { margin: 0; box-sizing: border-box; }
body { font-family: -apple-system, "PingFang SC", sans-serif; background: #14161c; color: #e8eaf0; }
.float-body { background: rgba(20, 22, 28, 0.92); border-radius: 12px; overflow: hidden; height: 100vh; }
```

- [ ] **Step 2: 生成图标**（纯 stdlib Python 生成 RGBA PNG，再用 tauri icon 转全套）

```bash
python3 - <<'EOF'
import zlib, struct
w = h = 1024
row = b'\x00' + bytes([79, 109, 245, 255]) * w
raw = row * h
def chunk(t, d):
    return struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d))
png = (b'\x89PNG\r\n\x1a\n'
       + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0))
       + chunk(b'IDAT', zlib.compress(raw))
       + chunk(b'IEND', b''))
open('app/icon.png', 'wb').write(png)
EOF
cd app/ui && npm install && npx tauri icon ../icon.png -o ../src-tauri/icons && cd ../..
```

- [ ] **Step 3: Tauri 后端**

`app/src-tauri/Cargo.toml`:

```toml
[package]
name = "bookholder-app"
version = "0.1.0"
edition = "2021"

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = ["tray-icon"] }
tauri-plugin-dialog = "2"
tauri-plugin-autostart = "2"
bookholder-core = { path = "../../crates/core" }
serde = { workspace = true }
serde_json = { workspace = true }
chrono = { workspace = true }
rusqlite = { workspace = true }
```

`app/src-tauri/build.rs`:

```rust
fn main() {
    tauri_build::build()
}
```

`app/src-tauri/tauri.conf.json`:

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Bookholder",
  "version": "0.1.0",
  "identifier": "dev.hou.bookholder",
  "build": {
    "beforeDevCommand": "npm --prefix ../ui run dev",
    "devUrl": "http://localhost:1420",
    "beforeBuildCommand": "npm --prefix ../ui run build",
    "frontendDist": "../ui/dist"
  },
  "app": {
    "windows": [
      {
        "label": "float",
        "url": "float.html",
        "width": 260,
        "height": 150,
        "alwaysOnTop": true,
        "decorations": false,
        "resizable": false,
        "skipTaskbar": true,
        "transparent": true
      },
      {
        "label": "main",
        "url": "index.html",
        "title": "Bookholder",
        "width": 1000,
        "height": 700,
        "visible": false
      }
    ]
  },
  "bundle": {
    "active": true,
    "targets": ["app"],
    "icon": ["icons/icon.icns", "icons/icon.png"]
  }
}
```

`app/src-tauri/capabilities/default.json`:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "windows": ["main", "float"],
  "permissions": [
    "core:default",
    "core:window:allow-show",
    "core:window:allow-hide",
    "core:window:allow-set-focus",
    "core:window:allow-start-dragging",
    "dialog:default",
    "autostart:default"
  ]
}
```

`app/src-tauri/src/main.rs`（本任务最小可运行版，Task 12 扩展）:

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .run(tauri::generate_context!())
        .expect("error while running bookholder");
}
```

根 `Cargo.toml` members 改为 `["crates/core", "crates/cli", "app/src-tauri"]`。

- [ ] **Step 4: 验证**

Run: `cargo check -p bookholder-app`
Expected: 编译通过

Run: `cd app/ui && npm run tauri:dev`（手动烟测，看到两个窗口——悬浮窗显示 "Bookholder float"；主窗口默认隐藏属正常，Ctrl-C 退出）
Expected: 悬浮窗出现在屏幕上、置顶、无边框

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(app): tauri scaffold with float + main windows"
```

---

### Task 12: Tauri commands + 后台采集线程 + 实时事件

**Files:**
- Create: `app/src-tauri/src/commands.rs`
- Modify: `app/src-tauri/src/main.rs`

**Interfaces:**
- Consumes: core 全部模块
- Produces（前端可 `invoke` 的命令，全部返回 JSON）:
  - `float_data() -> { today_cost, project_cost, project_name, burn_rate, model, billing_mode, hourly: HourRow[] }`
  - `overview() -> { today, week, month, all: Totals, daily: DailyModelRow[], models: ModelSplitRow[], main_cost, side_cost }`
  - `projects_list() -> ProjectRow[]`
  - `project_sessions(project_id: i64) -> SessionRow[]`
  - `session_events(session_pk: i64) -> EventRow[]`
  - `settings_status() -> { prices_last_fetch, prices_last_status, price_count, billing_mode, billing_override, skip_lines, bad_lines, db_path }`
  - `refresh_prices() -> Result<String, String>`（返回 "updated N models"）
  - `run_backfill() -> IngestStats`
  - `export_report(kind: String, dest: String) -> Result<(), String>`（kind: "md"|"csv"|"json"，写文件）
  - `set_billing_override(mode: String) -> ()`（"subscription"|"api"|""=清除）
  - `open_dashboard() -> ()`（show + focus 主窗口）
  - 事件：后台线程 ingest 到新数据时 `app.emit("usage-updated", stats)`
- 后台线程：启动即 `seed_snapshot` → `scan_all` → `reprice_null_costs` → 每日一次 `refresh_from_network`（间隔检查）→ `watch_projects` 循环。

- [ ] **Step 1: 实现 `commands.rs`**

```rust
use bookholder_core::{billing, ingest, pricing, queries, report, store};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};

pub struct Db(pub Mutex<Connection>);

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

fn now_utc() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

#[tauri::command]
pub fn float_data(db: State<Db>) -> Value {
    let conn = db.0.lock().unwrap();
    let (t0, t1) = queries::today_range();
    let today = queries::totals(&conn, Some(&t0), Some(&t1), None);
    let ctx = queries::current_context(&conn);
    let project_cost = ctx.as_ref()
        .map(|c| queries::totals(&conn, None, None, Some(c.project_id)).cost_usd)
        .unwrap_or(0.0);
    json!({
        "today_cost": today.cost_usd,
        "project_cost": project_cost,
        "project_name": ctx.as_ref().map(|c| c.project_name.clone()).unwrap_or_else(|| "—".into()),
        "model": ctx.as_ref().map(|c| c.model.clone()).unwrap_or_default(),
        "burn_rate": queries::burn_rate_per_hour(&conn),
        "billing_mode": billing::effective_mode(&conn, &home()),
        "hourly": queries::hourly_last24(&conn),
    })
}

#[tauri::command]
pub fn overview(db: State<Db>) -> Value {
    let conn = db.0.lock().unwrap();
    let (t0, t1) = queries::today_range();
    let (main_cost, side_cost) = queries::sidechain_split(&conn, None, None);
    json!({
        "today": queries::totals(&conn, Some(&t0), Some(&t1), None),
        "week": queries::totals(&conn, Some(&ago_days(7)), None, None),
        "month": queries::totals(&conn, Some(&ago_days(30)), None, None),
        "all": queries::totals(&conn, None, None, None),
        "daily": queries::daily_by_model(&conn, 30),
        "models": queries::model_split(&conn, None, None),
        "main_cost": main_cost,
        "side_cost": side_cost,
    })
}

fn ago_days(d: i64) -> String {
    (chrono::Utc::now() - chrono::Duration::days(d)).format("%Y-%m-%d %H:%M:%S").to_string()
}

#[tauri::command]
pub fn projects_list(db: State<Db>) -> Value {
    let conn = db.0.lock().unwrap();
    serde_json::to_value(queries::project_rows(&conn)).unwrap_or(Value::Null)
}

#[tauri::command]
pub fn project_sessions(db: State<Db>, project_id: i64) -> Value {
    let conn = db.0.lock().unwrap();
    serde_json::to_value(queries::session_rows(&conn, project_id)).unwrap_or(Value::Null)
}

#[tauri::command]
pub fn session_events(db: State<Db>, session_pk: i64) -> Value {
    let conn = db.0.lock().unwrap();
    serde_json::to_value(queries::event_rows(&conn, session_pk)).unwrap_or(Value::Null)
}

#[tauri::command]
pub fn settings_status(db: State<Db>) -> Value {
    let conn = db.0.lock().unwrap();
    let price_count: i64 = conn
        .query_row("SELECT COUNT(DISTINCT model) FROM model_prices", [], |r| r.get(0))
        .unwrap_or(0);
    json!({
        "prices_last_fetch": store::meta_get(&conn, "prices_last_fetch"),
        "prices_last_status": store::meta_get(&conn, "prices_last_status"),
        "price_count": price_count,
        "billing_mode": billing::effective_mode(&conn, &home()),
        "billing_override": store::meta_get(&conn, "billing_override"),
        "skip_lines": store::meta_get(&conn, "skip_lines"),
        "bad_lines": store::meta_get(&conn, "bad_lines"),
        "db_path": store::default_db_path().to_string_lossy(),
    })
}

#[tauri::command]
pub fn refresh_prices(db: State<Db>) -> Result<String, String> {
    let conn = db.0.lock().unwrap();
    let n = pricing::refresh_from_network(&conn, &now_utc())?;
    let re = pricing::reprice_null_costs(&conn).map_err(|e| e.to_string())?;
    Ok(format!("updated {n} models, repriced {re} events"))
}

#[tauri::command]
pub fn run_backfill(db: State<Db>) -> Value {
    let conn = db.0.lock().unwrap();
    let mode = billing::effective_mode(&conn, &home());
    let st = ingest::scan_all(&conn, &store::claude_projects_dir(), &mode);
    let _ = pricing::reprice_null_costs(&conn);
    serde_json::to_value(st).unwrap_or(Value::Null)
}

#[tauri::command]
pub fn export_report(db: State<Db>, kind: String, dest: String) -> Result<(), String> {
    let conn = db.0.lock().unwrap();
    let content = match kind.as_str() {
        "md" => report::markdown_report(&conn),
        "csv" => report::csv_events(&conn, None),
        "json" => report::json_report(&conn),
        _ => return Err(format!("unknown kind {kind}")),
    };
    std::fs::write(&dest, content).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_billing_override(db: State<Db>, mode: String) -> Result<(), String> {
    let conn = db.0.lock().unwrap();
    if mode.is_empty() {
        conn.execute("DELETE FROM meta WHERE key='billing_override'", [])
            .map_err(|e| e.to_string())?;
    } else {
        store::meta_set(&conn, "billing_override", &mode).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn open_dashboard(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}
```

- [ ] **Step 2: 改写 `main.rs`（注册命令、状态、后台线程、托盘、主窗口关闭改隐藏）**

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use bookholder_core::{billing, ingest, pricing, store, watcher};
use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager,
};

fn main() {
    let conn = store::open_db(&store::default_db_path()).expect("open db");
    let _ = pricing::seed_snapshot(&conn);

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(commands::Db(Mutex::new(conn)))
        .invoke_handler(tauri::generate_handler![
            commands::float_data,
            commands::overview,
            commands::projects_list,
            commands::project_sessions,
            commands::session_events,
            commands::settings_status,
            commands::refresh_prices,
            commands::run_backfill,
            commands::export_report,
            commands::set_billing_override,
            commands::open_dashboard,
        ])
        .setup(|app| {
            // 托盘
            let show = MenuItem::with_id(app, "show", "打开面板", true, None::<&str>)?;
            let float = MenuItem::with_id(app, "float", "显示/隐藏悬浮窗", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &float, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "float" => {
                        if let Some(w) = app.get_webview_window("float") {
                            if w.is_visible().unwrap_or(false) { let _ = w.hide(); } else { let _ = w.show(); }
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // 后台采集线程：初扫 + 监视 + 每日价格刷新
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let db_path = store::default_db_path();
                let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
                let Ok(conn) = store::open_db(&db_path) else { return };
                let mode = billing::effective_mode(&conn, &home);
                let st = ingest::scan_all(&conn, &store::claude_projects_dir(), &mode);
                let _ = pricing::reprice_null_costs(&conn);
                let _ = handle.emit("usage-updated", &st);
                maybe_refresh_prices(&conn);

                let handle2 = handle.clone();
                let db_path2 = db_path.clone();
                let guard = watcher::watch_projects(&store::claude_projects_dir(), move || {
                    let Ok(c) = store::open_db(&db_path2) else { return };
                    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
                    let mode = billing::effective_mode(&c, &home);
                    let st = ingest::scan_all(&c, &store::claude_projects_dir(), &mode);
                    if st.added > 0 {
                        let _ = pricing::reprice_null_costs(&c);
                        let _ = handle2.emit("usage-updated", &st);
                    }
                });
                match guard {
                    Ok(_g) => loop {
                        std::thread::sleep(std::time::Duration::from_secs(3600));
                        if let Ok(c) = store::open_db(&db_path) {
                            maybe_refresh_prices(&c);
                        }
                    },
                    Err(e) => eprintln!("watcher failed: {e}"),
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // 主窗口点关闭 = 隐藏，应用常驻
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running bookholder");
}

fn maybe_refresh_prices(conn: &rusqlite::Connection) {
    let now = chrono::Utc::now();
    let stale = store::meta_get(conn, "prices_last_fetch")
        .and_then(|s| chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S").ok())
        .map(|t| (now.naive_utc() - t).num_hours() >= 24)
        .unwrap_or(true);
    if stale {
        let ts = now.format("%Y-%m-%d %H:%M:%S").to_string();
        if let Err(e) = pricing::refresh_from_network(conn, &ts) {
            let _ = store::meta_set(conn, "prices_last_status", &format!("error: {e}"));
        }
    }
}
```

- [ ] **Step 3: 验证**

Run: `cargo check -p bookholder-app && cargo test`
Expected: 全部编译通过、core/cli 测试仍 PASS

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(app): tauri commands, background ingest thread, tray, live events"
```

---

### Task 13: 前端 API 层 + 悬浮窗 UI

**Files:**
- Create: `app/ui/src/api.ts`
- Modify: `app/ui/src/float.ts`, `app/ui/src/style.css`, `app/ui/float.html`

**Interfaces:**
- Consumes: Task 12 的命令
- Produces: `api.ts` 导出（供 Task 14–16 复用）:
  - `type Totals / DailyModelRow / HourRow / ModelSplitRow / ProjectRow / SessionRow / EventRow / FloatData / Overview / SettingsStatus`（字段与 Rust serde 输出一致，snake_case）
  - `const api = { floatData(), overview(), projects(), sessions(id), events(id), settings(), refreshPrices(), backfill(), exportReport(kind, dest), setBillingOverride(mode), openDashboard() }`（全部 `invoke` 包装）
  - `onUsageUpdated(cb: () => void): void`（listen 包装）
  - `fmtUsd(n: number): string`（`$0.0000`，>1 时两位小数）、`fmtTok(n: number): string`（1.2k / 3.4M）

- [ ] **Step 1: 实现 `api.ts`**

```ts
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface Totals {
  cost_usd: number; input: number; output: number; thinking: number;
  cache_read: number; cache_write: number; events: number; unpriced: number;
}
export interface DailyModelRow { date: string; model: string; cost_usd: number }
export interface HourRow { hour: string; main_cost: number; side_cost: number }
export interface ModelSplitRow { model: string; cost_usd: number; input: number; output: number; events: number }
export interface ProjectRow {
  id: number; display_name: string; cwd: string; cost_usd: number;
  tokens: number; sessions: number; active_days: number; last_seen: string;
}
export interface SessionRow {
  id: number; session_id: string; started_at: string; ended_at: string;
  billing_mode: string; cost_usd: number; events: number; side_cost: number;
}
export interface EventRow {
  ts: string; model: string; is_sidechain: boolean; input: number; output: number;
  thinking: number; cache_write_5m: number; cache_write_1h: number; cache_read: number;
  cost_usd: number | null;
}
export interface FloatData {
  today_cost: number; project_cost: number; project_name: string; model: string;
  burn_rate: number; billing_mode: string; hourly: HourRow[];
}
export interface Overview {
  today: Totals; week: Totals; month: Totals; all: Totals;
  daily: DailyModelRow[]; models: ModelSplitRow[]; main_cost: number; side_cost: number;
}
export interface SettingsStatus {
  prices_last_fetch: string | null; prices_last_status: string | null; price_count: number;
  billing_mode: string; billing_override: string | null;
  skip_lines: string | null; bad_lines: string | null; db_path: string;
}

export const api = {
  floatData: () => invoke<FloatData>("float_data"),
  overview: () => invoke<Overview>("overview"),
  projects: () => invoke<ProjectRow[]>("projects_list"),
  sessions: (projectId: number) => invoke<SessionRow[]>("project_sessions", { projectId }),
  events: (sessionPk: number) => invoke<EventRow[]>("session_events", { sessionPk }),
  settings: () => invoke<SettingsStatus>("settings_status"),
  refreshPrices: () => invoke<string>("refresh_prices"),
  backfill: () => invoke<{ added: number; skipped: number; bad: number }>("run_backfill"),
  exportReport: (kind: string, dest: string) => invoke<void>("export_report", { kind, dest }),
  setBillingOverride: (mode: string) => invoke<void>("set_billing_override", { mode }),
  openDashboard: () => invoke<void>("open_dashboard"),
};

export function onUsageUpdated(cb: () => void): void {
  void listen("usage-updated", cb);
}

export function fmtUsd(n: number): string {
  return n >= 1 ? `$${n.toFixed(2)}` : `$${n.toFixed(4)}`;
}

export function fmtTok(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)}B`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)}k`;
  return String(n);
}
```

- [ ] **Step 2: 悬浮窗**

`app/ui/float.html` body 换为：

```html
<body class="float-body">
  <div id="float" data-tauri-drag-region>
    <div class="f-top" data-tauri-drag-region>
      <span id="f-project">—</span>
      <span id="f-badge" class="badge">…</span>
      <span id="f-model" class="dim"></span>
    </div>
    <div class="f-nums" data-tauri-drag-region>
      <div><label>今日</label><b id="f-today">$0</b></div>
      <div><label>本项目</label><b id="f-proj">$0</b></div>
      <div><label>燃烧率/时</label><b id="f-burn">$0</b></div>
    </div>
    <div id="f-spark" class="spark" data-tauri-drag-region></div>
  </div>
  <script type="module" src="/src/float.ts"></script>
</body>
```

`app/ui/src/float.ts`:

```ts
import { api, fmtUsd, onUsageUpdated, FloatData } from "./api";

function el(id: string): HTMLElement {
  return document.getElementById(id)!;
}

function shortModel(m: string): string {
  return m.replace(/^claude-/, "").replace(/-\d{8}$/, "");
}

function renderSpark(hourly: FloatData["hourly"]): void {
  const spark = el("f-spark");
  spark.innerHTML = "";
  const max = Math.max(...hourly.map((h) => h.main_cost + h.side_cost), 1e-9);
  for (const h of hourly) {
    const col = document.createElement("div");
    col.className = "spark-col";
    const side = document.createElement("div");
    side.className = "spark-side";
    side.style.height = `${((h.side_cost / max) * 100).toFixed(1)}%`;
    const main = document.createElement("div");
    main.className = "spark-main";
    main.style.height = `${((h.main_cost / max) * 100).toFixed(1)}%`;
    col.append(side, main);
    col.title = `${h.hour}  主 ${fmtUsd(h.main_cost)} / 子 ${fmtUsd(h.side_cost)}`;
    spark.appendChild(col);
  }
}

async function refresh(): Promise<void> {
  const d = await api.floatData();
  el("f-project").textContent = d.project_name;
  el("f-model").textContent = shortModel(d.model);
  const badge = el("f-badge");
  badge.textContent = d.billing_mode === "subscription" ? "订阅" : d.billing_mode === "api" ? "API" : "?";
  badge.className = `badge badge-${d.billing_mode}`;
  el("f-today").textContent = fmtUsd(d.today_cost);
  el("f-proj").textContent = fmtUsd(d.project_cost);
  el("f-burn").textContent = fmtUsd(d.burn_rate);
  renderSpark(d.hourly);
}

document.body.addEventListener("dblclick", () => void api.openDashboard());
onUsageUpdated(() => void refresh());
void refresh();
setInterval(() => void refresh(), 60_000); // 兜底：burn rate 随时间衰减也要更新
```

`style.css` 追加：

```css
#float { display: flex; flex-direction: column; height: 100%; padding: 10px 12px; gap: 6px; }
.f-top { display: flex; align-items: center; gap: 6px; font-size: 12px; white-space: nowrap; overflow: hidden; }
#f-project { font-weight: 600; overflow: hidden; text-overflow: ellipsis; }
.dim { color: #8b90a0; }
.badge { font-size: 10px; padding: 1px 6px; border-radius: 8px; background: #2a2d38; }
.badge-subscription { background: #1d3a2f; color: #6fd49a; }
.badge-api { background: #3a2d1d; color: #d4b16f; }
.f-nums { display: flex; justify-content: space-between; }
.f-nums label { display: block; font-size: 10px; color: #8b90a0; }
.f-nums b { font-size: 16px; font-variant-numeric: tabular-nums; }
.spark { display: flex; align-items: flex-end; gap: 1px; flex: 1; min-height: 24px; }
.spark-col { flex: 1; display: flex; flex-direction: column; justify-content: flex-end; height: 100%; }
.spark-main { background: #4f6df5; border-radius: 1px; }
.spark-side { background: #b46ff5; border-radius: 1px; }
```

- [ ] **Step 3: 验证（手动烟测）**

Run: `cd app/ui && npm run tauri:dev`
Expected: 悬浮窗显示项目名、计费徽标、三个数字、sparkline；在另一个终端跑任意 Claude Code 会话（或 `touch` 一个 jsonl）观察数字秒级更新；双击悬浮窗打开主窗口。

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(ui): typed api layer and floating window"
```

---

### Task 14: 详细面板——框架 + 总览页（ECharts）

**Files:**
- Modify: `app/ui/src/main.ts`, `app/ui/index.html`, `app/ui/src/style.css`
- Create: `app/ui/src/pages/overview.ts`, `app/ui/src/charts.ts`

**Interfaces:**
- Consumes: `api.ts`
- Produces:
  - `charts.ts`: `mountChart(el: HTMLElement, option: echarts.EChartsOption): echarts.ECharts`（含 resize observer）；调色板常量 `PALETTE: string[]`
  - `main.ts`: hash 路由（`#overview` 默认 / `#projects` / `#sessions` / `#settings`），导出 `interface Page { render(root: HTMLElement): void }`；各页面模块导出 `const page: Page`
  - `pages/overview.ts`: 卡片 + 三张图（日成本堆叠面积、模型环形、主/子对比条）

- [ ] **Step 1: `index.html` 骨架**

```html
<body>
  <div id="layout">
    <nav id="nav">
      <h1>Bookholder</h1>
      <a href="#overview" class="active">总览</a>
      <a href="#projects">项目</a>
      <a href="#sessions">会话明细</a>
      <a href="#settings">设置</a>
    </nav>
    <main id="page"></main>
  </div>
  <script type="module" src="/src/main.ts"></script>
</body>
```

- [ ] **Step 2: `charts.ts`**

```ts
import * as echarts from "echarts";

export const PALETTE = ["#4f6df5", "#b46ff5", "#6fd49a", "#d4b16f", "#f56d6d", "#6fc4d4"];

export function mountChart(el: HTMLElement, option: echarts.EChartsOption): echarts.ECharts {
  const chart = echarts.init(el, undefined, { renderer: "canvas" });
  chart.setOption({
    color: PALETTE,
    textStyle: { color: "#e8eaf0" },
    backgroundColor: "transparent",
    ...option,
  });
  new ResizeObserver(() => chart.resize()).observe(el);
  return chart;
}
```

- [ ] **Step 3: `main.ts` 路由**

```ts
import { onUsageUpdated } from "./api";
import { page as overview } from "./pages/overview";
import { page as projects } from "./pages/projects";
import { page as sessions } from "./pages/sessions";
import { page as settings } from "./pages/settings";

export interface Page { render(root: HTMLElement): void }

const routes: Record<string, Page> = {
  "#overview": overview,
  "#projects": projects,
  "#sessions": sessions,
  "#settings": settings,
};

function route(): void {
  const hash = location.hash || "#overview";
  document.querySelectorAll("#nav a").forEach((a) =>
    a.classList.toggle("active", a.getAttribute("href") === hash));
  const root = document.getElementById("page")!;
  root.innerHTML = "";
  (routes[hash] ?? routes["#overview"]).render(root);
}

window.addEventListener("hashchange", route);
onUsageUpdated(route); // 数据更新 → 当前页整页重渲染（数据量小，简单可靠）
route();
```

（Task 14 只创建 `pages/overview.ts`；`projects/sessions/settings` 三个文件本任务先建占位：`export const page = { render(root: HTMLElement) { root.textContent = "…"; } };`，Task 15/16 替换。）

- [ ] **Step 4: `pages/overview.ts`**

```ts
import { api, fmtUsd, fmtTok, Totals } from "../api";
import { mountChart, PALETTE } from "../charts";
import type { Page } from "../main";

function card(label: string, t: Totals): string {
  return `<div class="card"><label>${label}</label><b>${fmtUsd(t.cost_usd)}</b>
    <span class="dim">in ${fmtTok(t.input)} · out ${fmtTok(t.output)} · cache读 ${fmtTok(t.cache_read)} · 写 ${fmtTok(t.cache_write)}</span>
    ${t.unpriced > 0 ? `<span class="warn">${t.unpriced} 条未计价</span>` : ""}</div>`;
}

export const page: Page = {
  render(root: HTMLElement): void {
    root.innerHTML = `<div id="cards" class="cards"></div>
      <div class="chart-grid">
        <div class="panel"><h3>近 30 天成本（按模型）</h3><div id="c-daily" class="chart"></div></div>
        <div class="panel"><h3>模型占比</h3><div id="c-models" class="chart"></div></div>
        <div class="panel"><h3>主对话 vs Subagent</h3><div id="c-side" class="chart chart-slim"></div></div>
      </div>`;
    void (async () => {
      const o = await api.overview();
      document.getElementById("cards")!.innerHTML =
        card("今日", o.today) + card("近 7 天", o.week) + card("近 30 天", o.month) + card("全部", o.all);

      const models = [...new Set(o.daily.map((d) => d.model))];
      const dates = [...new Set(o.daily.map((d) => d.date))].sort();
      mountChart(document.getElementById("c-daily")!, {
        tooltip: { trigger: "axis" },
        legend: { textStyle: { color: "#8b90a0" } },
        xAxis: { type: "category", data: dates },
        yAxis: { type: "value", axisLabel: { formatter: (v: number) => `$${v}` } },
        series: models.map((m) => ({
          name: m, type: "line", stack: "cost", areaStyle: {}, showSymbol: false,
          data: dates.map((dt) => o.daily.find((r) => r.date === dt && r.model === m)?.cost_usd ?? 0),
        })),
      });

      mountChart(document.getElementById("c-models")!, {
        tooltip: { formatter: (p: { name: string; value: number }) => `${p.name}: ${fmtUsd(p.value)}` },
        series: [{ type: "pie", radius: ["45%", "72%"],
          label: { color: "#e8eaf0" },
          data: o.models.map((m) => ({ name: m.model, value: +m.cost_usd.toFixed(4) })) }],
      });

      mountChart(document.getElementById("c-side")!, {
        tooltip: {},
        xAxis: { type: "value", axisLabel: { formatter: (v: number) => `$${v}` } },
        yAxis: { type: "category", data: ["成本"] },
        series: [
          { name: "主对话", type: "bar", stack: "s", data: [+o.main_cost.toFixed(4)], itemStyle: { color: PALETTE[0] } },
          { name: "Subagent", type: "bar", stack: "s", data: [+o.side_cost.toFixed(4)], itemStyle: { color: PALETTE[1] } },
        ],
        legend: { textStyle: { color: "#8b90a0" } },
      });
    })();
  },
};
```

- [ ] **Step 5: 样式追加**（style.css）

```css
#layout { display: flex; height: 100vh; }
#nav { width: 150px; padding: 16px 10px; background: #1a1d26; display: flex; flex-direction: column; gap: 4px; }
#nav h1 { font-size: 15px; margin-bottom: 12px; padding-left: 8px; }
#nav a { color: #8b90a0; text-decoration: none; padding: 7px 10px; border-radius: 7px; font-size: 13px; }
#nav a.active { background: #2a2d38; color: #e8eaf0; }
#page { flex: 1; overflow-y: auto; padding: 18px; }
.cards { display: grid; grid-template-columns: repeat(4, 1fr); gap: 10px; margin-bottom: 14px; }
.card { background: #1a1d26; border-radius: 10px; padding: 12px; display: flex; flex-direction: column; gap: 3px; }
.card label { font-size: 11px; color: #8b90a0; }
.card b { font-size: 20px; font-variant-numeric: tabular-nums; }
.card .dim { font-size: 10px; }
.warn { color: #d4b16f; font-size: 10px; }
.chart-grid { display: grid; grid-template-columns: 2fr 1fr; gap: 10px; }
.panel { background: #1a1d26; border-radius: 10px; padding: 12px; }
.panel h3 { font-size: 12px; color: #8b90a0; margin-bottom: 6px; }
.chart { height: 260px; }
.chart-slim { height: 120px; }
table { width: 100%; border-collapse: collapse; font-size: 12px; }
th, td { text-align: left; padding: 7px 8px; border-bottom: 1px solid #2a2d38; }
th { color: #8b90a0; font-weight: 500; }
tr.clickable { cursor: pointer; }
tr.clickable:hover { background: #21242e; }
button { background: #2a2d38; color: #e8eaf0; border: none; border-radius: 7px; padding: 7px 14px; font-size: 12px; cursor: pointer; }
button:hover { background: #343846; }
```

- [ ] **Step 6: 验证（手动烟测）**

Run: `cd app/ui && npm run tauri:dev`，托盘菜单点"打开面板"。
Expected: 总览页四张卡片有真实数字、三张图正常渲染、跑一个 Claude Code 会话后自动刷新。

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat(ui): dashboard shell and overview page with echarts"
```

---

### Task 15: 项目页 + 会话明细页

**Files:**
- Create/替换占位: `app/ui/src/pages/projects.ts`, `app/ui/src/pages/sessions.ts`

**Interfaces:**
- Consumes: `api.projects() / api.sessions(id) / api.events(id)`
- Produces: `#projects` 项目排行表 → 点击行进入该项目的会话列表 → 点击会话展开逐请求明细；`#sessions` 页为全项目扁平入口（先选项目再看会话）。两页共用渲染函数。

- [ ] **Step 1: 实现 `pages/projects.ts`**

```ts
import { api, fmtUsd, fmtTok, ProjectRow, SessionRow, EventRow } from "../api";
import type { Page } from "../main";

function eventTable(evs: EventRow[]): string {
  const rows = evs.map((e) => `<tr>
    <td>${e.ts}</td><td>${e.model}</td><td>${e.is_sidechain ? "sub" : "主"}</td>
    <td>${fmtTok(e.input)}</td><td>${fmtTok(e.output)}</td><td>${fmtTok(e.thinking)}</td>
    <td>${fmtTok(e.cache_write_5m + e.cache_write_1h)}</td><td>${fmtTok(e.cache_read)}</td>
    <td>${e.cost_usd == null ? "未计价" : fmtUsd(e.cost_usd)}</td></tr>`).join("");
  return `<table><tr><th>时间</th><th>模型</th><th>类型</th><th>in</th><th>out</th>
    <th>think</th><th>cache写</th><th>cache读</th><th>成本</th></tr>${rows}</table>`;
}

async function showSessions(root: HTMLElement, p: ProjectRow): Promise<void> {
  const sessions = await api.sessions(p.id);
  root.innerHTML = `<button id="back">← 项目列表</button>
    <h2 style="margin:10px 0">${p.display_name} <span class="dim">${fmtUsd(p.cost_usd)}</span></h2>
    <div id="s-list"></div>`;
  root.querySelector("#back")!.addEventListener("click", () => void page.render(root));
  const list = root.querySelector("#s-list")!;
  for (const s of sessions) {
    const div = document.createElement("div");
    div.className = "panel";
    div.style.marginBottom = "8px";
    div.innerHTML = `<div class="clickable sess-head" style="display:flex;gap:12px;cursor:pointer">
      <b>${s.session_id.slice(0, 8)}</b><span class="dim">${s.started_at} → ${s.ended_at}</span>
      <span>${fmtUsd(s.cost_usd)}</span><span class="dim">${s.events} 次请求</span>
      <span class="dim">subagent ${fmtUsd(s.side_cost)}</span>
      <span class="badge badge-${s.billing_mode}">${s.billing_mode === "subscription" ? "订阅" : s.billing_mode}</span>
    </div><div class="sess-body" style="display:none;margin-top:8px"></div>`;
    div.querySelector(".sess-head")!.addEventListener("click", () => {
      const body = div.querySelector(".sess-body") as HTMLElement;
      if (body.style.display === "none") {
        body.style.display = "block";
        void api.events(s.id).then((evs) => (body.innerHTML = eventTable(evs)));
      } else {
        body.style.display = "none";
      }
    });
    list.appendChild(div);
  }
}

export const page: Page = {
  render(root: HTMLElement): void {
    void (async () => {
      const projects = await api.projects();
      root.innerHTML = `<h2 style="margin-bottom:10px">项目</h2>
        <table><tr><th>项目</th><th>成本</th><th>tokens</th><th>会话</th><th>活跃天数</th><th>最近活动</th></tr>
        ${projects.map((p, i) => `<tr class="clickable" data-i="${i}">
          <td><b>${p.display_name}</b> <span class="dim">${p.cwd}</span></td>
          <td>${fmtUsd(p.cost_usd)}</td><td>${fmtTok(p.tokens)}</td>
          <td>${p.sessions}</td><td>${p.active_days}</td><td>${p.last_seen}</td></tr>`).join("")}
        </table>`;
      root.querySelectorAll("tr.clickable").forEach((tr) =>
        tr.addEventListener("click", () =>
          void showSessions(root, projects[Number((tr as HTMLElement).dataset.i)])));
    })();
  },
};
```

- [ ] **Step 2: 实现 `pages/sessions.ts`**（复用项目页：会话明细入口 = 项目页；此页直接渲染同一模块，保持导航语义）

```ts
import { page as projectsPage } from "./projects";
import type { Page } from "../main";

export const page: Page = {
  render(root: HTMLElement): void {
    projectsPage.render(root); // 会话明细从项目下钻进入，共用实现
  },
};
```

（若嫌重复可在导航里去掉"会话明细"项；保留是为了符合 spec 的四页导航，实现共用即可。）

- [ ] **Step 3: 验证（手动烟测）**

Run: `cd app/ui && npm run tauri:dev`
Expected: 项目表按成本降序；点项目进会话列表；点会话展开逐请求表格，subagent 行标"sub"，未计价行显示"未计价"。

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat(ui): projects and session drilldown pages"
```

---

### Task 16: 设置页（全部按钮化）

**Files:**
- Create/替换占位: `app/ui/src/pages/settings.ts`

**Interfaces:**
- Consumes: `api.settings() / refreshPrices() / backfill() / exportReport() / setBillingOverride()`、`@tauri-apps/plugin-dialog` 的 `save`、`@tauri-apps/plugin-autostart`
- Produces: 设置页，含价格状态 + 刷新按钮、计费口径覆盖、开机自启开关、重扫按钮、导出三键、解析跳过率。

- [ ] **Step 1: 实现 `pages/settings.ts`**

```ts
import { api } from "../api";
import { save } from "@tauri-apps/plugin-dialog";
import { enable, disable, isEnabled } from "@tauri-apps/plugin-autostart";
import type { Page } from "../main";

async function exportAs(kind: "md" | "csv" | "json", status: HTMLElement): Promise<void> {
  const ext = kind;
  const dest = await save({
    defaultPath: `bookholder-report.${ext}`,
    filters: [{ name: kind.toUpperCase(), extensions: [ext] }],
  });
  if (!dest) return;
  await api.exportReport(kind, dest);
  status.textContent = `已导出 ${dest}`;
}

export const page: Page = {
  render(root: HTMLElement): void {
    void (async () => {
      const s = await api.settings();
      const auto = await isEnabled().catch(() => false);
      root.innerHTML = `<h2 style="margin-bottom:10px">设置</h2>
        <div class="panel" style="margin-bottom:10px">
          <h3>价格数据</h3>
          <p>已知 ${s.price_count} 个模型 ｜ 最后更新：${s.prices_last_fetch ?? "从未（使用内置快照）"}
             ｜ 状态：${s.prices_last_status ?? "—"}</p>
          <button id="btn-prices">立即刷新价格</button>
        </div>
        <div class="panel" style="margin-bottom:10px">
          <h3>计费口径（当前：${s.billing_mode === "subscription" ? "订阅（显示等值 API 成本）" : s.billing_mode === "api" ? "API（实际计费成本）" : "未知"}）</h3>
          <select id="sel-billing">
            <option value="" ${!s.billing_override ? "selected" : ""}>自动检测</option>
            <option value="subscription" ${s.billing_override === "subscription" ? "selected" : ""}>强制：订阅</option>
            <option value="api" ${s.billing_override === "api" ? "selected" : ""}>强制：API</option>
          </select>
        </div>
        <div class="panel" style="margin-bottom:10px">
          <h3>数据</h3>
          <p>数据库：${s.db_path} ｜ 跳过行：${s.skip_lines ?? 0} ｜ 坏行：${s.bad_lines ?? 0}</p>
          <button id="btn-backfill">全量重扫</button>
          <button id="btn-md">导出 Markdown</button>
          <button id="btn-csv">导出 CSV</button>
          <button id="btn-json">导出 JSON</button>
        </div>
        <div class="panel">
          <h3>启动</h3>
          <label><input type="checkbox" id="chk-auto" ${auto ? "checked" : ""}/> 开机自动启动</label>
        </div>
        <p id="status" class="dim" style="margin-top:10px"></p>`;

      const status = root.querySelector("#status") as HTMLElement;
      const busy = async (btn: HTMLButtonElement, fn: () => Promise<string | void>): Promise<void> => {
        btn.disabled = true;
        try { status.textContent = (await fn()) ?? "完成"; }
        catch (e) { status.textContent = `失败：${e}`; }
        finally { btn.disabled = false; }
      };
      const q = (id: string): HTMLButtonElement => root.querySelector(id) as HTMLButtonElement;
      q("#btn-prices").addEventListener("click", () => void busy(q("#btn-prices"), () => api.refreshPrices()));
      q("#btn-backfill").addEventListener("click", () => void busy(q("#btn-backfill"), async () => {
        const st = await api.backfill();
        return `重扫完成：新增 ${st.added}，跳过 ${st.skipped}，坏行 ${st.bad}`;
      }));
      q("#btn-md").addEventListener("click", () => void busy(q("#btn-md"), () => exportAs("md", status)));
      q("#btn-csv").addEventListener("click", () => void busy(q("#btn-csv"), () => exportAs("csv", status)));
      q("#btn-json").addEventListener("click", () => void busy(q("#btn-json"), () => exportAs("json", status)));
      (root.querySelector("#sel-billing") as HTMLSelectElement).addEventListener("change", (e) => {
        void api.setBillingOverride((e.target as HTMLSelectElement).value).then(() => page.render(root));
      });
      (root.querySelector("#chk-auto") as HTMLInputElement).addEventListener("change", (e) => {
        const on = (e.target as HTMLInputElement).checked;
        void (on ? enable() : disable()).then(() => (status.textContent = on ? "已开启自启" : "已关闭自启"));
      });
    })();
  },
};
```

- [ ] **Step 2: 验证（手动烟测）**

Run: `cd app/ui && npm run tauri:dev` → 设置页
Expected: 点"立即刷新价格"显示 updated N models；点"全量重扫"显示统计；导出三键各自弹保存对话框并成功写文件；切换口径后页面刷新；勾选自启不报错。

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat(ui): settings page with all actions as buttons"
```

---

### Task 17: 打包、Makefile、README

**Files:**
- Create: `Makefile`, `README.md`

**Interfaces:**
- Produces: `make setup` 一条命令完成构建安装；README 说明数据来源、口径与三种使用方式（GUI 常驻 / 面板 / CLI）。

- [ ] **Step 1: Makefile**

```makefile
.PHONY: setup build test dev install clean

test:
	cargo test

dev:
	cd app/ui && npm run tauri:dev

build:
	cd app/ui && npm install && npm run tauri:build

install: build
	rm -rf /Applications/Bookholder.app
	cp -R app/src-tauri/target/release/bundle/macos/Bookholder.app /Applications/
	open /Applications/Bookholder.app

setup: test install
	@echo "✅ Bookholder 已安装并启动。开机自启请在应用设置页勾选。"

clean:
	cargo clean && rm -rf app/ui/node_modules app/ui/dist
```

（注意：workspace 下 tauri 的 bundle 输出目录可能是根 `target/release/bundle/macos/`，安装步骤如报路径不存在，用 `find target -name "Bookholder.app" -maxdepth 5` 找到实际路径修正 Makefile。）

- [ ] **Step 2: README.md**

```markdown
# Bookholder

统计 Claude Code agentic 开发项目的真实 token 消耗与成本。

- **数据来源**：本地 `~/.claude/projects/**/*.jsonl` 转录（每条请求的真实 usage），
  解析后持久化到本地 SQLite，历史不随 Claude Code 的 30 天清理丢失。
- **计价**：LiteLLM 社区价格表每日自动更新、保留历史版本；订阅模式显示
  "等值 API 成本"，API 模式显示"实际计费成本"。
- **统计维度**：项目 / 会话 / 模型 / 主对话 vs subagent，逐请求可追溯。

## 安装

    make setup

装完出现悬浮窗（今日成本 / 本项目 / 燃烧率 + 24h sparkline）。
双击悬浮窗或托盘菜单打开详细面板。全部功能在面板上都是按钮。

## 可选 CLI

    cargo run -p bookholder -- report          # Markdown 报告
    cargo run -p bookholder -- report --json   # JSON（供脚本）
    cargo run -p bookholder -- backfill        # 全量重扫
    cargo run -p bookholder -- live            # 实时事件流

## 已知边界

- 未知模型的事件不计价（面板会提示条数），价格表更新后自动补算。
- 订阅模式的"成本"是等值 API 价，不是你的订阅账单。
```

- [ ] **Step 3: 验证**

Run: `make test && make build`
Expected: 测试全过；`Bookholder.app` 产出成功

Run: `make install`（手动烟测：应用启动、悬浮窗常驻、面板可开）

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "chore: makefile setup pipeline and readme"
```

---

## Self-Review 记录

1. **Spec 覆盖**：数据模型/采集（Task 2–7）✅；价格更新与历史版本（Task 4、12 每日刷新）✅；订阅/API 口径（Task 5、12、16）✅；主对话 vs subagent 分账（parse 的 isSidechain → queries.sidechain_split → 总览图 + 会话展开）✅；悬浮窗（Task 13）✅；四页面板（Task 14–16）✅；GUI 优先按钮化（Task 16）✅；CLI 可选层（Task 10）✅；`make setup` 自动化（Task 17）✅；错误处理（坏行计数 Task 6、价格失败缓存 Task 4/12、未知模型 NULL 不计价）✅；第二阶段特征积累（projects 表预留列，Task 3）✅。
2. **占位符扫描**：无 TBD/TODO；Task 11 前端两个占位文件在 Task 14/15/16 明确替换，属计划内交接，均给出实际代码。
3. **类型一致性**：`record_event(conn, slug, e, cost, billing)`、`IngestStats{added,skipped,bad}`、`queries` 各 struct 与 `api.ts` TS 接口字段逐一核对为 snake_case 一致；`detect(home, env_api_key)` 签名在 Task 5 测试与实现同步修正 ✅。
```
