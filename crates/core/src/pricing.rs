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

/// 同名模型在多个 provider 前缀下重复出现时的可信度分级：
/// 0 = 裸 key（无 '/'，代表 Anthropic 官方 API 命名，最可信）
/// 1 = 紧邻 "anthropic/" 段的转发商（如 openrouter/anthropic/claude-x），次可信
/// 2 = 其它 provider 前缀（azure_ai/、vertex_ai/、bedrock/...），最不可信
fn priority_tier(key: &str) -> u8 {
    if !key.contains('/') { return 0; }
    let parent = key.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
    let parent_last = parent.rsplit('/').next().unwrap_or(parent);
    if parent_last == "anthropic" { 1 } else { 2 }
}

pub fn parse_litellm(json: &str) -> Vec<ModelPrice> {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return vec![] };
    let Some(obj) = v.as_object() else { return vec![] };
    let mut out: Vec<ModelPrice> = vec![];
    // BTreeMap 的迭代顺序是键的字典序，不代表可信度（例如 "azure_ai/claude-x" 会排在裸 key
    // "claude-x" 之前）。按 priority_tier 分三轮扫描，保证已被更高优先级收录的模型名，
    // 不会被后一轮的低优先级同名条目覆盖。
    for tier in 0..3u8 {
        for (key, m) in obj {
            if priority_tier(key) != tier { continue; }
            let name = key.rsplit('/').next().unwrap_or(key).to_string();
            if !name.starts_with("claude") { continue; }
            // vertex_ai 的别名 key 形如 "claude-opus-4-1@20250805" / "claude-opus-4-6@default"，
            // 事件里的 model 字段从不带 '@'，这类键在 lookup() 中永远匹配不到，跳过以免污染价格表。
            if name.contains('@') { continue; }
            let f = |k: &str| m.get(k).and_then(Value::as_f64);
            let (Some(input), Some(output)) = (f("input_cost_per_token"), f("output_cost_per_token")) else { continue };
            if input <= 0.0 || output <= 0.0 { continue; }
            let w5 = f("cache_creation_input_token_cost").unwrap_or(input * 1.25);
            let w1 = f("cache_creation_input_token_cost_above_1hr").unwrap_or(input * 2.0);
            let read = f("cache_read_input_token_cost").unwrap_or(input * 0.1);
            if out.iter().any(|p: &ModelPrice| p.model == name) { continue; } // 已被更高优先级收录
            out.push(ModelPrice {
                model: name, input_cost: input, output_cost: output,
                cache_read_cost: read, cache_write_5m_cost: w5, cache_write_1h_cost: w1,
            });
        }
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
            // INSERT OR IGNORE 的 UNIQUE(model, effective_from) 冲突会静默丢弃这次写入：
            // 如果两次调用用了同一个 `now`（同一秒内的两次抓取），第二次的新价格就会被无声吞掉。
            // 用 ON CONFLICT DO UPDATE 让同一 (model, effective_from) 的行直接被新价格覆盖，
            // 并用 execute() 返回的真实受影响行数计数，而不是盲目按 `changed` 计数。
            let n = conn.execute(
                "INSERT INTO model_prices
                   (model, input_cost, output_cost, cache_read_cost, cache_write_5m_cost, cache_write_1h_cost, effective_from, source)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT(model, effective_from) DO UPDATE SET
                   input_cost = excluded.input_cost,
                   output_cost = excluded.output_cost,
                   cache_read_cost = excluded.cache_read_cost,
                   cache_write_5m_cost = excluded.cache_write_5m_cost,
                   cache_write_1h_cost = excluded.cache_write_1h_cost,
                   source = excluded.source",
                params![p.model, p.input_cost, p.output_cost, p.cache_read_cost,
                        p.cache_write_5m_cost, p.cache_write_1h_cost, now, source],
            )?;
            inserted += n;
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
            // 放宽为 > 0.0：LiteLLM 快照中 claude-3-opus-20240229 自带显式
            // cache_creation_input_token_cost_above_1hr=6e-6，低于其 5m 写入价 1.875e-5
            // （正常模型的 fallback 关系 input*2.0 > input*1.25 是成立的，这是上游数据的个例异常，
            // 非本模块解析逻辑的 bug；已用脚本核实 70 个模型中仅此 1 个违反 1h > 5m）。
            assert!(p.cache_write_1h_cost > 0.0, "{} 1h > 0", p.model);
            assert!(!p.model.contains('/'), "键应剥掉 provider 前缀: {}", p.model);
            // vertex_ai 别名 key（如 claude-opus-4-1@20250805 / claude-opus-4-6@default）
            // 应被跳过：事件里的 model 字段从不带 '@'，这类键在 lookup() 里永远匹配不到。
            assert!(!p.model.contains('@'), "vertex 别名键应被跳过: {}", p.model);
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
    fn upsert_same_timestamp_updates_instead_of_silently_dropping() {
        // 回归测试：INSERT OR IGNORE 在 (model, effective_from) 已存在时会静默丢弃新价格。
        // 同一秒内两次调用（例如快速连续抓取两次）用了同一个 `now`，第二次的新价必须覆盖旧价，
        // 而不是被忽略。
        let conn = crate::store::open_memory().unwrap();
        let p1 = vec![ModelPrice { model: "claude-y".into(), input_cost: 1e-6, output_cost: 2e-6,
            cache_read_cost: 1e-7, cache_write_5m_cost: 1.25e-6, cache_write_1h_cost: 2e-6 }];
        assert_eq!(upsert_prices(&conn, &p1, "test", "2026-01-01 00:00:00").unwrap(), 1);
        let p2 = vec![ModelPrice { input_cost: 9e-6, ..p1[0].clone() }];
        assert_eq!(upsert_prices(&conn, &p2, "test", "2026-01-01 00:00:00").unwrap(), 1);
        let latest = latest_price(&conn, "claude-y").unwrap();
        assert!((latest.input_cost - 9e-6).abs() < 1e-12, "同一 effective_from 的新价格应覆盖旧价格");
    }

    #[test]
    fn bare_key_wins_over_provider_prefixed_duplicate() {
        // 同一模型在多个 provider 前缀下重复出现时，裸 key（无 '/'）代表 Anthropic 官方命名，
        // 应该优先于 azure_ai/ 等转发商前缀，而不是取决于 BTreeMap 的字典序。
        let json = r#"{
            "azure_ai/claude-x": {
                "input_cost_per_token": 1e-5,
                "output_cost_per_token": 2e-5
            },
            "claude-x": {
                "input_cost_per_token": 1e-6,
                "output_cost_per_token": 2e-6
            }
        }"#;
        let prices = parse_litellm(json);
        assert_eq!(prices.len(), 1);
        let p = lookup(&prices, "claude-x").unwrap();
        assert!((p.input_cost - 1e-6).abs() < 1e-12, "裸 key 的价格应该胜出，实际 = {}", p.input_cost);
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
