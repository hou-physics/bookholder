use assert_cmd::Command;

#[test]
fn report_json_on_empty_db() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("t.db");
    let claude = dir.path().join("projects");
    std::fs::create_dir_all(&claude).unwrap();
    Command::cargo_bin("bookholder").unwrap()
        .env("BOOKHOLDER_DB", &db)
        .env("BOOKHOLDER_CLAUDE_DIR", &claude)
        .args(["report", "--json"])
        .assert()
        .success()
        .stdout(predicates::str::contains("\"totals\""));
}

#[test]
fn backfill_ingests_fixture() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("t.db");
    let proj = dir.path().join("projects/-u-alpha");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(proj.join("s.jsonl"),
        r#"{"type":"assistant","uuid":"u1","requestId":"r1","sessionId":"s1","cwd":"/u/alpha","isSidechain":false,"timestamp":"2026-08-27T01:00:00Z","message":{"id":"m1","model":"claude-fable-5","usage":{"input_tokens":10,"output_tokens":20}}}
"#).unwrap();
    Command::cargo_bin("bookholder").unwrap()
        .env("BOOKHOLDER_DB", &db)
        .env("BOOKHOLDER_CLAUDE_DIR", dir.path().join("projects"))
        .arg("backfill")
        .assert()
        .success()
        .stdout(predicates::str::contains("added: 1"));
}
