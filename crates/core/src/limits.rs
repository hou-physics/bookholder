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

fn read_oauth_json() -> Result<serde_json::Value, String> {
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
        .cloned()
        .ok_or_else(|| "no claudeAiOauth in credential".into())
}

/// 钥匙串凭证的两层寿命：access token 只活几小时（CLI 跑一次就续）；
/// refresh token 固定约 7 天、不随续期滚动，到期后 CLI 会把两者清空——
/// 那时任何自动续期都无效，只能重新走浏览器登录。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CredentialStatus {
    pub access_remaining_secs: i64,
    /// None = 凭证里没有该字段
    pub refresh_remaining_secs: Option<i64>,
    /// false = token 已被清空 / refresh token 已到期：需要用户重新登录
    pub renewable: bool,
}

pub fn credential_status() -> Result<CredentialStatus, String> {
    let oauth = read_oauth_json()?;
    Ok(status_from_oauth(&oauth))
}

fn status_from_oauth(oauth: &serde_json::Value) -> CredentialStatus {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let ms = |k: &str| oauth.get(k).and_then(|e| e.as_i64());
    let access_remaining_secs = (ms("expiresAt").unwrap_or(0) - now_ms) / 1000;
    let refresh_remaining_secs = ms("refreshTokenExpiresAt").map(|e| (e - now_ms) / 1000);
    let has_tokens = ["accessToken", "refreshToken"]
        .iter()
        .all(|k| oauth.get(k).and_then(|t| t.as_str()).map(|s| !s.is_empty()).unwrap_or(false));
    CredentialStatus {
        access_remaining_secs,
        refresh_remaining_secs,
        renewable: has_tokens && refresh_remaining_secs.map(|r| r > 0).unwrap_or(true),
    }
}

/// 错误码约定（前端据此选提示与按钮）：
/// `login_required` = 需要重新登录（自动续无效）；`token_expired` = 自动续期即可恢复。
fn keychain_token() -> Result<String, String> {
    let oauth = read_oauth_json()?;
    let st = status_from_oauth(&oauth);
    if !st.renewable {
        return Err("login_required".into());
    }
    if st.access_remaining_secs <= 0 {
        return Err("token_expired".into());
    }
    oauth
        .get("accessToken")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "no accessToken in credential".into())
}

/// token 距过期还剩多少秒（负数=已过期），不经网络、只读钥匙串。
pub fn token_remaining_secs() -> Result<i64, String> {
    credential_status().map(|s| s.access_remaining_secs)
}

/// 定位本机 `claude` CLI 可执行文件：打包后的 app 由 Finder/launchd 启动，
/// PATH 里没有用户 shell 配置文件加过的目录（nvm/homebrew/~/.local/bin 等），
/// 所以先探测常见安装位置，探测不到再用登录 shell 展开 PATH 兜底。
fn locate_claude_cli() -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let candidates = [
        format!("{home}/.local/bin/claude"),
        format!("{home}/.claude/local/claude"),
        "/opt/homebrew/bin/claude".to_string(),
        "/usr/local/bin/claude".to_string(),
    ];
    for c in candidates {
        if std::path::Path::new(&c).exists() {
            return Some(c);
        }
    }
    let out = std::process::Command::new("zsh")
        .args(["-lc", "command -v claude"])
        .output()
        .ok()?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !path.is_empty()).then_some(path)
}

/// 用一次最小化的 CLI 调用换取钥匙串里 token 的续期——GUI 按钮替代
/// "去终端敲 claude 命令"，用户不需要记命令行。token 只在钥匙串里更新，
/// 本进程不读取、不留存它。
pub fn refresh_login() -> Result<(), String> {
    // refresh token 已到期时 CLI 只会打印 "could not be refreshed"——别白跑一次
    if let Ok(st) = credential_status() {
        if !st.renewable {
            return Err("login_required".into());
        }
    }
    let bin = locate_claude_cli().ok_or("未找到 claude 命令行工具（claude CLI not found on this machine）")?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // --no-session-persistence：不写转录，否则这些 "ok" 会话会被本工具
        // 自己当成项目用量统计进去；--tools/--setting-sources 置空：
        // 不带工具定义和插件 hook，把这次续期调用的 token 消耗压到最低。
        // 不能用 --bare：它跳过钥匙串、只认 API key。
        let result = std::process::Command::new(&bin)
            .args(["-p", "ok", "--model", "haiku", "--no-session-persistence", "--tools", "", "--setting-sources", ""])
            .output();
        let _ = tx.send(result);
    });
    let out = rx
        .recv_timeout(std::time::Duration::from_secs(45))
        .map_err(|_| "刷新超时（45s）".to_string())?
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if stdout.contains("Failed to authenticate") {
        return Err("login_required".into());
    }
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if msg.is_empty() { stdout.trim().to_string() } else { msg });
    }
    // CLI 退出码不可靠（认证失败也可能是 0）：以钥匙串里的实际结果为准
    match credential_status() {
        Ok(st) if st.renewable && st.access_remaining_secs > 0 => Ok(()),
        Ok(st) if !st.renewable => Err("login_required".into()),
        Ok(_) => Err("CLI 调用成功但 token 未更新".into()),
        Err(e) => Err(e),
    }
}

/// 需要浏览器登录时的 GUI 出口：生成一个 .command 脚本并交给系统打开——
/// macOS 会用 Terminal 运行它，`claude auth login` 随即拉起浏览器授权。
/// 不走 AppleScript（需要辅助功能/自动化权限，且会被系统随时重置）。
pub fn open_login_terminal(data_dir: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let bin = locate_claude_cli().ok_or("未找到 claude 命令行工具（claude CLI not found on this machine）")?;
    let script = data_dir.join("relogin.command");
    std::fs::write(
        &script,
        format!("#!/bin/zsh\necho 'Bookholder: 正在重新登录 Claude Code…'\n\"{bin}\" auth login\necho\necho '登录完成后可以关闭这个窗口。'\n"),
    )
    .map_err(|e| e.to_string())?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    let ok = std::process::Command::new("open")
        .arg(&script)
        .status()
        .map_err(|e| e.to_string())?
        .success();
    ok.then_some(()).ok_or_else(|| "无法打开终端".into())
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
    let put = |kind: &str, w: &Option<WindowUsage>| -> rusqlite::Result<()> {
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
/// 典型工作日 = max(14 日活跃日中位数, 最近 3 活跃日均值, 今日已消耗)：
/// 中位数给出常态基准，后两项让爆发期立即收敛到"照现在的速度"（≥3 活跃日才启用）。
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
    // 最近 3 个活跃日均值：行为变重时比中位数反应快
    let last3: f64 = conn
        .prepare(
            "SELECT AVG(c) FROM (
               SELECT SUM(cost_usd) c FROM usage_events
               WHERE date(ts,'localtime') < date('now','localtime')
                 AND (?1 IS NULL OR lower(model) LIKE ?1)
               GROUP BY date(ts,'localtime') HAVING SUM(cost_usd) > 0
               ORDER BY date(ts,'localtime') DESC LIMIT 3)",
        )
        .and_then(|mut st| st.query_row(params![like], |r| r.get(0)))
        .unwrap_or(0.0);
    // 今天已消耗：爆发日当天立即把口径拉到"照现在的速度"
    let today: f64 = conn
        .query_row(
            "SELECT COALESCE(SUM(cost_usd),0) FROM usage_events
             WHERE date(ts,'localtime') = date('now','localtime')
               AND (?1 IS NULL OR lower(model) LIKE ?1)",
            params![like],
            |r| r.get(0),
        )
        .unwrap_or(0.0);
    let typical_day = median.max(last3).max(today);
    if typical_day <= 0.0 {
        return None;
    }
    Some(remaining_budget / typical_day)
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
    fn credential_status_distinguishes_renewable_from_login_required() {
        let now = chrono::Utc::now().timestamp_millis();
        let mk = |access: &str, refresh: &str, exp: i64, rexp: i64| {
            status_from_oauth(&serde_json::json!({
                "accessToken": access, "refreshToken": refresh,
                "expiresAt": exp, "refreshTokenExpiresAt": rexp,
            }))
        };
        // access 已过期但 refresh 仍有效 → 可自动续
        let s = mk("a", "r", now - 3_600_000, now + 86_400_000);
        assert!(s.renewable && s.access_remaining_secs < 0);
        // CLI 清空了 token（refresh 到期后的真实形态）→ 必须重新登录
        let s = mk("", "", 0, now - 1000);
        assert!(!s.renewable);
        // token 还在但 refresh 已到期 → 同样必须重新登录
        assert!(!mk("a", "r", now + 3_600_000, now - 1000).renewable);
        // 没有 refreshTokenExpiresAt 字段：不臆断，视为可续
        let s = status_from_oauth(&serde_json::json!({"accessToken": "a", "refreshToken": "r", "expiresAt": now + 1000}));
        assert!(s.renewable && s.refresh_remaining_secs.is_none());
    }

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
        // 过去完整日（新→旧）：30/10/10/10 —— 中位数 10，最近 3 日均值 (30+10+10)/3≈16.67
        mk("d1", 1, 30.0, "claude-fable-5");
        mk("d2", 2, 10.0, "claude-fable-5");
        mk("d3", 3, 10.0, "claude-fable-5");
        mk("d4", 4, 10.0, "claude-fable-5");
        // 本周窗口（重置在 2 天后 → 窗口始于 5 天前）内成本 = d1..d4 全部 60
        let resets = (chrono::Utc::now() + chrono::Duration::days(2)).to_rfc3339();
        // U=20% → 配额 = 60*100/20 = 300，剩余 = 240
        // 分母 = max(中位数 10, 最近3活跃日均值 (10+10+30)/3=16.67, 今日 0) = 16.67 → 14.4 天
        let d = weekly_days_left(&conn, 20.0, Some(&resets), None).unwrap();
        assert!((d - 14.4).abs() < 0.05, "{d}");
        // 爆发日：今天已烧 60 → 窗口成本 120，配额 600，剩余 480；分母 max(10,16.67,60)=60 → 8 天
        mk("today", 0, 60.0, "claude-fable-5");
        let d2 = weekly_days_left(&conn, 20.0, Some(&resets), None).unwrap();
        assert!(d2 < d, "爆发日应立即缩短续航: {d2} vs {d}");
        assert!((d2 - 8.0).abs() < 0.1, "{d2}");
        // 模型过滤：无匹配成本 → None
        assert!(weekly_days_left(&conn, 20.0, Some(&resets), Some("opus")).is_none());
        // 活跃日不足 → None（过滤出 0 天）
        assert!(weekly_days_left(&conn, 1.0, Some(&resets), None).is_none()); // U<3
    }

    #[test]
    fn samples_dedupe_and_prune() {
        let conn = crate::store::open_memory().unwrap();
        let u = parse_usage(FIXTURE).unwrap();
        // 固定日期会随真实时间流逝掉进 14 天清理窗口，改用相对现在的时间戳
        let ts = (chrono::Utc::now() - chrono::Duration::hours(1))
            .format("%Y-%m-%d %H:%M:%S").to_string();
        record_samples(&conn, &u, &ts).unwrap();
        record_samples(&conn, &u, &ts).unwrap(); // 幂等
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM usage_samples", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2); // five_hour + seven_day
    }
}
