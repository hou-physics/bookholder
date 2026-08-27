use rusqlite::{params, Connection};
use serde::Serialize;

/// 订阅用量限额（与 Claude Code /usage 同源：OAuth usage 接口）。
/// 凭证只从 macOS 钥匙串现读、驻留内存，不落库；库里只存百分比采样。
#[derive(Debug, Clone, Serialize, Default)]
pub struct WindowUsage {
    pub utilization: f64,          // 0–100
    pub resets_at: Option<String>, // RFC3339
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct UsageLimits {
    pub five_hour: Option<WindowUsage>,
    pub seven_day: Option<WindowUsage>,
    /// 分模型周窗口（如 seven_day_opus），接口返回非空才有
    pub model_windows: Vec<(String, WindowUsage)>,
}

/// 接口 `limits` 数组的一行——Claude 客户端实际展示的三类进度条来源：
/// session(5h) / weekly_all / weekly_scoped(带模型名)。
#[derive(Debug, Clone, Serialize)]
pub struct LimitWindow {
    pub key: String,               // 采样键：session / weekly_all / weekly_<model>
    pub kind: String,              // 原始 kind
    pub scope_label: Option<String>, // weekly_scoped 的模型显示名（如 "Fable"）
    pub utilization: f64,
    pub resets_at: Option<String>,
}

/// 首选解析 `limits` 数组；缺失时回退 five_hour/seven_day 顶级字段。
pub fn parse_limit_windows(json: &str) -> Result<Vec<LimitWindow>, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let mut out = vec![];
    if let Some(arr) = v.get("limits").and_then(|l| l.as_array()) {
        for l in arr {
            let Some(kind) = l.get("kind").and_then(|k| k.as_str()) else { continue };
            let Some(pct) = l.get("percent").and_then(|p| p.as_f64()) else { continue };
            let scope_label = l
                .get("scope")
                .and_then(|s| s.get("model"))
                .and_then(|m| m.get("display_name"))
                .and_then(|n| n.as_str())
                .map(|s| s.to_string());
            let key = match (kind, &scope_label) {
                ("session", _) => "session".to_string(),
                (k, Some(m)) => format!("{k}_{m}"),
                (k, None) => k.to_string(),
            };
            out.push(LimitWindow {
                key,
                kind: kind.to_string(),
                scope_label,
                utilization: pct,
                resets_at: l.get("resets_at").and_then(|r| r.as_str()).map(|s| s.to_string()),
            });
        }
    }
    if out.is_empty() {
        // 回退旧字段
        let u = parse_usage(json)?;
        if let Some(w) = u.five_hour {
            out.push(LimitWindow { key: "session".into(), kind: "session".into(), scope_label: None, utilization: w.utilization, resets_at: w.resets_at });
        }
        if let Some(w) = u.seven_day {
            out.push(LimitWindow { key: "weekly_all".into(), kind: "weekly_all".into(), scope_label: None, utilization: w.utilization, resets_at: w.resets_at });
        }
    }
    Ok(out)
}

pub fn record_window_samples(conn: &Connection, ws: &[LimitWindow], now_utc: &str) -> rusqlite::Result<()> {
    ensure_table(conn)?;
    for w in ws {
        conn.execute(
            "INSERT OR IGNORE INTO usage_samples (ts, kind, utilization) VALUES (?1, ?2, ?3)",
            params![now_utc, w.key, w.utilization],
        )?;
    }
    conn.execute("DELETE FROM usage_samples WHERE ts < datetime('now', '-14 days')", [])?;
    Ok(())
}

pub const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

fn keychain_token() -> Result<String, String> {
    let out = std::process::Command::new("security")
        .args(["find-generic-password", "-s", "Claude Code-credentials", "-w"])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("keychain denied or credential missing".into());
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(raw.trim()).map_err(|e| e.to_string())?;
    v.get("claudeAiOauth")
        .and_then(|o| o.get("accessToken"))
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "no accessToken in credential".into())
}

pub fn fetch_usage_json() -> Result<String, String> {
    let token = keychain_token()?;
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(15))
        .build();
    agent
        .get(USAGE_URL)
        .set("Authorization", &format!("Bearer {token}"))
        .set("anthropic-beta", "oauth-2025-04-20")
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())
}

pub fn parse_usage(json: &str) -> Result<UsageLimits, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let win = |o: &serde_json::Value| -> Option<WindowUsage> {
        let u = o.get("utilization")?.as_f64()?;
        Some(WindowUsage {
            utilization: u,
            resets_at: o.get("resets_at").and_then(|r| r.as_str()).map(|s| s.to_string()),
        })
    };
    let mut out = UsageLimits {
        five_hour: v.get("five_hour").and_then(win),
        seven_day: v.get("seven_day").and_then(win),
        model_windows: vec![],
    };
    for key in ["seven_day_opus", "seven_day_sonnet"] {
        if let Some(w) = v.get(key).filter(|x| !x.is_null()).and_then(win) {
            out.model_windows.push((key.to_string(), w));
        }
    }
    Ok(out)
}

pub fn ensure_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS usage_samples (
           ts TEXT NOT NULL,
           kind TEXT NOT NULL,
           utilization REAL NOT NULL,
           PRIMARY KEY (ts, kind)
         );",
    )
}

pub fn record_samples(conn: &Connection, u: &UsageLimits, now_utc: &str) -> rusqlite::Result<()> {
    ensure_table(conn)?;
    let mut put = |kind: &str, w: &Option<WindowUsage>| -> rusqlite::Result<()> {
        if let Some(w) = w {
            conn.execute(
                "INSERT OR IGNORE INTO usage_samples (ts, kind, utilization) VALUES (?1, ?2, ?3)",
                params![now_utc, kind, w.utilization],
            )?;
        }
        Ok(())
    };
    put("five_hour", &u.five_hour)?;
    put("seven_day", &u.seven_day)?;
    // 保留 14 天，防膨胀
    conn.execute(
        "DELETE FROM usage_samples WHERE ts < datetime('now', '-14 days')",
        [],
    )?;
    Ok(())
}

/// 特斯拉式"续航"估算：回看 `lookback_mins` 内最早的采样，用斜率外推还有几小时打满 100%。
/// 斜率 ≤ 0（窗口重置回落）或采样不足 → None。
pub fn eta_hours(conn: &Connection, kind: &str, current: f64, lookback_mins: i64) -> Option<f64> {
    let _ = ensure_table(conn);
    let (then_ts, then_util): (String, f64) = conn
        .query_row(
            "SELECT ts, utilization FROM usage_samples
             WHERE kind = ?1 AND ts >= datetime('now', ?2)
             ORDER BY ts ASC LIMIT 1",
            params![kind, format!("-{lookback_mins} minutes")],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()?;
    let then = chrono::NaiveDateTime::parse_from_str(&then_ts, "%Y-%m-%d %H:%M:%S").ok()?;
    let hours = (chrono::Utc::now().naive_utc() - then).num_seconds() as f64 / 3600.0;
    if hours < 0.05 {
        return None; // 窗口内只有刚写入的采样，斜率不可信
    }
    let slope = (current - then_util) / hours; // %/h
    if slope <= 0.05 {
        return None;
    }
    Some((100.0 - current) / slope)
}

/// 周窗口的人性化续航：换算成"还能用多少个典型工作日"。
///
/// 配额绝对量未知，用成本代理外推：本周窗口内已消耗成本 C 对应利用率 U%，
/// 则整周配额 ≈ C×100/U，剩余预算 ≈ 配额×(100−U)/100。
/// 典型工作日 = 过去 14 个完整日中活跃日（有任何消耗）的日成本中位数（≥3 天才启用）。
/// `model_like`：weekly_scoped 传模型名子串（如 "fable"），只统计该模型的成本。
pub fn weekly_days_left(
    conn: &Connection,
    utilization: f64,
    resets_at: Option<&str>,
    model_like: Option<&str>,
) -> Option<f64> {
    if !(3.0..100.0).contains(&utilization) {
        return None; // 样本太少或已打满
    }
    // 窗口起点 = 重置时刻 - 7 天
    let reset = chrono::DateTime::parse_from_rfc3339(resets_at?).ok()?;
    let window_start = (reset.with_timezone(&chrono::Utc) - chrono::Duration::days(7))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let like = model_like.map(|m| format!("%{}%", m.to_lowercase()));
    let c_window: f64 = conn
        .query_row(
            "SELECT COALESCE(SUM(cost_usd),0) FROM usage_events
             WHERE ts >= ?1 AND (?2 IS NULL OR lower(model) LIKE ?2)",
            params![window_start, like],
            |r| r.get(0),
        )
        .unwrap_or(0.0);
    if c_window <= 0.0 {
        return None;
    }
    let quota = c_window * 100.0 / utilization;
    let remaining_budget = quota * (100.0 - utilization) / 100.0;
    // 过去 14 个完整日的活跃日成本中位数
    let mut daily: Vec<f64> = conn
        .prepare(
            "SELECT SUM(cost_usd) FROM usage_events
             WHERE date(ts,'localtime') >= date('now','localtime','-14 days')
               AND date(ts,'localtime') < date('now','localtime')
               AND (?1 IS NULL OR lower(model) LIKE ?1)
             GROUP BY date(ts,'localtime') HAVING SUM(cost_usd) > 0",
        )
        .ok()?
        .query_map(params![like], |r| r.get::<_, f64>(0))
        .ok()?
        .flatten()
        .collect();
    if daily.len() < 3 {
        return None;
    }
    daily.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = daily[daily.len() / 2];
    if median <= 0.0 {
        return None;
    }
    Some(remaining_budget / median)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
      "five_hour": {"utilization": 13.0, "resets_at": "2026-08-27T22:49:59+00:00"},
      "seven_day": {"utilization": 19.0, "resets_at": "2026-09-02T08:59:59+00:00"},
      "seven_day_opus": null,
      "seven_day_sonnet": {"utilization": 4.5, "resets_at": null},
      "extra_usage": {"is_enabled": false}
    }"#;

    #[test]
    fn parses_real_shape() {
        let u = parse_usage(FIXTURE).unwrap();
        assert_eq!(u.five_hour.as_ref().unwrap().utilization, 13.0);
        assert!(u.five_hour.unwrap().resets_at.unwrap().starts_with("2026-08-27"));
        assert_eq!(u.seven_day.unwrap().utilization, 19.0);
        assert_eq!(u.model_windows.len(), 1);
        assert_eq!(u.model_windows[0].0, "seven_day_sonnet");
    }

    const LIMITS_FIXTURE: &str = r#"{
      "five_hour": {"utilization": 14.0, "resets_at": "2026-08-27T22:49:59+00:00"},
      "limits": [
        {"kind": "session", "group": "session", "percent": 14, "resets_at": "2026-08-27T22:49:59+00:00", "scope": null},
        {"kind": "weekly_all", "group": "weekly", "percent": 20, "resets_at": "2026-09-02T08:59:59+00:00", "scope": null},
        {"kind": "weekly_scoped", "group": "weekly", "percent": 21, "resets_at": "2026-09-02T08:59:59+00:00",
         "scope": {"model": {"id": null, "display_name": "Fable"}, "surface": null}}
      ]
    }"#;

    #[test]
    fn parses_limits_array_with_scoped_model() {
        let ws = parse_limit_windows(LIMITS_FIXTURE).unwrap();
        assert_eq!(ws.len(), 3);
        assert_eq!(ws[0].key, "session");
        assert_eq!(ws[1].key, "weekly_all");
        assert_eq!(ws[2].key, "weekly_scoped_Fable");
        assert_eq!(ws[2].scope_label.as_deref(), Some("Fable"));
        assert_eq!(ws[2].utilization, 21.0);
    }

    #[test]
    fn falls_back_to_top_level_fields() {
        let ws = parse_limit_windows(FIXTURE).unwrap(); // FIXTURE 无 limits 数组
        assert_eq!(ws.len(), 2);
        assert_eq!(ws[0].key, "session");
        assert_eq!(ws[0].utilization, 13.0);
    }

    #[test]
    fn eta_from_sample_slope() {
        let conn = crate::store::open_memory().unwrap();
        ensure_table(&conn).unwrap();
        // 30 分钟前 10%，现在 20% → 20%/h → 到 100% 还有 4 小时
        let then = (chrono::Utc::now() - chrono::Duration::minutes(30))
            .format("%Y-%m-%d %H:%M:%S").to_string();
        conn.execute(
            "INSERT INTO usage_samples (ts, kind, utilization) VALUES (?1, 'five_hour', 10.0)",
            [&then],
        ).unwrap();
        let eta = eta_hours(&conn, "five_hour", 20.0, 60).unwrap();
        assert!((eta - 4.0).abs() < 0.2, "eta {eta}");
        // 回落（重置后）→ None
        assert!(eta_hours(&conn, "five_hour", 5.0, 60).is_none());
        // 无采样 kind → None
        assert!(eta_hours(&conn, "seven_day", 50.0, 60).is_none());
    }

    #[test]
    fn weekly_days_left_from_cost_proxy() {
        let conn = crate::store::open_memory().unwrap();
        let mk = |key: &str, days_ago: i64, cost: f64, model: &str| {
            let ts = (chrono::Utc::now() - chrono::Duration::days(days_ago) - chrono::Duration::hours(1))
                .format("%Y-%m-%dT%H:%M:%SZ").to_string();
            let e = crate::model::UsageEvent {
                dedup_key: key.into(), ts, session_id: format!("s{days_ago}"), cwd: "/u/p".into(),
                model: model.into(), is_sidechain: false, input_tokens: 1, output_tokens: 1,
                thinking_tokens: 0, cache_write_5m: 0, cache_write_1h: 0, cache_read: 0,
            };
            crate::store::record_event(&conn, "-p", &e, Some(cost), "subscription").unwrap();
        };
        // 过去完整日：日成本 10/10/10/30（中位数 10）
        mk("d1", 1, 10.0, "claude-fable-5");
        mk("d2", 2, 10.0, "claude-fable-5");
        mk("d3", 3, 10.0, "claude-fable-5");
        mk("d4", 4, 30.0, "claude-fable-5");
        // 本周窗口（重置在 2 天后 → 窗口始于 5 天前）内成本 = d1..d4 全部 60
        let resets = (chrono::Utc::now() + chrono::Duration::days(2)).to_rfc3339();
        // U=20% → 配额 = 60*100/20 = 300，剩余 = 240，中位日成本 10 → 24 天
        let d = weekly_days_left(&conn, 20.0, Some(&resets), None).unwrap();
        assert!((d - 24.0).abs() < 0.01, "{d}");
        // 模型过滤：无匹配成本 → None
        assert!(weekly_days_left(&conn, 20.0, Some(&resets), Some("opus")).is_none());
        // 活跃日不足 → None（过滤出 0 天）
        assert!(weekly_days_left(&conn, 1.0, Some(&resets), None).is_none()); // U<3
    }

    #[test]
    fn samples_dedupe_and_prune() {
        let conn = crate::store::open_memory().unwrap();
        let u = parse_usage(FIXTURE).unwrap();
        record_samples(&conn, &u, "2026-08-27 18:00:00").unwrap();
        record_samples(&conn, &u, "2026-08-27 18:00:00").unwrap(); // 幂等
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM usage_samples", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2); // five_hour + seven_day
    }
}
