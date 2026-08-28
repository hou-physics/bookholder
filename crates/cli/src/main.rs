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
    /// 估算一个仓库的开发成本（本地路径或 git URL）
    Estimate {
        /// 本地目录或 https/git URL
        target: String,
        #[arg(long)] json: bool,
    },
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
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let _ = bookholder_core::metrics::collect_all(&conn, &today);
            println!("added: {} skipped: {} bad: {}", st.added, st.skipped, st.bad);
        }
        Cmd::Estimate { target, json } => {
            let (path, _tmp);
            if target.starts_with("http") || target.starts_with("git@") {
                let tmp = tempfile::tempdir().expect("tempdir");
                eprintln!("cloning {target} …");
                let ok = std::process::Command::new("git")
                    .args(["clone", "--quiet", &target, &tmp.path().to_string_lossy()])
                    .status().map(|s| s.success()).unwrap_or(false);
                if !ok { eprintln!("clone failed"); std::process::exit(1); }
                path = tmp.path().to_path_buf();
                _tmp = Some(tmp);
            } else {
                path = std::path::PathBuf::from(&target);
                _tmp = None;
            }
            match bookholder_core::estimate::estimate(&conn, &path) {
                Ok(e) => {
                    if json {
                        println!("{}", serde_json::to_string_pretty(&e).unwrap());
                    } else {
                        println!("仓库: {target}");
                        println!("  翻动行: {}  最终行: {}  提交: {}  构成: {}",
                            e.stats.churn.map(|c| c.to_string()).unwrap_or("-".into()),
                            e.stats.code_lines,
                            e.stats.commits.map(|c| c.to_string()).unwrap_or("-".into()),
                            e.stats.top_ext);
                        println!("  口径: {}（校准项目 {} 个，跳过部分覆盖 {} 个）",
                            if e.basis == "churn" { "翻动行" } else { "最终行（历史不完整，区间放大）" },
                            e.calibration_projects, e.calibration_skipped);
                        println!("  估算成本: ${:.0} — ${:.0} — ${:.0}  (P25–P50–P75)",
                            e.cost_p25, e.cost_p50, e.cost_p75);
                        println!("  估算 token(P50): {:.1}B", e.tokens_p50 / 1e9);
                    }
                }
                Err(e) => { eprintln!("无法估算: {e}"); std::process::exit(1); }
            }
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
