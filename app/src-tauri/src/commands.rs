use bookholder_core::{billing, estimate, ingest, limits, metrics, pricing, queries, report, store, subscription};
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
pub fn set_ui_prefs(db: State<Db>, theme: Option<String>, opacity: Option<f64>, lang: Option<String>) -> Result<(), String> {
    let conn = db.0.lock().unwrap();
    if let Some(l) = lang {
        match l.as_str() {
            "auto" => { let _ = conn.execute("DELETE FROM meta WHERE key='ui_lang'", []); }
            "zh" | "en" | "de" => store::meta_set(&conn, "ui_lang", &l).map_err(|e| e.to_string())?,
            _ => return Err(format!("unknown lang {l}")),
        }
    }
    if let Some(t) = theme {
        if t != "light" && t != "dark" {
            return Err(format!("unknown theme {t}"));
        }
        store::meta_set(&conn, "ui_theme", &t).map_err(|e| e.to_string())?;
    }
    if let Some(o) = opacity {
        if !(0.3..=1.0).contains(&o) {
            return Err(format!("opacity out of range: {o}"));
        }
        store::meta_set(&conn, "ui_float_opacity", &o.to_string()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn usage_limits(db: State<'_, Db>) -> Result<Value, String> {
    // 60 秒节流：新鲜缓存直接返回，避免每次悬浮窗刷新都打接口
    let cached: Option<String> = {
        let conn = db.0.lock().unwrap();
        let fresh = store::meta_get(&conn, "usage_cache_ts")
            .and_then(|t| chrono::NaiveDateTime::parse_from_str(&t, "%Y-%m-%d %H:%M:%S").ok())
            .map(|t| (chrono::Utc::now().naive_utc() - t).num_seconds() < 60)
            .unwrap_or(false);
        if fresh { store::meta_get(&conn, "usage_cache_json") } else { None }
    };
    let mut stale = false;
    let body = match cached {
        Some(b) => b,
        None => match limits::fetch_usage_json() { // 网络在锁外
            Ok(b) => {
                let conn = db.0.lock().unwrap();
                let now = now_utc();
                let _ = store::meta_set(&conn, "usage_cache_json", &b);
                let _ = store::meta_set(&conn, "usage_cache_ts", &now);
                if let Ok(ws) = limits::parse_limit_windows(&b) {
                    let _ = limits::record_window_samples(&conn, &ws, &now);
                }
                b
            }
            Err(e) => {
                // 钥匙串重签名后未授权等场景：退回上次成功的缓存（标记 stale），
                // 完全没有缓存才把错误抛给前端。
                let conn = db.0.lock().unwrap();
                match store::meta_get(&conn, "usage_cache_json") {
                    Some(b) => { stale = true; b }
                    None => return Err(e),
                }
            }
        },
    };
    let ws = limits::parse_limit_windows(&body)?;
    let conn = db.0.lock().unwrap();
    let rows: Vec<Value> = ws
        .iter()
        .map(|w| {
            let (eta_h, eta_days) = if w.kind == "session" {
                (limits::eta_hours(&conn, &w.key, w.utilization, 90), None)
            } else {
                // 周窗口：典型工作日续航；不可用时回退斜率法
                let days = limits::weekly_days_left(
                    &conn, w.utilization, w.resets_at.as_deref(), w.scope_label.as_deref());
                let fallback = if days.is_none() {
                    limits::eta_hours(&conn, &w.key, w.utilization, 1440)
                } else { None };
                (fallback, days)
            };
            json!({
                "key": w.key, "kind": w.kind, "scope": w.scope_label,
                "utilization": w.utilization, "resets_at": w.resets_at,
                "eta_h": eta_h, "eta_days": eta_days,
            })
        })
        .collect();
    Ok(json!({ "windows": rows, "stale": stale }))
}

#[tauri::command]
pub async fn estimate_repo(db: State<'_, Db>, path: String) -> Result<Value, String> {
    let conn = db.0.lock().unwrap();
    estimate::estimate(&conn, std::path::Path::new(&path))
        .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
}

#[tauri::command]
pub fn project_metrics(db: State<Db>, project_id: i64) -> Value {
    let conn = db.0.lock().unwrap();
    match metrics::latest_for(&conn, project_id) {
        Some((row, days)) => json!({ "latest": row, "days": days }),
        None => Value::Null,
    }
}

#[tauri::command]
pub fn project_hourly(db: State<Db>, project_id: i64) -> Value {
    let conn = db.0.lock().unwrap();
    serde_json::to_value(queries::hourly_last24_project(&conn, project_id)).unwrap_or(Value::Null)
}

#[tauri::command]
pub fn sessions_recent(db: State<Db>, limit: i64) -> Value {
    let conn = db.0.lock().unwrap();
    serde_json::to_value(queries::recent_sessions(&conn, limit)).unwrap_or(Value::Null)
}

#[tauri::command]
pub fn subscription_comparison(db: State<Db>) -> Value {
    let conn = db.0.lock().unwrap();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let tier = billing::detect_tier(&home());
    serde_json::to_value(subscription::comparison(&conn, &today, tier)).unwrap_or(Value::Null)
}

#[tauri::command]
pub fn set_subscription_fees(db: State<Db>, fees_json: String) -> Result<(), String> {
    let fees = subscription::parse_fees(&fees_json)?;
    let conn = db.0.lock().unwrap();
    subscription::save_fees(&conn, &fees).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn float_data(db: State<Db>) -> Value {
    let conn = db.0.lock().unwrap();
    let (t0, t1) = queries::today_range();
    let today = queries::totals(&conn, Some(&t0), Some(&t1), None);
    let ctx = queries::current_context(&conn);
    let proj_totals = ctx.as_ref().map(|c| queries::totals(&conn, None, None, Some(c.project_id)));
    let project_cost = proj_totals.as_ref().map(|t| t.cost_usd).unwrap_or(0.0);
    let project_tokens = proj_totals.as_ref()
        .map(|t| t.input + t.output + t.cache_read + t.cache_write)
        .unwrap_or(0);
    let project_hit_rate = proj_totals.as_ref()
        .map(|t| {
            let denom = t.input + t.cache_read + t.cache_write;
            if denom > 0 { t.cache_read as f64 / denom as f64 } else { 0.0 }
        })
        .unwrap_or(0.0);
    json!({
        "today_cost": today.cost_usd,
        "today_tokens": today.input + today.output + today.cache_read + today.cache_write,
        "project_cost": project_cost,
        "project_tokens": project_tokens,
        "project_hit_rate": project_hit_rate,
        "project_id": ctx.as_ref().map(|c| c.project_id),
        "project_name": ctx.as_ref().map(|c| c.project_name.clone()).unwrap_or_else(|| "—".into()),
        "model": ctx.as_ref().map(|c| c.model.clone()).unwrap_or_default(),
        "burn_rate": queries::burn_rate_per_hour(&conn),
        "burn_tokens": queries::burn_tokens_per_hour(&conn),
        "billing_mode": billing::effective_mode(&conn, &home()),
        "hourly": queries::hourly_last24(&conn),
        "active": queries::active_projects(&conn, 30),
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
        "cache_savings": queries::cache_savings(&conn),
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
        "theme": store::meta_get(&conn, "ui_theme").unwrap_or_else(|| "light".into()),
        "ui_lang": store::meta_get(&conn, "ui_lang"),
        "float_opacity": store::meta_get(&conn, "ui_float_opacity")
            .and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.95),
    })
}

#[tauri::command]
pub async fn refresh_prices(db: State<'_, Db>) -> Result<String, String> {
    // 网络请求在拿锁之前完成，避免长时间占用 Mutex<Connection> 阻塞其它命令。
    let prices = match pricing::fetch_remote_prices() {
        Ok(p) => p,
        Err(e) => {
            let conn = db.0.lock().unwrap();
            let _ = store::meta_set(&conn, "prices_last_status", &format!("error: {e}"));
            return Err(e);
        }
    };
    let conn = db.0.lock().unwrap();
    let now = now_utc();
    let result = (|| -> Result<String, String> {
        let n = pricing::upsert_prices(&conn, &prices, "litellm", &now).map_err(|e| e.to_string())?;
        let re = pricing::reprice_null_costs(&conn).map_err(|e| e.to_string())?;
        let _ = store::meta_set(&conn, "prices_last_fetch", &now);
        let _ = store::meta_set(&conn, "prices_last_status", "ok");
        Ok(format!("updated {n} models, repriced {re} events"))
    })();
    if let Err(e) = &result {
        let _ = store::meta_set(&conn, "prices_last_status", &format!("error: {e}"));
    }
    result
}

#[tauri::command]
pub async fn run_backfill(db: State<'_, Db>) -> Result<Value, ()> {
    let conn = db.0.lock().unwrap();
    let mode = billing::effective_mode(&conn, &home());
    let st = ingest::scan_all(&conn, &store::claude_projects_dir(), &mode);
    let _ = pricing::reprice_null_costs(&conn);
    Ok(serde_json::to_value(st).unwrap_or(Value::Null))
}

#[tauri::command]
pub async fn export_report(db: State<'_, Db>, kind: String, dest: String) -> Result<(), String> {
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
pub fn project_overview(db: State<Db>, project_id: i64) -> Value {
    let conn = db.0.lock().unwrap();
    json!({
        "daily": queries::project_daily(&conn, project_id, 30),
        "models": queries::project_models(&conn, project_id),
    })
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
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub fn open_dashboard(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}
