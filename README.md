# Bookholder

统计 Claude Code agentic 开发项目的真实 token 消耗与成本。

- **数据来源**：本地 `~/.claude/projects/**/*.jsonl` 转录（每条请求的真实 usage），
  解析后持久化到本地 SQLite，历史不随 Claude Code 的 30 天清理丢失。
- **计价**：LiteLLM 社区价格表每日自动更新、保留历史版本；订阅模式显示
  "等值 API 成本"，API 模式显示"实际计费成本"。
- **统计维度**：项目 / 会话 / 模型 / 主对话 vs subagent，逐请求可追溯。

## 安装

    make setup

装完出现悬浮窗（今日成本 / 本项目 / 燃烧率 + 24h sparkline）。
双击悬浮窗或托盘菜单打开详细面板。全部功能在面板上都是按钮。

## 可选 CLI

    cargo run -p bookholder -- report          # Markdown 报告
    cargo run -p bookholder -- report --json   # JSON（供脚本）
    cargo run -p bookholder -- backfill        # 全量重扫
    cargo run -p bookholder -- live            # 实时事件流

## 已知边界

- 未知模型的事件不计价（面板会提示条数），价格表更新后自动补算。
- 订阅模式的"成本"是等值 API 价，不是你的订阅账单。
