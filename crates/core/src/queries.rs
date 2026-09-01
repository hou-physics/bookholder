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

pub fn project_daily(conn: &Connection, project_id: i64, days: i64) -> Vec<DailyModelRow> {
    let mut stmt = conn.prepare(
        "SELECT date(ts, 'localtime') d, model, COALESCE(SUM(cost_usd),0)
         FROM usage_events
         WHERE ts >= datetime('now', ?1) AND project_id = ?2
         GROUP BY d, model ORDER BY d",
    ).unwrap();
    stmt.query_map(params![format!("-{days} days"), project_id], |r| Ok(DailyModelRow {
        date: r.get(0)?, model: r.get(1)?, cost_usd: r.get(2)?,
    })).map(|it| it.flatten().collect()).unwrap_or_default()
}

#[derive(Debug, Clone, Serialize)]
pub struct HourRow { pub hour: String, pub main_cost: f64, pub side_cost: f64 }

pub fn hourly_last24(conn: &Connection) -> Vec<HourRow> {
    hourly_last24_impl(conn, None)
}

/// 单项目版：悬浮窗"展开当前任务明细"用。
pub fn hourly_last24_project(conn: &Connection, project_id: i64) -> Vec<HourRow> {
    hourly_last24_impl(conn, Some(project_id))
}

fn hourly_last24_impl(conn: &Connection, project_id: Option<i64>) -> Vec<HourRow> {
    let mut stmt = conn.prepare(
        "SELECT strftime('%Y-%m-%d %H:00', ts, 'localtime') h,
                COALESCE(SUM(CASE WHEN is_sidechain=0 THEN cost_usd ELSE 0 END),0),
                COALESCE(SUM(CASE WHEN is_sidechain=1 THEN cost_usd ELSE 0 END),0)
         FROM usage_events
         WHERE ts >= datetime('now', '-24 hours') AND (?1 IS NULL OR project_id = ?1)
         GROUP BY h ORDER BY h",
    ).unwrap();
    let rows: Vec<HourRow> = stmt.query_map(params![project_id], |r| Ok(HourRow { hour: r.get(0)?, main_cost: r.get(1)?, side_cost: r.get(2)? }))
        .map(|it| it.flatten().collect()).unwrap_or_default();

    // 零填充到固定 24 小时，缺失小时补 0，保证悬浮窗 sparkline 长度恒定
    use chrono::{Duration, Local};
    let now = Local::now();
    (0..24)
        .map(|i| {
            let label = (now - Duration::hours(23 - i)).format("%Y-%m-%d %H:00").to_string();
            rows.iter().find(|r| r.hour == label).cloned().unwrap_or(HourRow {
                hour: label, main_cost: 0.0, side_cost: 0.0,
            })
        })
        .collect()
}

/// 最近 30 分钟 token 总量 ×2 —— 与 burn_rate_per_hour 同窗口的 token 口径。
pub fn burn_tokens_per_hour(conn: &Connection) -> i64 {
    let half: i64 = conn.query_row(
        "SELECT COALESCE(SUM(input_tokens + output_tokens + cache_read + cache_write_5m + cache_write_1h),0)
         FROM usage_events WHERE ts >= datetime('now','-30 minutes')",
        [], |r| r.get(0),
    ).unwrap_or(0);
    half * 2
}

pub fn burn_rate_per_hour(conn: &Connection) -> f64 {
    let half_hour: f64 = conn.query_row(
        "SELECT COALESCE(SUM(cost_usd),0) FROM usage_events WHERE ts >= datetime('now','-30 minutes')",
        [], |r| r.get(0),
    ).unwrap_or(0.0);
    half_hour * 2.0
}

/// 缓存节省统计：实际成本 vs "假设完全无缓存、全部按 input 全价" 的对照。
#[derive(Debug, Clone, Serialize)]
pub struct CacheSavings {
    pub actual_usd: f64,
    pub no_cache_usd: f64,
    pub saved_usd: f64,
    pub cache_read_tokens: i64,
}

pub fn cache_savings(conn: &Connection) -> CacheSavings {
    let rows: Vec<(String, i64, i64, i64, i64, i64, f64)> = conn
        .prepare(
            "SELECT model, SUM(input_tokens), SUM(output_tokens), SUM(cache_read),
                    SUM(cache_write_5m), SUM(cache_write_1h), COALESCE(SUM(cost_usd),0)
             FROM usage_events GROUP BY model",
        )
        .and_then(|mut st| {
            st.query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
            })
            .map(|it| it.flatten().collect())
        })
        .unwrap_or_default();
    let mut out = CacheSavings { actual_usd: 0.0, no_cache_usd: 0.0, saved_usd: 0.0, cache_read_tokens: 0 };
    for (model, inp, outp, cr, w5, w1, cost) in rows {
        let Some(p) = crate::pricing::latest_price(conn, &model) else { continue };
        out.actual_usd += cost;
        out.no_cache_usd += (inp + cr + w5 + w1) as f64 * p.input_cost + outp as f64 * p.output_cost;
        out.cache_read_tokens += cr;
    }
    out.saved_usd = out.no_cache_usd - out.actual_usd;
    out
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

pub fn project_models(conn: &Connection, project_id: i64) -> Vec<ModelSplitRow> {
    let mut stmt = conn.prepare(
        "SELECT model, COALESCE(SUM(cost_usd),0), COALESCE(SUM(input_tokens),0),
                COALESCE(SUM(output_tokens),0), COUNT(*)
         FROM usage_events WHERE project_id = ?1
         GROUP BY model ORDER BY 2 DESC",
    ).unwrap();
    stmt.query_map([project_id], |r| Ok(ModelSplitRow {
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
pub struct ActiveProjectRow {
    pub project_id: i64,
    pub project_name: String,
    pub recent_cost: f64,
    pub total_cost: f64,
    pub total_tokens: i64,
    pub hit_rate: f64, // cache_read / (input+cache_read+cache_write)，0–1
    pub last_model: String,
}

/// 最近 `mins` 分钟内有事件的项目（并发任务切换视图），按窗口内消耗降序。
pub fn active_projects(conn: &Connection, mins: i64) -> Vec<ActiveProjectRow> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.display_name, COALESCE(SUM(e.cost_usd),0),
                (SELECT COALESCE(SUM(cost_usd),0) FROM usage_events WHERE project_id = p.id),
                (SELECT COALESCE(SUM(input_tokens + output_tokens + cache_read + cache_write_5m + cache_write_1h),0) FROM usage_events WHERE project_id = p.id),
                (SELECT COALESCE(CAST(SUM(cache_read) AS REAL) / NULLIF(SUM(input_tokens + cache_read + cache_write_5m + cache_write_1h), 0), 0) FROM usage_events WHERE project_id = p.id),
                (SELECT model FROM usage_events WHERE project_id = p.id ORDER BY ts DESC LIMIT 1)
         FROM usage_events e JOIN projects p ON p.id = e.project_id
         WHERE e.ts >= datetime('now', ?1)
         GROUP BY p.id ORDER BY 3 DESC LIMIT 5",
    ).unwrap();
    stmt.query_map([format!("-{mins} minutes")], |r| Ok(ActiveProjectRow {
        project_id: r.get(0)?, project_name: r.get(1)?, recent_cost: r.get(2)?,
        total_cost: r.get(3)?, total_tokens: r.get(4)?, hit_rate: r.get(5)?, last_model: r.get(6)?,
    })).map(|it| it.flatten().collect()).unwrap_or_default()
}

#[derive(Debug, Clone, Serialize)]
pub struct RecentSessionRow {
    pub id: i64, pub session_id: String, pub project_name: String, pub started_at: String,
    pub ended_at: String, pub billing_mode: String, pub cost_usd: f64, pub events: i64,
    pub side_cost: f64,
}

/// 跨项目按开始时间倒序的会话流水（会话明细页）。
pub fn recent_sessions(conn: &Connection, limit: i64) -> Vec<RecentSessionRow> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.session_id, p.display_name, COALESCE(s.started_at,''), COALESCE(s.ended_at,''),
                COALESCE(s.billing_mode,'unknown'), COALESCE(SUM(e.cost_usd),0), COUNT(e.id),
                COALESCE(SUM(CASE WHEN e.is_sidechain=1 THEN e.cost_usd ELSE 0 END),0)
         FROM sessions s
         JOIN projects p ON p.id = s.project_id
         LEFT JOIN usage_events e ON e.session_id = s.id
         GROUP BY s.id ORDER BY s.started_at DESC LIMIT ?1",
    ).unwrap();
    stmt.query_map([limit], |r| Ok(RecentSessionRow {
        id: r.get(0)?, session_id: r.get(1)?, project_name: r.get(2)?, started_at: r.get(3)?,
        ended_at: r.get(4)?, billing_mode: r.get(5)?, cost_usd: r.get(6)?, events: r.get(7)?,
        side_cost: r.get(8)?,
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
    fn active_projects_only_within_window() {
        let conn = crate::store::open_memory().unwrap();
        seed(&conn); // 种子数据都在过去，不算活跃
        let mk = |key: &str, mins_ago: i64, cost: f64, sess: &str, cwd: &str| {
            let ts = (chrono::Utc::now() - chrono::Duration::minutes(mins_ago))
                .format("%Y-%m-%dT%H:%M:%SZ").to_string();
            let e = crate::model::UsageEvent {
                dedup_key: key.into(), ts, session_id: sess.into(), cwd: cwd.into(),
                model: "claude-fable-5".into(), is_sidechain: false,
                input_tokens: 1, output_tokens: 1, thinking_tokens: 0,
                cache_write_5m: 0, cache_write_1h: 0, cache_read: 0,
            };
            crate::store::record_event(&conn, &format!("-{}", cwd.rsplit('/').next().unwrap()), &e, Some(cost), "subscription").unwrap();
        };
        mk("a1", 5, 2.0, "sa", "/u/hot");
        mk("a2", 10, 1.0, "sa", "/u/hot");
        mk("b1", 20, 0.5, "sb", "/u/warm");
        mk("c1", 90, 9.0, "sc", "/u/cold"); // 窗口外
        let act = active_projects(&conn, 30);
        assert_eq!(act.len(), 2);
        assert_eq!(act[0].project_name, "hot");
        assert!((act[0].recent_cost - 3.0).abs() < 1e-9);
        assert!((act[0].total_cost - 3.0).abs() < 1e-9); // hot 项目全部消耗都在窗口内
        assert_eq!(act[0].last_model, "claude-fable-5");
        assert_eq!(act[1].project_name, "warm");
        assert!(act[0].project_id > 0);
    }

    #[test]
    fn cache_savings_vs_full_price() {
        let conn = crate::store::open_memory().unwrap();
        crate::pricing::upsert_prices(&conn, &[crate::pricing::ModelPrice {
            model: "claude-x".into(), input_cost: 10e-6, output_cost: 50e-6,
            cache_read_cost: 1e-6, cache_write_5m_cost: 12.5e-6, cache_write_1h_cost: 20e-6,
        }], "test", "2026-01-01 00:00:00").unwrap();
        let e = crate::model::UsageEvent {
            dedup_key: "k".into(), ts: "2026-08-27T00:00:00Z".into(), session_id: "s".into(),
            cwd: "/p".into(), model: "claude-x".into(), is_sidechain: false,
            input_tokens: 1000, output_tokens: 100, thinking_tokens: 0,
            cache_write_5m: 2000, cache_write_1h: 0, cache_read: 100_000,
        };
        // 实际成本 = 1000*10 + 100*50 + 2000*12.5 + 100000*1 (µ$) = 0.14
        let cost = crate::pricing::cost_usd(&e, &crate::pricing::latest_price(&conn, "claude-x").unwrap());
        crate::store::record_event(&conn, "-p", &e, Some(cost), "subscription").unwrap();
        let cs = cache_savings(&conn);
        assert!((cs.actual_usd - 0.14).abs() < 1e-9, "{}", cs.actual_usd);
        // 无缓存 = (1000+2000+100000)*10 + 100*50 (µ$) = 1.035
        assert!((cs.no_cache_usd - 1.035).abs() < 1e-9, "{}", cs.no_cache_usd);
        assert!((cs.saved_usd - 0.895).abs() < 1e-9);
        assert_eq!(cs.cache_read_tokens, 100_000);
    }

    #[test]
    fn recent_sessions_flat_across_projects_newest_first() {
        let conn = crate::store::open_memory().unwrap();
        seed(&conn);
        let rows = recent_sessions(&conn, 10);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].project_name, "beta"); // s2 起于 08-27，晚于 s1
        assert_eq!(rows[1].project_name, "alpha");
        assert_eq!(rows[1].events, 2);
        assert!((rows[1].side_cost - 0.2).abs() < 1e-9);
        assert_eq!(recent_sessions(&conn, 1).len(), 1);
    }

    #[test]
    fn burn_rate_zero_on_old_data() {
        let conn = crate::store::open_memory().unwrap();
        seed(&conn);
        assert_eq!(burn_rate_per_hour(&conn), 0.0); // 种子数据都在过去
    }

    #[test]
    fn project_scoped_daily_and_model_split() {
        let conn = crate::store::open_memory().unwrap();
        seed(&conn);
        let pid_a: i64 = conn.query_row("SELECT id FROM projects WHERE slug='-a'", [], |r| r.get(0)).unwrap();
        let pid_b: i64 = conn.query_row("SELECT id FROM projects WHERE slug='-b'", [], |r| r.get(0)).unwrap();

        // project -a 有两条事件（两种模型），project -b 有一条
        let daily_a = project_daily(&conn, pid_a, 400);
        assert_eq!(daily_a.len(), 2);
        assert!(daily_a.iter().all(|r| r.date == "2026-08-26"));

        let daily_b = project_daily(&conn, pid_b, 400);
        assert_eq!(daily_b.len(), 1);
        assert_eq!(daily_b[0].model, "claude-fable-5");

        let models_a = project_models(&conn, pid_a);
        assert_eq!(models_a.len(), 2);
        let models_b = project_models(&conn, pid_b);
        assert_eq!(models_b.len(), 1);
        assert_eq!(models_b[0].events, 1);
    }

    #[test]
    fn hourly_last24_zero_fills_to_24_rows() {
        let conn = crate::store::open_memory().unwrap();
        // 空数据库
        let rows = hourly_last24(&conn);
        assert_eq!(rows.len(), 24);
        assert!(rows.iter().all(|r| r.main_cost == 0.0 && r.side_cost == 0.0));
    }

    #[test]
    fn hourly_project_filters_and_fills_24() {
        let conn = crate::store::open_memory().unwrap();
        let ts = (chrono::Utc::now() - chrono::Duration::minutes(10))
            .format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let mk = |key: &str, cwd: &str, cost: f64| crate::model::UsageEvent {
            dedup_key: key.into(), ts: ts.clone(), session_id: format!("s-{key}"), cwd: cwd.into(),
            model: "claude-fable-5".into(), is_sidechain: false,
            input_tokens: 1, output_tokens: 1, thinking_tokens: 0,
            cache_write_5m: 0, cache_write_1h: 0, cache_read: 0,
        };
        crate::store::record_event(&conn, "-a", &mk("k1", "/u/a", 5.0), Some(5.0), "subscription").unwrap();
        crate::store::record_event(&conn, "-b", &mk("k2", "/u/b", 3.0), Some(3.0), "subscription").unwrap();
        let pid_a: i64 = conn.query_row("SELECT id FROM projects WHERE slug='-a'", [], |r| r.get(0)).unwrap();
        let rows = hourly_last24_project(&conn, pid_a);
        assert_eq!(rows.len(), 24);
        let total: f64 = rows.iter().map(|r| r.main_cost + r.side_cost).sum();
        assert!((total - 5.0).abs() < 1e-9); // 只含项目 a，不含 b 的 3.0
    }
}
