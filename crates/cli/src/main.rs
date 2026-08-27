use bookholder_core::{billing, ingest, pricing, queries, report, store, watcher};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "bookholder", about = "Claude Code token 成本统计（GUI 的可选命令行入口）")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 输出统计报告
    Report {
        #[arg(long)] json: bool,
        #[arg(long)] csv: bool,
        #[arg(long)] md: bool,
        /// 按项目显示名过滤（仅 csv 生效）
        #[arg(long)] project: Option<String>,
    },
    /// 全量重扫转录目录
    Backfill,
    /// 实时输出新事件（JSON 行）
    Live,
}

fn open() -> rusqlite::Connection {
    let conn = store::open_db(&store::default_db_path()).expect("open db");
    let _ = pricing::seed_snapshot(&conn);
    conn
}

fn billing_mode(conn: &rusqlite::Connection) -> String {
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
    billing::effective_mode(conn, &home)
}

fn main() {
    let cli = Cli::parse();
    let conn = open();
    let mode = billing_mode(&conn);
    match cli.cmd {
        Cmd::Report { json, csv, project, .. } => {
            // 报告前先增量扫一遍，保证数字最新
            ingest::scan_all(&conn, &store::claude_projects_dir(), &mode);
            let _ = pricing::reprice_null_costs(&conn);
            if json {
                println!("{}", report::json_report(&conn));
            } else if csv {
                let pid = project.and_then(|name| {
                    queries::project_rows(&conn).into_iter().find(|p| p.display_name == name).map(|p| p.id)
                });
                print!("{}", report::csv_events(&conn, pid));
            } else {
                print!("{}", report::markdown_report(&conn));
            }
        }
        Cmd::Backfill => {
            let st = ingest::scan_all(&conn, &store::claude_projects_dir(), &mode);
            let _ = pricing::reprice_null_costs(&conn);
            println!("added: {} skipped: {} bad: {}", st.added, st.skipped, st.bad);
        }
        Cmd::Live => {
            let dir = store::claude_projects_dir();
            ingest::scan_all(&conn, &dir, &mode);
            let mut last_id: i64 = conn
                .query_row("SELECT COALESCE(MAX(id),0) FROM usage_events", [], |r| r.get(0))
                .unwrap_or(0);
            let db_path = store::default_db_path();
            let mode2 = mode.clone();
            let _guard = watcher::watch_projects(&dir, move || {
                let Ok(c) = store::open_db(&db_path) else { return };
                ingest::scan_all(&c, &store::claude_projects_dir(), &mode2);
                let mut stmt = c.prepare(
                    "SELECT id, ts, model, is_sidechain, input_tokens, output_tokens, cost_usd
                     FROM usage_events WHERE id > ?1 ORDER BY id").unwrap();
                let rows: Vec<(i64, String, String, i64, i64, i64, Option<f64>)> = stmt
                    .query_map([last_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))
                    .unwrap().flatten().collect();
                for (id, ts, model, side, input, output, cost) in rows {
                    last_id = id;
                    println!("{}", serde_json::json!({
                        "ts": ts, "model": model, "is_sidechain": side != 0,
                        "input": input, "output": output, "cost_usd": cost
                    }));
                }
            }).expect("watch");
            eprintln!("watching {} … Ctrl-C 退出", dir.display());
            loop { std::thread::sleep(std::time::Duration::from_secs(3600)); }
        }
    }
}
