# Bookholder — Agentic 项目成本统计工具 设计文档

日期：2026-08-27
状态：已与用户逐节确认

## 1. 目标与范围

Bookholder 是一个 macOS 桌面工具，用于统计 agentic 开发项目（第一版仅支持
Claude Code）的真实 token 消耗与成本：

- 从本地转录文件解析每一次 API 请求的真实 token 用量，按项目、会话、模型、
  主对话 / subagent 精确分账。
- 实时监控：悬浮小窗显示当前消耗与 burn rate，详细面板提供完整图表与报告。
- 费用有来源、有时效：价格表每日自动更新并保留历史版本；订阅与 API 付费
  两种口径自动识别、明确标注、绝不混算。

### 设计原则

1. **GUI 优先**：所有功能都有直观的界面按钮，用户无需记忆任何命令。
   CLI 仅作为给脚本 / 自动化的附加层，与 GUI 功能等价但永远是可选的。
2. **轻量**：Tauri 应用，安装包约 10MB 量级，采集用 fsevents 增量读取，
   常驻开销接近零。
3. **数字为主、图表为证**：每个图表旁有精确数值，不做装饰性图表。
4. **数据可追溯**：任何汇总数字都能下钻到逐请求的事实记录。

### 第一版明确不做

- 其他 agent（Codex / Gemini CLI 等）的适配——架构预留适配器接口。
- 校准估算模型（用历史项目推算未完成 / 第三方项目）——放在第二阶段，
  但支撑它的项目特征数据从第一版就开始自动积累。
- 跨设备同步、历史价格的追溯修正 UI。

## 2. 数据来源（已在本机验证）

Claude Code 将转录存于 `~/.claude/projects/<项目目录 slug>/*.jsonl`。
`type == "assistant"` 的记录包含：

- `message.model` — 该请求实际使用的模型（subagent 不默认与主对话相同，
  每条记录各自带模型名，天然支持分模型计价）。
- `message.usage` — `input_tokens`、`output_tokens`（含
  `output_tokens_details.thinking_tokens` 细分）、
  `cache_read_input_tokens`、`cache_creation_input_tokens`（含 5 分钟 / 1
  小时两档 ephemeral 细分，两档价格不同，需分开计价）。
- `isSidechain` — 标记 subagent（sidechain）轮次。
- `cwd`、`sessionId`、`timestamp`、`requestId`。

**去重**：流式写入会为同一请求产生多条部分记录，按
`message.id + requestId` 去重，保留 token 数最大的最终记录。
不去重会重复计数，这是正确性的关键点。

**持久化必要性**：Claude Code 转录默认约 30 天清理，因此解析结果必须落库,
SQLite 是唯一持久层，入库后历史永久保留。

## 3. 架构

单体 Tauri 应用（方案 A），核心逻辑放在独立 Rust 库模块中：

```
bookholder/
├── core/          # Rust 库：解析、去重、计价、聚合（纯逻辑，无 UI 依赖）
│   ├── ingest    # 全量扫描 + fsevents 增量读取（记录每文件偏移量）
│   ├── pricing   # 价格表拉取、历史版本管理、逐事件计价
│   └── store     # SQLite 读写与聚合查询
├── app/           # Tauri 应用：悬浮窗 + 详细面板（前端 ECharts，本地打包）
└── cli/           # 同一二进制的 CLI 子命令（可选附加层）
```

- 数据库位置：`~/Library/Application Support/bookholder/bookholder.db`。
- 实时链路：core 写入新事件 → Tauri 事件推送前端 → 悬浮窗秒级刷新。

## 4. 数据模型（SQLite，3 张业务表 + 1 张价格表）

### usage_events（事实表）

每条 API 请求一行：时间、项目 id、会话 id、模型、是否 sidechain、
input / output / thinking / cache_write_5m / cache_write_1h / cache_read
各类 token 数、计价快照（成本与所用价格版本）、去重键
（message.id + requestId，唯一索引）。所有统计从此表聚合。

### sessions（会话维度）

所属项目、起止时间、入口类型、计费模式快照（订阅 / API，从 `~/.claude`
配置的登录方式自动判断，记录判断当时的值）。

### projects（项目维度）

目录路径、显示名、首次 / 最近活动时间；预留第二阶段校准特征列
（有效代码行数、文件数、语言、提交数、会话轮次等），第一版即开始积累。

### model_prices（价格表）

模型名、input / output / cache 各档单价、生效时间、来源。每日自动从
LiteLLM 的 `model_prices_and_context_window.json` 拉取，**保留历史版本**：
价格变动时旧账单仍按当时价格计算。拉取失败用缓存价，UI 标注价格数据的
最后更新时间。

### 费用口径

事件成本 = 各类 token × 对应模型、对应时期单价（5 分钟与 1 小时 cache
写入分开计价）。订阅模式显示为「等值 API 成本」，API 模式显示为
「实际计费成本」，界面明确标注口径。

## 5. UI 设计

### 悬浮小窗（常驻置顶、无边框、约 260×150、可拖动、可收纳为菜单栏图标）

- 顶行：当前活跃项目名 + 计费模式徽标 + 当前模型简称。
- 三个大数字：今日成本、当前项目累计成本、burn rate（最近 30 分钟的
  每小时消耗速率）。
- 底部 sparkline：最近 24 小时每小时消耗柱状图，subagent 活动以第二种
  颜色叠加。
- 双击打开详细面板。

### 详细面板（约 1000×700，左侧导航四页）

1. **总览** — 今日 / 本周 / 本月 / 全部的成本与 token 卡片；按天成本
   趋势面积图（按模型分色堆叠）；模型占比环形图；主对话 vs subagent
   成本对比条。
2. **项目** — 项目排行表（成本、token、会话数、活跃天数、最近活动）；
   单项目页：趋势图、模型分布、会话明细列表（展开到主对话 / 各
   subagent 分账）。
3. **会话明细** — 事实表级浏览：逐请求列出时间、模型、各类 token、
   单条成本、是否 sidechain。所有数字可追溯到这里。
4. **设置** — 价格表状态（来源、最后更新时间、**手动刷新按钮**）、
   计费模式覆盖开关、开机自启、悬浮窗显示项配置、**重新扫描按钮**
   （backfill）、解析跳过率显示。

**GUI 优先落实**：导出报告（Markdown / CSV）、手动刷新价格、全量重扫等
全部操作在面板上都是一个按钮；CLI 只是这些按钮背后同一函数的第二个入口。

图表用 ECharts 本地打包（不走 CDN）。

## 6. CLI（可选附加层，与 GUI 等价）

- `bookholder report [--project X] [--from/--to] [--json|--csv|--md]`
- `bookholder live --json` — 实时事件流，供脚本订阅。
- `bookholder backfill` — 全量重扫（自愈用）。

无 GUI 环境（SSH）下 CLI 独立可用。用户不需要记住任何命令即可使用全部
功能。

## 7. 自动化安装

一条 `make setup`（或安装脚本）完成：构建、注册 launchd 开机自启、
初始化数据库、首次价格拉取。装完零手动配置。

## 8. 错误处理

- 坏行跳过并计数，设置页显示跳过率（异常升高 = 格式可能变了）。
- 未来 JSONL 格式变化：降级为记录未知格式样本 + 提示更新，绝不崩溃、
  绝不写入错误数字。
- 价格拉取失败：用缓存价并明确标注时效。

## 9. 测试策略

- 解析 / 计价 / 去重是纯函数，用真实脱敏 JSONL 样本做单元测试，覆盖：
  流式重复记录、sidechain、5m/1h cache 混合、坏行、未知模型名。
- 聚合查询用内存 SQLite 测试。
- UI 是最薄展示层，不承载业务逻辑。

## 10. 第二阶段展望（不在本版实现）

- 校准估算：用积累的项目特征 + 真实 token 数据做回归，推算未完成 /
  第三方项目的可能用量，输出带置信区间的量级估计（预期误差 ±50% 级别，
  定位为参考而非精确值）。
- 其他 agent 适配器（Codex CLI 优先）。
