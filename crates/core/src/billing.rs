use rusqlite::Connection;
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BillingMode { Subscription, Api, Unknown }

impl BillingMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            BillingMode::Subscription => "subscription",
            BillingMode::Api => "api",
            BillingMode::Unknown => "unknown",
        }
    }
}

pub fn detect(home: &Path, env_api_key: Option<&str>) -> BillingMode {
    let claude_json = home.join(".claude.json");
    if let Ok(txt) = std::fs::read_to_string(&claude_json) {
        if let Ok(v) = serde_json::from_str::<Value>(&txt) {
            if v.get("oauthAccount").map(|o| o.is_object() && !o.as_object().unwrap().is_empty()).unwrap_or(false) {
                return BillingMode::Subscription;
            }
        }
    }
    if env_api_key.map(|k| !k.is_empty()).unwrap_or(false) {
        return BillingMode::Api;
    }
    if let Ok(txt) = std::fs::read_to_string(home.join(".claude/settings.json")) {
        if txt.contains("apiKeyHelper") {
            return BillingMode::Api;
        }
    }
    BillingMode::Unknown
}

pub fn effective_mode(conn: &Connection, home: &Path) -> String {
    if let Some(o) = crate::store::meta_get(conn, "billing_override") {
        if o == "subscription" || o == "api" { return o; }
    }
    detect(home, std::env::var("ANTHROPIC_API_KEY").ok().as_deref()).as_str().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn detects_subscription_from_oauth_account() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"a@b.c"}}"#).unwrap();
        assert!(matches!(detect(dir.path(), None), BillingMode::Subscription));
    }

    #[test]
    fn detects_api_from_api_key_helper() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".claude.json"), r#"{}"#).unwrap();
        fs::create_dir_all(dir.path().join(".claude")).unwrap();
        fs::write(dir.path().join(".claude/settings.json"), r#"{"apiKeyHelper":"/bin/key.sh"}"#).unwrap();
        assert!(matches!(detect(dir.path(), None), BillingMode::Api));
    }

    #[test]
    fn unknown_when_nothing_present() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(detect(dir.path(), None), BillingMode::Unknown));
    }

    #[test]
    fn override_wins() {
        let conn = crate::store::open_memory().unwrap();
        crate::store::meta_set(&conn, "billing_override", "api").unwrap();
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(effective_mode(&conn, dir.path()), "api");
    }
}
