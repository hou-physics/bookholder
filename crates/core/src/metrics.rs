use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

/// 每项目每天一行的轻量代码指标快照——为跨项目成本估算（v2 校准）积累特征。
/// 冗余控制：同一 (project, date) 只写一次；单行只有 5 个标量字段。
#[derive(Debug, Clone, Serialize)]
pub struct MetricsRow {
    pub date: String,
    pub files: i64,
    pub code_bytes: i64,
    pub commits: Option<i64>,
    pub top_ext: String, // 如 "rs:41,ts:12,css:3"
}

const SKIP_DIRS: &[&str] = &[
    ".git", "node_modules", "target", "dist", ".venv", "venv",
    ".superpowers", ".next", "build", "__pycache__", ".cache",
];

const CODE_EXTS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "py", "go", "java", "kt", "swift", "c", "h",
    "cpp", "hpp", "cs", "rb", "php", "sh", "sql", "html", "css", "scss", "vue",
    "svelte", "toml", "yaml", "yml", "json", "md",
];

pub fn ensure_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS project_metrics (
           project_id INTEGER NOT NULL REFERENCES projects(id),
           date TEXT NOT NULL,
           files INTEGER NOT NULL,
           code_bytes INTEGER NOT NULL,
           commits INTEGER,
           top_ext TEXT NOT NULL DEFAULT '',
           PRIMARY KEY (project_id, date)
         );",
    )
}

fn snapshot_dir(root: &Path) -> (i64, i64, String) {
    let mut files = 0i64;
    let mut bytes = 0i64;
    let mut by_ext: HashMap<String, i64> = HashMap::new();
    let mut stack = vec![root.to_path_buf()];
    let mut visited = 0u32;
    while let Some(dir) = stack.pop() {
        visited += 1;
        if visited > 2000 {
            break; // 防御：异常巨大的目录树不无限走
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) && !name.starts_with('.') {
                    stack.push(p);
                }
            } else if let Some(ext) = p.extension().and_then(|x| x.to_str()) {
                let ext = ext.to_lowercase();
                if CODE_EXTS.contains(&ext.as_str()) {
                    files += 1;
                    bytes += e.metadata().map(|m| m.len() as i64).unwrap_or(0);
                    *by_ext.entry(ext).or_insert(0) += 1;
                }
            }
        }
    }
    let mut exts: Vec<(String, i64)> = by_ext.into_iter().collect();
    exts.sort_by(|a, b| b.1.cmp(&a.1));
    let top = exts
        .iter()
        .take(3)
        .map(|(e, n)| format!("{e}:{n}"))
        .collect::<Vec<_>>()
        .join(",");
    (files, bytes, top)
}

fn git_commit_count(root: &Path) -> Option<i64> {
    let out = std::process::Command::new("git")
        .args(["-C", &root.to_string_lossy(), "rev-list", "--count", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// 对每个 cwd 仍存在的项目，若今天还没有快照则采集一行。返回新写入的行数。
pub fn collect_all(conn: &Connection, today_local: &str) -> usize {
    let _ = ensure_table(conn);
    let projects: Vec<(i64, String)> = conn
        .prepare("SELECT id, cwd FROM projects WHERE cwd != ''")
        .and_then(|mut st| {
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map(|it| it.flatten().collect())
        })
        .unwrap_or_default();
    let mut written = 0;
    for (pid, cwd) in projects {
        let root = Path::new(&cwd);
        if !root.is_dir() {
            continue;
        }
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM project_metrics WHERE project_id = ?1 AND date = ?2",
                params![pid, today_local],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if exists {
            continue;
        }
        let (files, bytes, top) = snapshot_dir(root);
        let commits = git_commit_count(root);
        if conn
            .execute(
                "INSERT OR IGNORE INTO project_metrics (project_id, date, files, code_bytes, commits, top_ext)
                 VALUES (?1,?2,?3,?4,?5,?6)",
                params![pid, today_local, files, bytes, commits, top],
            )
            .is_ok()
        {
            written += 1;
        }
    }
    written
}

/// 项目最新一行快照 + 累计快照天数。
pub fn latest_for(conn: &Connection, project_id: i64) -> Option<(MetricsRow, i64)> {
    let _ = ensure_table(conn);
    let row = conn
        .query_row(
            "SELECT date, files, code_bytes, commits, top_ext FROM project_metrics
             WHERE project_id = ?1 ORDER BY date DESC LIMIT 1",
            [project_id],
            |r| {
                Ok(MetricsRow {
                    date: r.get(0)?,
                    files: r.get(1)?,
                    code_bytes: r.get(2)?,
                    commits: r.get(3)?,
                    top_ext: r.get(4)?,
                })
            },
        )
        .ok()?;
    let days: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM project_metrics WHERE project_id = ?1",
            [project_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    Some((row, days))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_once_per_day_and_counts_code() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("b.ts"), "export {};\n").unwrap();
        std::fs::write(dir.path().join("noise.bin"), [0u8; 10]).unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules/x")).unwrap();
        std::fs::write(dir.path().join("node_modules/x/c.js"), "ignored").unwrap();

        let conn = crate::store::open_memory().unwrap();
        conn.execute(
            "INSERT INTO projects (slug, cwd, display_name) VALUES ('-t', ?1, 't')",
            [dir.path().to_string_lossy()],
        )
        .unwrap();

        assert_eq!(collect_all(&conn, "2026-08-27"), 1);
        assert_eq!(collect_all(&conn, "2026-08-27"), 0); // 同日不重复
        assert_eq!(collect_all(&conn, "2026-08-28"), 1); // 新一天再采

        let (m, days) = latest_for(&conn, 1).unwrap();
        assert_eq!(m.files, 2); // .bin 与 node_modules 均被排除
        assert!(m.code_bytes > 0);
        assert_eq!(days, 2);
        assert!(m.top_ext.contains("rs:1") && m.top_ext.contains("ts:1"));
    }
}
