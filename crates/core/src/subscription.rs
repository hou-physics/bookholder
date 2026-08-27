use chrono::NaiveDate;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// 一段订阅费率：从 `from`（含）起每月 `usd` 美元，直到下一段开始。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeePeriod {
    pub from: String, // YYYY-MM-DD
    pub usd: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubComparison {
    pub fees: Vec<FeePeriod>,
    pub window_start: Option<String>, // 最早订阅事件日期
    pub window_days: f64,
    pub actual_usd: f64,     // 订阅实付（按天折算跨段累加）
    pub equiv_usd: f64,      // 订阅会话的等值 API 成本
    pub api_usd: f64,        // API 会话的实付成本（如有）
    pub savings_usd: f64,    // equiv - actual
    pub leverage: Option<f64>, // equiv / actual
    pub month_equiv_usd: f64, // 本月（自然月）订阅等值成本
    pub month_fee_usd: Option<f64>, // 当前生效月费
    pub detected_tier: Option<String>,
}

const AVG_MONTH_DAYS: f64 = 30.4375;
pub const FEES_META_KEY: &str = "subscription_fees";

pub fn parse_fees(json: &str) -> Result<Vec<FeePeriod>, String> {
    let mut fees: Vec<FeePeriod> = serde_json::from_str(json).map_err(|e| e.to_string())?;
    for f in &fees {
        NaiveDate::parse_from_str(&f.from, "%Y-%m-%d").map_err(|e| format!("{}: {e}", f.from))?;
        if f.usd < 0.0 {
            return Err(format!("negative fee: {}", f.usd));
        }
    }
    fees.sort_by(|a, b| a.from.cmp(&b.from));
    Ok(fees)
}

pub fn load_fees(conn: &Connection) -> Vec<FeePeriod> {
    crate::store::meta_get(conn, FEES_META_KEY)
        .and_then(|j| parse_fees(&j).ok())
        .unwrap_or_default()
}

pub fn save_fees(conn: &Connection, fees: &[FeePeriod]) -> rusqlite::Result<()> {
    crate::store::meta_set(conn, FEES_META_KEY, &serde_json::to_string(fees).unwrap_or_default())
}

/// 按天折算的订阅实付：每段费率覆盖 [window_start, today] 与该段区间的交集天数。
fn prorated_spend(fees: &[FeePeriod], window_start: NaiveDate, today: NaiveDate) -> f64 {
    let mut total = 0.0;
    for (i, f) in fees.iter().enumerate() {
        let Ok(seg_start) = NaiveDate::parse_from_str(&f.from, "%Y-%m-%d") else { continue };
        let seg_end = fees
            .get(i + 1)
            .and_then(|n| NaiveDate::parse_from_str(&n.from, "%Y-%m-%d").ok())
            .unwrap_or(today);
        let start = seg_start.max(window_start);
        let end = seg_end.min(today);
        if end > start {
            let days = (end - start).num_days() as f64;
            total += days * f.usd / AVG_MONTH_DAYS;
        }
    }
    total
}

fn current_fee(fees: &[FeePeriod], today: NaiveDate) -> Option<f64> {
    fees.iter()
        .filter(|f| {
            NaiveDate::parse_from_str(&f.from, "%Y-%m-%d").map(|d| d <= today).unwrap_or(false)
        })
        .last()
        .map(|f| f.usd)
}

/// `today_local` 形如 "2026-08-27"（注入以便测试）。
pub fn comparison(conn: &Connection, today_local: &str, detected_tier: Option<String>) -> SubComparison {
    let fees = load_fees(conn);
    let today = NaiveDate::parse_from_str(today_local, "%Y-%m-%d")
        .unwrap_or_else(|_| NaiveDate::from_ymd_opt(1970, 1, 1).unwrap());

    let (equiv_usd, window_start): (f64, Option<String>) = conn
        .query_row(
            "SELECT COALESCE(SUM(e.cost_usd),0), MIN(date(e.ts))
             FROM usage_events e JOIN sessions s ON s.id = e.session_id
             WHERE s.billing_mode = 'subscription'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((0.0, None));
    let api_usd: f64 = conn
        .query_row(
            "SELECT COALESCE(SUM(e.cost_usd),0)
             FROM usage_events e JOIN sessions s ON s.id = e.session_id
             WHERE s.billing_mode = 'api'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0.0);
    let month_equiv_usd: f64 = conn
        .query_row(
            "SELECT COALESCE(SUM(e.cost_usd),0)
             FROM usage_events e JOIN sessions s ON s.id = e.session_id
             WHERE s.billing_mode = 'subscription'
               AND date(e.ts,'localtime') >= date('now','localtime','start of month')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0.0);

    let ws = window_start
        .as_deref()
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
    let (actual_usd, window_days) = match ws {
        Some(start) if !fees.is_empty() => (
            prorated_spend(&fees, start, today),
            ((today - start).num_days() as f64).max(0.0),
        ),
        Some(start) => (0.0, ((today - start).num_days() as f64).max(0.0)),
        None => (0.0, 0.0),
    };
    let leverage = if actual_usd > 0.0 { Some(equiv_usd / actual_usd) } else { None };

    SubComparison {
        month_fee_usd: current_fee(&fees, today),
        fees,
        window_start,
        window_days,
        actual_usd,
        equiv_usd,
        api_usd,
        savings_usd: equiv_usd - actual_usd,
        leverage,
        month_equiv_usd,
        detected_tier,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::UsageEvent;

    fn ev(key: &str, ts: &str, cost: f64, billing: &str, conn: &Connection) {
        let e = UsageEvent {
            dedup_key: key.into(), ts: ts.into(), session_id: format!("s-{billing}"),
            cwd: "/u/p".into(), model: "claude-fable-5".into(), is_sidechain: false,
            input_tokens: 1, output_tokens: 1, thinking_tokens: 0,
            cache_write_5m: 0, cache_write_1h: 0, cache_read: 0,
        };
        crate::store::record_event(conn, "-p", &e, Some(cost), billing).unwrap();
    }

    #[test]
    fn fees_parse_sort_and_reject_bad() {
        let fees = parse_fees(r#"[{"from":"2026-08-27","usd":200},{"from":"1970-01-01","usd":100}]"#).unwrap();
        assert_eq!(fees[0].usd, 100.0);
        assert_eq!(fees[1].from, "2026-08-27");
        assert!(parse_fees(r#"[{"from":"not-a-date","usd":1}]"#).is_err());
        assert!(parse_fees(r#"[{"from":"2026-01-01","usd":-5}]"#).is_err());
    }

    #[test]
    fn prorated_spend_across_fee_change() {
        // 窗口 2026-07-14 起，8-17 换档：100 美元 34 天 + 200 美元 10 天
        let fees = parse_fees(r#"[{"from":"1970-01-01","usd":100},{"from":"2026-08-17","usd":200}]"#).unwrap();
        let start = NaiveDate::from_ymd_opt(2026, 7, 14).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 8, 27).unwrap();
        let spend = prorated_spend(&fees, start, today);
        let expect = 34.0 * 100.0 / 30.4375 + 10.0 * 200.0 / 30.4375;
        assert!((spend - expect).abs() < 1e-9, "{spend} vs {expect}");
    }

    #[test]
    fn comparison_splits_modes_and_computes_leverage() {
        let conn = crate::store::open_memory().unwrap();
        save_fees(&conn, &parse_fees(r#"[{"from":"1970-01-01","usd":100}]"#).unwrap()).unwrap();
        ev("k1", "2026-08-01T00:00:00Z", 300.0, "subscription", &conn);
        ev("k2", "2026-08-10T00:00:00Z", 700.0, "subscription", &conn);
        ev("k3", "2026-08-10T00:00:00Z", 42.0, "api", &conn);
        let c = comparison(&conn, "2026-08-31", Some("default_claude_max_20x".into()));
        assert!((c.equiv_usd - 1000.0).abs() < 1e-9);
        assert!((c.api_usd - 42.0).abs() < 1e-9);
        assert_eq!(c.window_start.as_deref(), Some("2026-08-01"));
        let expect_actual = 30.0 * 100.0 / 30.4375;
        assert!((c.actual_usd - expect_actual).abs() < 1e-9);
        assert!((c.savings_usd - (1000.0 - expect_actual)).abs() < 1e-9);
        assert!((c.leverage.unwrap() - 1000.0 / expect_actual).abs() < 1e-9);
        assert_eq!(c.month_fee_usd, Some(100.0));
        assert_eq!(c.detected_tier.as_deref(), Some("default_claude_max_20x"));
    }

    #[test]
    fn comparison_without_fees_or_events_is_zeroed() {
        let conn = crate::store::open_memory().unwrap();
        let c = comparison(&conn, "2026-08-27", None);
        assert_eq!(c.actual_usd, 0.0);
        assert!(c.leverage.is_none());
        assert!(c.window_start.is_none());
        assert!(c.fees.is_empty());
    }
}
