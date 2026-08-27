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
    fn samples_dedupe_and_prune() {
        let conn = crate::store::open_memory().unwrap();
        let u = parse_usage(FIXTURE).unwrap();
        record_samples(&conn, &u, "2026-08-27 18:00:00").unwrap();
        record_samples(&conn, &u, "2026-08-27 18:00:00").unwrap(); // 幂等
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM usage_samples", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2); // five_hour + seven_day
    }
}
