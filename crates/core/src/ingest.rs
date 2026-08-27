use crate::parse::{parse_line, ParseOutcome};
use rusqlite::Connection;
use serde::Serialize;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Default, Clone, Serialize)]
pub struct IngestStats {
    pub added: u64,
    pub skipped: u64,
    pub bad: u64,
}

impl IngestStats {
    pub fn merge(&mut self, o: &IngestStats) {
        self.added += o.added;
        self.skipped += o.skipped;
        self.bad += o.bad;
    }
}

pub fn ingest_file(conn: &Connection, path: &Path, slug: &str, billing: &str) -> IngestStats {
    let mut st = IngestStats::default();
    let key = path.to_string_lossy().to_string();
    let mut offset = crate::store::get_offset(conn, &key);
    let Ok(mut f) = std::fs::File::open(path) else { return st };
    let len = f.metadata().map(|m| m.len() as i64).unwrap_or(0);
    if len < offset {
        offset = 0; // 文件被重建，重读（dedup 保证不重复计数）
    }
    if len == offset { return st; }
    if f.seek(SeekFrom::Start(offset as u64)).is_err() { return st; }
    let mut buf = String::new();
    if f.read_to_string(&mut buf).is_err() {
        // 非 UTF-8 边界等：按字节读再有损转换
        let mut bytes = vec![];
        let _ = f.seek(SeekFrom::Start(offset as u64));
        if f.read_to_end(&mut bytes).is_err() { return st; }
        buf = String::from_utf8_lossy(&bytes).into_owned();
    }
    // 只消费完整行
    let consumed_end = match buf.rfind('\n') {
        Some(i) => i + 1,
        None => { return st; } // 整段都是半行
    };
    for line in buf[..consumed_end].lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        match parse_line(line) {
            ParseOutcome::Event(e) => {
                let cost = crate::pricing::latest_price(conn, &e.model)
                    .map(|p| crate::pricing::cost_usd(&e, &p));
                match crate::store::record_event(conn, slug, &e, cost, billing) {
                    Ok(true) => st.added += 1,
                    Ok(false) => {} // 去重
                    Err(_) => st.bad += 1,
                }
            }
            ParseOutcome::Skipped => st.skipped += 1,
            ParseOutcome::Bad => st.bad += 1,
        }
    }
    let _ = crate::store::set_offset(conn, &key, offset + consumed_end as i64);
    let _ = crate::store::bump_counter(conn, "skip_lines", st.skipped as i64);
    let _ = crate::store::bump_counter(conn, "bad_lines", st.bad as i64);
    st
}

pub fn scan_all(conn: &Connection, projects_dir: &Path, billing: &str) -> IngestStats {
    let mut total = IngestStats::default();
    let Ok(entries) = std::fs::read_dir(projects_dir) else { return total };
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_dir() { continue; }
        let slug = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let Ok(files) = std::fs::read_dir(&p) else { continue };
        for f in files.flatten() {
            let fp = f.path();
            if fp.extension().map(|e| e == "jsonl").unwrap_or(false) {
                total.merge(&ingest_file(conn, &fp, &slug, billing));
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, OpenOptions};
    use std::io::Write;

    const L1: &str = r#"{"type":"assistant","uuid":"u1","requestId":"r1","sessionId":"s1","cwd":"/Users/me/alpha","isSidechain":false,"timestamp":"2026-08-27T01:00:00Z","message":{"id":"m1","model":"claude-fable-5","usage":{"input_tokens":10,"output_tokens":20}}}"#;
    const L2: &str = r#"{"type":"user","message":{}}"#;
    const L3: &str = r#"{"type":"assistant","uuid":"u2","requestId":"r2","sessionId":"s1","cwd":"/Users/me/alpha","isSidechain":true,"timestamp":"2026-08-27T01:01:00Z","message":{"id":"m2","model":"claude-haiku-4-5-20251001","usage":{"input_tokens":5,"output_tokens":7}}}"#;

    fn setup() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("-Users-me-alpha");
        fs::create_dir_all(&proj).unwrap();
        (dir, proj.join("sess1.jsonl"))
    }

    #[test]
    fn scan_ingests_and_counts() {
        let (dir, file) = setup();
        fs::write(&file, format!("{L1}\n{L2}\nnot-json\n")).unwrap();
        let conn = crate::store::open_memory().unwrap();
        let st = scan_all(&conn, dir.path(), "api");
        assert_eq!((st.added, st.skipped, st.bad), (1, 1, 1));
        let slug: String = conn.query_row("SELECT slug FROM projects", [], |r| r.get(0)).unwrap();
        assert_eq!(slug, "-Users-me-alpha");
    }

    #[test]
    fn incremental_reads_only_new_bytes() {
        let (dir, file) = setup();
        fs::write(&file, format!("{L1}\n")).unwrap();
        let conn = crate::store::open_memory().unwrap();
        assert_eq!(scan_all(&conn, dir.path(), "api").added, 1);
        assert_eq!(scan_all(&conn, dir.path(), "api").added, 0); // 无新内容
        let mut f = OpenOptions::new().append(true).open(&file).unwrap();
        writeln!(f, "{L3}").unwrap();
        let st = scan_all(&conn, dir.path(), "api");
        assert_eq!(st.added, 1);
        let side: i64 = conn.query_row(
            "SELECT COUNT(*) FROM usage_events WHERE is_sidechain=1", [], |r| r.get(0)).unwrap();
        assert_eq!(side, 1);
    }

    #[test]
    fn partial_last_line_is_not_consumed() {
        let (dir, file) = setup();
        fs::write(&file, format!("{L1}\n{}", &L3[..40])).unwrap(); // 半行
        let conn = crate::store::open_memory().unwrap();
        assert_eq!(scan_all(&conn, dir.path(), "api").added, 1);
        // 半行补全后能读到
        let mut f = OpenOptions::new().append(true).open(&file).unwrap();
        write!(f, "{}\n", &L3[40..]).unwrap();
        assert_eq!(scan_all(&conn, dir.path(), "api").added, 1);
    }

    #[test]
    fn shrunk_file_rereads_from_zero() {
        let (dir, file) = setup();
        fs::write(&file, format!("{L1}\n{L3}\n")).unwrap();
        let conn = crate::store::open_memory().unwrap();
        assert_eq!(scan_all(&conn, dir.path(), "api").added, 2);
        fs::write(&file, format!("{L1}\n")).unwrap(); // 缩水
        let st = scan_all(&conn, dir.path(), "api");
        assert_eq!(st.added, 0); // 重读但 dedup 挡住
    }
}
