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
