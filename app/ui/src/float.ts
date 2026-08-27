import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { api, applyTheme, fmtTok, fmtUsd, onUiPrefsChanged, onUsageUpdated, FloatData } from "./api";
import { applyStaticI18n, resolveLang, setLang, t, t2 } from "./i18n";

const win = getCurrentWindow();

async function applyPrefs(): Promise<void> {
  const s = await api.settings();
  applyTheme(s.theme);
  setLang(resolveLang(s.ui_lang));
  applyStaticI18n();
  document.body.style.opacity = String(s.float_opacity); // 整窗透明度（窗口本体 transparent）
}

function el(id: string): HTMLElement {
  return document.getElementById(id)!;
}

function fmtDur(hours: number): string {
  if (hours < 1) return `${Math.round(hours * 60)}m`;
  if (hours < 48) { const h = Math.floor(hours); return `${h}h${String(Math.round((hours - h) * 60)).padStart(2, "0")}`; }
  return `${(hours / 24).toFixed(1)}d`;
}

function hoursUntil(iso: string | null): number | null {
  if (!iso) return null;
  const ms = new Date(iso).getTime() - Date.now();
  return ms > 0 ? ms / 3.6e6 : null;
}

function shortModel(m: string): string {
  return m.replace(/^claude-/, "").replace(/-\d{8}$/, "");
}

function renderSparkInto(spark: HTMLElement, hourly: FloatData["hourly"]): void {
  spark.innerHTML = "";
  const max = Math.max(...hourly.map((h) => h.main_cost + h.side_cost), 1e-9);
  for (const h of hourly) {
    const col = document.createElement("div");
    col.className = "spark-col";
    const side = document.createElement("div");
    side.className = "spark-side";
    side.style.height = `${((h.side_cost / max) * 100).toFixed(1)}%`;
    const main = document.createElement("div");
    main.className = "spark-main";
    main.style.height = `${((h.main_cost / max) * 100).toFixed(1)}%`;
    // 柱顶倒角：给最上面那段圆角
    const top = h.side_cost > 0 ? side : main;
    if (h.main_cost + h.side_cost > 0) top.style.borderRadius = "2.5px 2.5px 0 0";
    col.append(side, main);
    col.title = `${h.hour}  ${t("e.main")} ${fmtUsd(h.main_cost)} / ${t("subagent")} ${fmtUsd(h.side_cost)}`;
    spark.appendChild(col);
  }
}

// 活跃任务切换：selectedId 记住用户选择；项目掉出活跃列表后回落到消耗最高者。
let selectedId: number | null = null;
let viewList: FloatData["active"] = [];

function renderSelection(d: FloatData): void {
  const idx = Math.max(0, viewList.findIndex((a) => a.project_id === selectedId));
  const cur = viewList[idx];
  const multi = viewList.length > 1;
  el("f-project").textContent = cur ? cur.project_name : d.project_name;
  el("f-project").title = cur && multi ? `${t("f.activeTip")} ${idx + 1}/${viewList.length} (${t("f.last30")} ${fmtUsd(cur.recent_cost)})` : "";
  el("f-count").textContent = multi ? `${idx + 1}/${viewList.length}` : "";
  el("f-model").textContent = shortModel(cur ? cur.last_model : d.model);
  el("f-proj").textContent = fmtUsd(cur ? cur.total_cost : d.project_cost);
  el("f-proj-label").textContent = multi ? t("f.taskTotal") : t("f.currentProject");
  (el("f-prev") as HTMLButtonElement).style.display = multi ? "" : "none";
  (el("f-next") as HTMLButtonElement).style.display = multi ? "" : "none";
}

function cycle(step: number): void {
  if (viewList.length < 2) return;
  const idx = Math.max(0, viewList.findIndex((a) => a.project_id === selectedId));
  const next = (idx + step + viewList.length) % viewList.length;
  selectedId = viewList[next].project_id;
  void refresh();
}

function limitLabel(w: { kind: string; scope: string | null }): string {
  if (w.kind === "session") return t("f.win5h");
  if (w.kind === "weekly_all") return t("f.winWeekAll");
  return `${t("f.winWeekPrefix")}${w.scope ?? w.kind}`;
}

async function refreshLimits(): Promise<void> {
  const box = el("f-limits");
  try {
    const u = await api.usageLimits();
    box.innerHTML = "";
    for (const w of u.windows) {
      const pct = Math.max(0, Math.min(100, w.utilization));
      const cls = pct >= 95 ? " crit" : pct >= 80 ? " hot" : "";
      const reset = hoursUntil(w.resets_at);
      // 电量式简洁文本：14% · ↻3h27 · est 2h10（est 仅在会先于重置耗尽时显示）
      let txt = `${Math.round(pct)}%`;
      if (reset != null) txt += ` · ↻${fmtDur(reset)}`;
      if (w.eta_h != null && (reset == null || w.eta_h < reset)) txt += ` · ${t("f.est")} ${fmtDur(w.eta_h)}`;
      const row = document.createElement("div");
      row.className = "limit-row";
      row.innerHTML = `<span class="limit-label">${limitLabel(w)}</span>
        <div class="limit-track"><div class="limit-fill${cls}" style="width:${pct}%"></div></div>
        <span class="limit-txt dim">${txt}</span>`;
      box.appendChild(row);
    }
  } catch {
    box.innerHTML = `<div class="f-eta dim">${t("f.limitErr")}</div>`;
  }
}

async function refresh(): Promise<void> {
  const d = await api.floatData();
  viewList = d.active;
  if (selectedId != null && !viewList.some((a) => a.project_id === selectedId)) {
    selectedId = null; // 选中的任务已不活跃，回落
  }
  const badge = el("f-badge");
  badge.textContent = d.billing_mode === "subscription" ? t("badge.sub") : d.billing_mode === "api" ? t("badge.api") : "?";
  badge.className = `badge badge-${d.billing_mode}`;
  badge.title =
    d.billing_mode === "subscription" ? t("f.tipSub") : d.billing_mode === "api" ? t("f.tipApi") : "";
  el("f-today").textContent = fmtUsd(d.today_cost);
  el("f-burn").textContent = fmtUsd(d.burn_rate);
  renderSelection(d);
  lastData = d;
  renderAxis(d.hourly);
  void updateTaskSpark(d);   // 折叠区主图：当前任务（切换 ‹ › 跟随）
  renderSparkInto(el("f-spark2"), d.hourly); // 展开区：全部项目
  renderTokens(d);
  void refreshLimits();
}

// 时间轴：5 个真实时刻刻度（每 6 小时一个），取代文字说明
function renderAxis(hourly: FloatData["hourly"]): void {
  const ax = el("f-axis");
  ax.innerHTML = "";
  for (const i of [0, 6, 12, 18, 23]) {
    const span = document.createElement("span");
    span.textContent = hourly[i] ? hourly[i].hour.slice(11, 16) : "";
    ax.appendChild(span);
  }
}

// 折叠区 24h 图显示当前选中任务；无任务时回退全部项目
async function updateTaskSpark(d: FloatData): Promise<void> {
  const task = currentTask();
  if (task) {
    renderSparkInto(el("f-spark"), await api.projectHourly(task.id));
  } else {
    renderSparkInto(el("f-spark"), d.hourly);
  }
}

/* ---- 展开：当前任务的 24 小时明细 ---- */
let expanded = false;
let lastData: FloatData | null = null;

// 展开态下的 token 口径行：今日 / 该任务 / 每小时燃烧（与上方美元数字同源同窗口）
function renderTokens(d: FloatData): void {
  const row = el("f-tokens");
  const idx = Math.max(0, viewList.findIndex((a) => a.project_id === selectedId));
  const cur = viewList[idx];
  const taskTokens = cur ? cur.total_tokens : d.project_tokens;
  row.textContent =
    `${t("f.today")} ${fmtTok(d.today_tokens)} tok · ` +
    `${t("f.taskTotal")} ${fmtTok(taskTokens)} tok · ` +
    `${fmtTok(d.burn_tokens)} tok/h`;
}

function currentTask(): { id: number; name: string } | null {
  const idx = Math.max(0, viewList.findIndex((a) => a.project_id === selectedId));
  const cur = viewList[idx];
  if (cur) return { id: cur.project_id, name: cur.project_name };
  if (lastData?.project_id != null) return { id: lastData.project_id, name: lastData.project_name };
  return null;
}


async function setExpanded(on: boolean): Promise<void> {
  expanded = on;
  el("f-expanded").style.display = on ? "flex" : "none";
  el("f-expand").textContent = on ? "⌃" : "⌄";
  await win.setSize(new LogicalSize(304, on ? 356 : 236));
}

// Tauri 的 data-tauri-drag-region 只匹配 mousedown 的精确目标元素，
// 悬浮窗表面全被子元素覆盖，属性永远不命中 —— 因此改为程序化拖拽。
// 所有按钮动作在 mousedown 处理：窗口未激活时的首次点击只投递 mousedown，
// DOM click 不触发——用 mousedown 保证"第一次点击就生效"。
document.body.addEventListener("mousedown", (e) => {
  if (e.button !== 0) return;
  const btn = (e.target as HTMLElement).closest("button");
  if (btn) {
    if (btn.id === "f-hide") void win.hide();
    else if (btn.id === "f-expand") void setExpanded(!expanded);
    else if (btn.id === "f-prev") cycle(-1);
    else if (btn.id === "f-next") cycle(1);
    return;
  }
  if (e.detail >= 2) {
    void api.openDashboard(); // 双击的第二次 mousedown：开面板
  } else {
    void win.startDragging();
  }
});
onUsageUpdated(() => void refresh());
onUiPrefsChanged(() => void applyPrefs());
void applyPrefs();
void refresh();
setInterval(() => void refresh(), 60_000); // 兜底：burn rate 随时间衰减也要更新
