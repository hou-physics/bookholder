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

/// RFC4180 字段转义：内部 `"` 双写，并整体加引号。
fn q(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
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
            q(&r.get::<_, String>(0)?), q(&r.get::<_, String>(1)?), q(&r.get::<_, String>(2)?), q(&r.get::<_, String>(3)?),
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
