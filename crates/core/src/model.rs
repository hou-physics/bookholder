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
