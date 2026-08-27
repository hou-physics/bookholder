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

function renderBar(barId: string, txtId: string, w: { utilization: number; resets_at: string | null } | null): void {
  const bar = el(barId);
  const txt = el(txtId);
  if (!w) { bar.style.width = "0%"; txt.textContent = "—"; return; }
  const u = Math.max(0, Math.min(100, w.utilization));
  bar.style.width = `${u}%`;
  bar.className = "limit-fill" + (u >= 95 ? " crit" : u >= 80 ? " hot" : "");
  const reset = hoursUntil(w.resets_at);
  txt.textContent = `${Math.round(u)}%${reset != null ? ` · ${t("f.reset")} ${fmtDur(reset)}` : ""}`;
}

async function refreshLimits(): Promise<void> {
  try {
    const u = await api.usageLimits();
    renderBar("bar-5h", "txt-5h", u.five_hour);
    renderBar("bar-7d", "txt-7d", u.seven_day);
    // "续航"：斜率外推的耗尽时间，与重置时间取较小者才有意义（先耗尽才显示）
    const capEta = (eta: number | null, w: { resets_at: string | null } | null): string => {
      if (eta == null) return "—";
      const reset = w ? hoursUntil(w.resets_at) : null;
      return reset != null && reset < eta ? "—" : fmtDur(eta);
    };
    el("f-eta").textContent = t2("f.etaLine", {
      a: capEta(u.five_hour_eta_h, u.five_hour),
      b: capEta(u.seven_day_eta_h, u.seven_day),
    });
  } catch {
    el("f-eta").textContent = t("f.limitErr");
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
  el("f-mini").textContent =
    `${t("f.today")} ${fmtUsd(d.today_cost)} · ${fmtTok(d.today_tokens)} tok · ${fmtUsd(d.burn_rate)}/h`;
  renderSelection(d);
  renderSparkInto(el("f-spark"), d.hourly);
  lastData = d;
  renderTokens(d);
  if (expanded) void updateDetail();
  void refreshLimits();
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

async function updateDetail(): Promise<void> {
  const task = currentTask();
  if (!task) return;
  el("f-detail-name").textContent = task.name;
  renderSparkInto(el("f-spark2"), await api.projectHourly(task.id));
}

async function setExpanded(on: boolean): Promise<void> {
  expanded = on;
  el("f-expanded").style.display = on ? "flex" : "none";
  el("f-detail").style.display = on ? "flex" : "none";
  el("f-expand").textContent = on ? "⌃" : "⌄";
  await win.setSize(new LogicalSize(304, on ? 396 : 154));
  if (on) await updateDetail();
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
