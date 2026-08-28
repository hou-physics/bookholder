use crate::metrics;
use rusqlite::Connection;
use serde::Serialize;
use std::path::Path;

/// 跨项目成本估算：用本机已监测项目回归出 $/翻动行、$/行、$/提交 的分布，
/// 对任意仓库输出 P25–P50–P75 成本区间（区间而非单点——单点必然错）。

#[derive(Debug, Clone, Serialize)]
pub struct Calibration {
    /// (项目名, $/翻动行, $/最终行, $/提交, 该项目总成本)
    pub samples: Vec<(String, f64, f64, f64, f64)>,
    pub skipped_partial: usize, // 因追踪窗口不覆盖 git 史而跳过的项目
    pub tokens_per_dollar: f64, // 全局 token/$ 混合比（把 $ 区间换算成 token 区间）
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoStats {
    pub churn: Option<i64>,
    pub code_lines: i64,
    pub files: i64,
    pub commits: Option<i64>,
    pub top_ext: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Estimate {
    pub stats: RepoStats,
    pub basis: String, // "churn" | "loc"（churn 不可用时退化）
    pub cost_p25: f64,
    pub cost_p50: f64,
    pub cost_p75: f64,
    pub tokens_p50: f64,
    pub calibration_projects: usize,
    pub calibration_skipped: usize,
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = (q * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// 校准：遍历本机监测过的项目，取"追踪窗口覆盖完整 git 史"的项目
/// （首个提交不早于首次事件前 7 天，否则成本只覆盖部分开发、系数偏低）。
pub fn calibration(conn: &Connection) -> Calibration {
    let projects: Vec<(i64, String, String, String)> = conn
        .prepare(
            "SELECT id, display_name, cwd, COALESCE(first_seen,'') FROM projects WHERE cwd != ''",
        )
        .and_then(|mut st| {
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .map(|it| it.flatten().collect())
        })
        .unwrap_or_default();

    let mut samples = vec![];
    let mut skipped = 0;
    for (pid, name, cwd, first_seen) in projects {
        let root = Path::new(&cwd);
        if !root.is_dir() {
            continue;
        }
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd),0) FROM usage_events WHERE project_id = ?1",
                [pid],
                |r| r.get(0),
            )
            .unwrap_or(0.0);
        if cost < 5.0 {
            continue; // 样本太小
        }
        let Some(churn) = metrics::git_churn(root) else { continue };
        if churn < 50 {
            continue;
        }
        // 覆盖检查：git 首提交须不早于追踪起点 7 天
        if let (Some(first_commit), Ok(seen)) = (
            git_first_commit(root),
            chrono::NaiveDateTime::parse_from_str(&first_seen, "%Y-%m-%d %H:%M:%S"),
        ) {
            if first_commit < seen - chrono::Duration::days(7) {
                skipped += 1;
                continue;
            }
        }
        let snap = metrics::snapshot_dir(root);
        let commits = metrics::git_churn(root).map(|_| ());
        let _ = commits;
        let commit_n: f64 = std::process::Command::new("git")
            .args(["-C", &cwd, "rev-list", "--count", "HEAD"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
            .unwrap_or(1.0);
        let per_loc = if snap.code_lines > 0 { cost / snap.code_lines as f64 } else { 0.0 };
        samples.push((name, cost / churn as f64, per_loc, cost / commit_n.max(1.0), cost));
    }

    let (tok, dol): (f64, f64) = conn
        .query_row(
            "SELECT COALESCE(SUM(input_tokens+output_tokens+cache_read+cache_write_5m+cache_write_1h),0),
                    COALESCE(SUM(cost_usd),1)
             FROM usage_events",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((0.0, 1.0));

    Calibration { samples, skipped_partial: skipped, tokens_per_dollar: tok / dol.max(1e-9) }
}

fn git_first_commit(root: &Path) -> Option<chrono::NaiveDateTime> {
    let out = std::process::Command::new("git")
        .args(["-C", &root.to_string_lossy(), "log", "--reverse", "--format=%cI"])
        .output()
        .ok()?;
    let first = String::from_utf8_lossy(&out.stdout);
    let line = first.lines().next()?;
    chrono::DateTime::parse_from_rfc3339(line.trim())
        .ok()
        .map(|d| d.naive_utc())
}

pub fn repo_stats(path: &Path) -> RepoStats {
    let snap = metrics::snapshot_dir(path);
    let commits: Option<i64> = std::process::Command::new("git")
        .args(["-C", &path.to_string_lossy(), "rev-list", "--count", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8_lossy(&o.stdout).trim().parse().ok()
            } else {
                None
            }
        });
    RepoStats {
        churn: metrics::git_churn(path),
        code_lines: snap.code_lines,
        files: snap.files,
        commits,
        top_ext: snap.top_ext,
    }
}

pub fn estimate(conn: &Connection, path: &Path) -> Result<Estimate, String> {
    let cal = calibration(conn);
    if cal.samples.len() < 2 {
        return Err(format!(
            "calibration needs >=2 fully-tracked projects (have {}, skipped {} partial)",
            cal.samples.len(),
            cal.skipped_partial
        ));
    }
    let stats = repo_stats(path);
    // churn 可用且历史未被 squash（提交数太少说明历史不完整）→ churn 口径，否则 LOC 口径
    let squashed = stats.commits.unwrap_or(0) < 5;
    let (basis, unit_qty, mut coeffs): (String, f64, Vec<f64>) = match (stats.churn, squashed) {
        (Some(c), false) if c > 0 => (
            "churn".into(),
            c as f64,
            cal.samples.iter().map(|s| s.1).collect(),
        ),
        _ => (
            "loc".into(),
            stats.code_lines as f64,
            cal.samples.iter().map(|s| s.2).collect(),
        ),
    };
    coeffs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let (mut p25, p50, mut p75) = (
        quantile(&coeffs, 0.25) * unit_qty,
        quantile(&coeffs, 0.50) * unit_qty,
        quantile(&coeffs, 0.75) * unit_qty,
    );
    if basis == "loc" {
        // LOC 口径看不到重写量，区间放大
        p25 *= 0.7;
        p75 *= 1.8;
    }
    Ok(Estimate {
        stats,
        basis,
        cost_p25: p25,
        cost_p50: p50,
        cost_p75: p75,
        tokens_p50: p50 * cal.tokens_per_dollar,
        calibration_projects: cal.samples.len(),
        calibration_skipped: cal.skipped_partial,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantiles_basic() {
        let v = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(quantile(&v, 0.5), 3.0);
        assert_eq!(quantile(&v, 0.25), 2.0);
        assert_eq!(quantile(&v, 0.75), 4.0);
        assert_eq!(quantile(&[], 0.5), 0.0);
    }

    #[test]
    fn repo_stats_on_a_real_git_repo() {
        // 在临时目录造一个真实 git 仓库：两次提交产生可数的 churn
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t")
                .output().unwrap().status.success());
        };
        run(&["init", "-q"]);
        std::fs::write(dir.path().join("a.rs"), "line1\nline2\nline3\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "one"]);
        std::fs::write(dir.path().join("a.rs"), "line1\nCHANGED\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "two"]);

        let st = repo_stats(dir.path());
        assert_eq!(st.commits, Some(2));
        assert_eq!(st.code_lines, 2);
        // churn = 首提交 +3，第二次 +1/-2 → 6
        assert_eq!(st.churn, Some(6));
    }
}
